//! Process-local Quick Unlock authorization. No keys, persistence or OS calls live here.

use vault_format::QuickUnlockBinding;

pub const MASTER_REQUIRED_AFTER_MS: u64 = 7 * 24 * 60 * 60_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    Unavailable,
    MasterPasswordRequired,
    StaleRequest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Enroll,
    Unlock,
}

/// A one-shot action captured before native authentication. No secret or IPC representation.
#[derive(Debug)]
pub struct Request {
    generation: u64,
    binding: QuickUnlockBinding,
    action: Action,
}

struct Verification {
    binding: QuickUnlockBinding,
    deadline: u64,
}

#[derive(Default)]
pub struct QuickUnlockGate {
    registration: Option<QuickUnlockBinding>,
    verification: Option<Verification>,
    last_clock: Option<u64>,
    generation: u64,
    pending: Option<(u64, Action)>,
}

impl QuickUnlockGate {
    /// A persisted enabled flag only remembers enrollment. Process startup still requires
    /// native Master Password verification, including when the protected item survives.
    pub fn load_registration(&mut self, binding: QuickUnlockBinding) {
        self.cancel_pending();
        self.registration = Some(binding);
        self.verification = None;
    }

    pub fn registration(&self) -> Option<QuickUnlockBinding> {
        self.registration
    }

    /// Call only after a fresh native Master Password check of the current committed file.
    /// Recovery credentials and biometric reads must never call this method.
    pub fn master_verified(
        &mut self,
        binding: QuickUnlockBinding,
        continuous_ms: Option<u64>,
    ) -> Result<(), Refused> {
        self.cancel_pending();
        let now = self.clock(continuous_ms)?;
        let Some(deadline) = now.checked_add(MASTER_REQUIRED_AFTER_MS) else {
            self.verification = None;
            return Err(Refused::MasterPasswordRequired);
        };
        if self
            .registration
            .is_some_and(|registered| registered != binding)
        {
            self.registration = None;
        }
        self.verification = Some(Verification { binding, deadline });
        Ok(())
    }

    /// Starts enrollment after its fresh native Master Password check. Registration is only
    /// committed by complete_enrollment after a protected read and authenticated current file.
    pub fn begin_enrollment_after_master(
        &mut self,
        binding: QuickUnlockBinding,
        continuous_ms: Option<u64>,
    ) -> Result<Request, Refused> {
        self.registration = None;
        self.master_verified(binding, continuous_ms)?;
        self.begin(binding, Action::Enroll)
    }

    pub fn complete_enrollment(
        &mut self,
        request: Request,
        current_binding: QuickUnlockBinding,
        continuous_ms: Option<u64>,
    ) -> Result<(), Refused> {
        self.complete(request, current_binding, Action::Enroll, continuous_ms)?;
        self.registration = Some(current_binding);
        Ok(())
    }

    /// Observe every core tick, including while Locked, so expiry or a bad clock cannot be
    /// undone by a later clock reading before the next biometric request.
    pub fn tick(&mut self, continuous_ms: Option<u64>) -> bool {
        let Ok(now) = self.clock(continuous_ms) else {
            return false;
        };
        if self
            .verification
            .as_ref()
            .is_some_and(|verified| now < verified.deadline)
        {
            return true;
        }
        self.verification = None;
        self.cancel_pending();
        false
    }

    /// A read-only UI hint. Actual requests always recheck and revoke on clock faults.
    pub fn available_at(&self, binding: QuickUnlockBinding, continuous_ms: Option<u64>) -> bool {
        let Some(now) = continuous_ms else {
            return false;
        };
        self.last_clock.is_none_or(|last| now >= last)
            && self.registration == Some(binding)
            && self
                .verification
                .as_ref()
                .is_some_and(|verified| verified.binding == binding && now < verified.deadline)
    }

    pub fn can_unlock(
        &mut self,
        current_binding: QuickUnlockBinding,
        continuous_ms: Option<u64>,
    ) -> bool {
        self.authorized(current_binding, continuous_ms).is_ok()
            && self.registration == Some(current_binding)
    }

    pub fn begin_unlock(
        &mut self,
        current_binding: QuickUnlockBinding,
        continuous_ms: Option<u64>,
    ) -> Result<Request, Refused> {
        self.authorized(current_binding, continuous_ms)?;
        if self.registration != Some(current_binding) {
            return Err(Refused::Unavailable);
        }
        self.begin(current_binding, Action::Unlock)
    }

    /// Call only after the actual protected key has opened the current file. A late native
    /// callback must pass this check immediately before installing the ordinary open session.
    pub fn complete_unlock(
        &mut self,
        request: Request,
        current_binding: QuickUnlockBinding,
        continuous_ms: Option<u64>,
    ) -> Result<(), Refused> {
        self.complete(request, current_binding, Action::Unlock, continuous_ms)?;
        if self.registration != Some(current_binding) {
            return Err(Refused::Unavailable);
        }
        Ok(())
    }

    /// Ordinary lock/sleep/switch: wipe the open session in AppCore and cancel its native
    /// context. The enrolled item and process deadline remain usable for a later fresh read.
    pub fn cancel_pending(&mut self) {
        self.pending = None;
        if let Some(next) = self.generation.checked_add(1) {
            self.generation = next;
        } else {
            self.registration = None;
            self.verification = None;
        }
    }

    /// Mark unavailable before exact-item deletion. Failed deletion cannot re-enable the
    /// surviving item; another explicit enrollment is required.
    pub fn disable(&mut self) {
        self.cancel_pending();
        self.registration = None;
    }

    /// Before credential changes, recovery, replacement or rotation, including failed or
    /// cancelled rotation: no pending callback or old registration may reopen the old file.
    pub fn invalidate_registration(&mut self) {
        self.disable();
        self.verification = None;
    }

    fn begin(&mut self, binding: QuickUnlockBinding, action: Action) -> Result<Request, Refused> {
        self.cancel_pending();
        if self.generation == u64::MAX {
            self.invalidate_registration();
            return Err(Refused::Unavailable);
        }
        self.pending = Some((self.generation, action));
        Ok(Request {
            generation: self.generation,
            binding,
            action,
        })
    }

    fn complete(
        &mut self,
        request: Request,
        current_binding: QuickUnlockBinding,
        action: Action,
        continuous_ms: Option<u64>,
    ) -> Result<(), Refused> {
        if request.action != action
            || request.binding != current_binding
            || self.pending != Some((request.generation, action))
        {
            return Err(Refused::StaleRequest);
        }
        self.pending = None;
        self.authorized(current_binding, continuous_ms)
    }

    fn authorized(
        &mut self,
        binding: QuickUnlockBinding,
        continuous_ms: Option<u64>,
    ) -> Result<(), Refused> {
        let now = self.clock(continuous_ms)?;
        if self
            .verification
            .as_ref()
            .is_some_and(|verified| verified.binding == binding && now < verified.deadline)
        {
            return Ok(());
        }
        self.verification = None;
        self.cancel_pending();
        Err(Refused::MasterPasswordRequired)
    }

    fn clock(&mut self, continuous_ms: Option<u64>) -> Result<u64, Refused> {
        let Some(now) = continuous_ms else {
            self.verification = None;
            self.cancel_pending();
            return Err(Refused::MasterPasswordRequired);
        };
        if self.last_clock.is_some_and(|previous| now < previous) {
            self.verification = None;
            self.cancel_pending();
            return Err(Refused::MasterPasswordRequired);
        }
        self.last_clock = Some(now);
        Ok(now)
    }
}

#[cfg(test)]
mod tests {
    use super::{MASTER_REQUIRED_AFTER_MS, QuickUnlockGate, Refused};
    use secrecy::SecretString;
    use vault_format::{KdfParams, Limits, QuickUnlockBinding};

    fn binding() -> QuickUnlockBinding {
        let created = vault_format::create(
            &SecretString::from("obviously-fake-quick-unlock-test-password"),
            KdfParams::TESTING,
            &Limits::TESTING,
            b"fake document",
        )
        .unwrap();
        vault_format::quick_unlock_binding(&created.bytes, &Limits::TESTING).unwrap()
    }

    fn enrolled(binding: QuickUnlockBinding) -> QuickUnlockGate {
        let mut gate = QuickUnlockGate::default();
        let request = gate
            .begin_enrollment_after_master(binding, Some(0))
            .unwrap();
        gate.complete_enrollment(request, binding, Some(1)).unwrap();
        gate
    }

    #[test]
    fn startup_needs_master_even_with_persisted_enrollment() {
        let binding = binding();
        let mut gate = QuickUnlockGate::default();
        assert!(!gate.can_unlock(binding, Some(0)));
        gate.load_registration(binding);
        assert_eq!(
            gate.begin_unlock(binding, Some(1)).unwrap_err(),
            Refused::MasterPasswordRequired
        );
        gate.master_verified(binding, Some(2)).unwrap();
        assert!(gate.can_unlock(binding, Some(3)));
    }

    #[test]
    fn biometric_reads_never_extend_the_seven_day_deadline() {
        let binding = binding();
        let mut gate = enrolled(binding);
        let near_deadline = MASTER_REQUIRED_AFTER_MS - 1;
        let request = gate.begin_unlock(binding, Some(near_deadline)).unwrap();
        gate.complete_unlock(request, binding, Some(near_deadline))
            .unwrap();
        assert!(!gate.can_unlock(binding, Some(MASTER_REQUIRED_AFTER_MS)));
        gate.master_verified(binding, Some(MASTER_REQUIRED_AFTER_MS + 1))
            .unwrap();
        assert!(gate.can_unlock(binding, Some(MASTER_REQUIRED_AFTER_MS + 2)));
    }

    #[test]
    fn lock_cancel_and_newer_requests_discard_late_callbacks() {
        let binding = binding();
        let mut gate = enrolled(binding);
        let stale = gate.begin_unlock(binding, Some(2)).unwrap();
        gate.cancel_pending();
        assert_eq!(
            gate.complete_unlock(stale, binding, Some(3)),
            Err(Refused::StaleRequest)
        );
        assert!(gate.can_unlock(binding, Some(4)));
        let stale = gate.begin_unlock(binding, Some(5)).unwrap();
        let current = gate.begin_unlock(binding, Some(6)).unwrap();
        assert_eq!(
            gate.complete_unlock(stale, binding, Some(7)),
            Err(Refused::StaleRequest)
        );
        gate.complete_unlock(current, binding, Some(7)).unwrap();
    }

    #[test]
    fn incomplete_or_cancelled_enrollment_never_registers() {
        let binding = binding();
        let mut gate = QuickUnlockGate::default();
        let stale = gate
            .begin_enrollment_after_master(binding, Some(0))
            .unwrap();
        assert_eq!(gate.registration(), None);
        gate.cancel_pending();
        assert_eq!(
            gate.complete_enrollment(stale, binding, Some(1)),
            Err(Refused::StaleRequest)
        );
        assert!(!gate.can_unlock(binding, Some(2)));
    }

    #[test]
    fn credential_changes_and_disable_deny_surviving_items() {
        let binding = binding();
        let mut gate = enrolled(binding);
        let stale = gate.begin_unlock(binding, Some(2)).unwrap();
        gate.invalidate_registration();
        assert_eq!(
            gate.complete_unlock(stale, binding, Some(3)),
            Err(Refused::StaleRequest)
        );
        assert!(!gate.can_unlock(binding, Some(4)));
        gate.master_verified(binding, Some(5)).unwrap();
        assert!(!gate.can_unlock(binding, Some(6)));
        let mut gate = enrolled(binding);
        gate.disable();
        assert!(!gate.can_unlock(binding, Some(2)));
    }

    #[test]
    fn a_different_binding_cannot_use_an_enrollment_or_verification() {
        let binding = binding();
        let other = self::binding();
        let mut gate = enrolled(binding);
        let request = gate.begin_unlock(binding, Some(2)).unwrap();
        assert_eq!(
            gate.complete_unlock(request, other, Some(3)),
            Err(Refused::StaleRequest)
        );
        gate.master_verified(other, Some(4)).unwrap();
        assert_eq!(gate.registration(), None);
        assert!(!gate.can_unlock(other, Some(5)));
    }

    #[test]
    fn unavailable_backwards_or_overflowing_clocks_fail_closed() {
        let binding = binding();
        for now in [None, Some(0), Some(u64::MAX)] {
            let mut gate = enrolled(binding);
            assert!(!gate.can_unlock(binding, now));
            assert!(!gate.can_unlock(binding, Some(2)));
        }
        let mut gate = QuickUnlockGate::default();
        assert_eq!(
            gate.master_verified(binding, Some(u64::MAX)),
            Err(Refused::MasterPasswordRequired)
        );
    }

    #[test]
    fn observed_expiry_or_missing_clock_never_revives_after_a_later_reading() {
        let binding = binding();
        let mut gate = enrolled(binding);
        assert!(!gate.tick(Some(MASTER_REQUIRED_AFTER_MS)));
        assert!(!gate.can_unlock(binding, Some(MASTER_REQUIRED_AFTER_MS - 1)));
        let mut gate = enrolled(binding);
        assert!(!gate.tick(None));
        assert!(!gate.can_unlock(binding, Some(2)));
    }

    #[test]
    fn expiry_during_authentication_and_generation_overflow_fail_closed() {
        let binding = binding();
        let mut gate = enrolled(binding);
        let request = gate
            .begin_unlock(binding, Some(MASTER_REQUIRED_AFTER_MS - 1))
            .unwrap();
        assert_eq!(
            gate.complete_unlock(request, binding, Some(MASTER_REQUIRED_AFTER_MS)),
            Err(Refused::MasterPasswordRequired)
        );
        let mut gate = enrolled(binding);
        gate.generation = u64::MAX - 1;
        assert_eq!(
            gate.begin_unlock(binding, Some(2)).unwrap_err(),
            Refused::Unavailable
        );
        assert!(!gate.can_unlock(binding, Some(3)));
    }
}
