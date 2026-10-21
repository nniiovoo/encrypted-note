//! Rust-only Quick Unlock coordinator. OS authentication and cryptography run off the core lock.

use super::*;
use crate::quick_unlock::{Refused, Request};
use vault_format::{QuickUnlockBinding, QuickUnlockKey};

const QUICK_OPERATION_MS: u64 = 2 * 60_000;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QuickUnlockStatus {
    pub available: bool,
    pub enabled: bool,
    pub can_unlock: bool,
    pub reason: Option<&'static str>,
}

struct Snapshot {
    epoch: u64,
    generation: u64,
    locked: bool,
    counter: Option<u64>,
    bytes: Vec<u8>,
    binding: QuickUnlockBinding,
    deadline: u64,
}

/// Kept entirely in Rust between native Master Password verification and protected enrollment.
pub struct QuickUnlockEnrollment {
    pub key: QuickUnlockKey,
    snapshot: Snapshot,
    request: Request,
    limits: Limits,
}

pub struct QuickUnlockEnrollmentCompletion {
    snapshot: Snapshot,
    request: Request,
    verified: Result<(), ApiError>,
}

pub struct QuickUnlockJob {
    snapshot: Snapshot,
    request: Request,
    limits: Limits,
}

pub struct QuickUnlockCompletion {
    snapshot: Snapshot,
    request: Request,
    open: Result<Box<Open>, ApiError>,
}

impl QuickUnlockEnrollment {
    /// Verify the key actually released by the protected item against the frozen committed file.
    pub fn verify(self, returned_key: QuickUnlockKey) -> QuickUnlockEnrollmentCompletion {
        let verified = if returned_key.binding() != self.key.binding() {
            Err(quick_required())
        } else {
            vault_format::open_with_quick_unlock(&self.snapshot.bytes, returned_key, &self.limits)
                .map_err(quick_format_error)
                .and_then(|(_, body)| Document::from_json(&body).map(|_| ()).map_err(notes_error))
        };
        QuickUnlockEnrollmentCompletion {
            snapshot: self.snapshot,
            request: self.request,
            verified,
        }
    }
}

impl QuickUnlockJob {
    pub fn open(self, returned_key: QuickUnlockKey) -> QuickUnlockCompletion {
        let open =
            vault_format::open_with_quick_unlock(&self.snapshot.bytes, returned_key, &self.limits)
                .map_err(quick_format_error)
                .and_then(|(vault, body)| {
                    Document::from_json(&body)
                        .map(|doc| Box::new(Open { vault, doc }))
                        .map_err(notes_error)
                });
        QuickUnlockCompletion {
            snapshot: self.snapshot,
            request: self.request,
            open,
        }
    }
}

impl AppCore {
    pub fn set_quick_unlock_availability(&mut self, available: bool, reason: Option<&'static str>) {
        self.quick_unlock_available = available;
        self.quick_unlock_reason = reason;
        if !available {
            self.invalidate_native();
        }
    }

    pub(super) fn load_quick_unlock_registration(&mut self) {
        let Some(saved) = self.settings.quick_unlock_binding else {
            // A crash can publish the blob before enabled policy commits. Reap only our exact
            // local file; an already-disabled registration needs no extra settings write.
            #[cfg(target_os = "macos")]
            {
                self.quick_cleanup_needed = true;
            }
            return;
        };
        let binding = self
            .dir
            .read_vault()
            .ok()
            .and_then(|bytes| vault_format::quick_unlock_binding(&bytes, &self.limits).ok());
        if binding.is_some_and(|binding| binding.to_bytes() == saved) {
            self.quick_unlock
                .load_registration(QuickUnlockBinding::from_bytes(saved));
        } else {
            self.settings.quick_unlock_binding = None;
            if self.write_settings().is_ok() {
                self.quick_cleanup_needed = true;
            } else {
                self.settings.quick_unlock_binding = Some(saved);
            }
        }
    }

    pub(super) fn quick_unlock_status(&self) -> QuickUnlockStatus {
        let available = self.quick_unlock_available && self.clock.continuous_ms().is_some();
        let enabled = self.settings.quick_unlock_binding.is_some()
            && self.quick_unlock.registration().is_some();
        let can_unlock = available
            && matches!(self.phase, Phase::Locked)
            && !self.session.sharing_guard_up()
            && self.quick_unlock.registration().is_some_and(|binding| {
                self.quick_unlock
                    .available_at(binding, self.clock.continuous_ms())
            });
        let reason = self.quick_unlock_reason.or_else(|| {
            if self.clock.continuous_ms().is_none() {
                Some("Touch ID needs a clock that keeps counting through sleep.")
            } else if enabled && !can_unlock && matches!(self.phase, Phase::Locked) {
                Some("Use your Master Password before Touch ID.")
            } else {
                None
            }
        });
        QuickUnlockStatus {
            available,
            enabled,
            can_unlock,
            reason,
        }
    }

    /// The command coordinator consumes this flag and deletes only the exact app-owned item,
    /// outside the core mutex. This flag never authorizes a read or enables registration.
    pub fn quick_unlock_pending(&self) -> bool {
        self.quick_pending.is_some()
    }

    pub fn take_quick_unlock_cleanup(&mut self) -> bool {
        std::mem::take(&mut self.quick_cleanup_needed)
    }

    pub fn quick_unlock_cleanup_failed(&mut self) {
        self.quick_unlock_reason = Some(
            "The old Touch ID registration could not be removed. Use your Master Password and retry cleanup.",
        );
    }

    pub fn disable_quick_unlock(&mut self) -> Result<(), ApiError> {
        self.invalidate_native();
        self.quick_unlock.disable();
        let previous = self.settings.quick_unlock_binding.take();
        if let Err(error) = self.write_settings() {
            self.settings.quick_unlock_binding = previous;
            return Err(error);
        }
        self.quick_cleanup_needed = true;
        Ok(())
    }

    pub(super) fn invalidate_quick_unlock_registration(&mut self) -> Result<(), ApiError> {
        let previous = self.settings.quick_unlock_binding.take();
        let existed = previous.is_some()
            || self.quick_unlock.registration().is_some()
            || self.quick_pending.is_some();
        self.quick_unlock.invalidate_registration();
        self.quick_pending = None;
        // A failed write restores the old binding as retry state while the gate stays off.
        if existed {
            if let Err(error) = self.write_settings() {
                self.settings.quick_unlock_binding = previous;
                return Err(error);
            }
            self.quick_cleanup_needed = true;
        }
        Ok(())
    }

    pub(super) fn quick_master_verified(
        &mut self,
        bytes: &[u8],
        authenticated_binding: QuickUnlockBinding,
    ) {
        if let Ok(binding) = vault_format::quick_unlock_binding(bytes, &self.limits)
            && binding == authenticated_binding
        {
            let _ = self
                .quick_unlock
                .master_verified(binding, self.clock.continuous_ms());
        }
    }

    pub(super) fn prepare_quick_unlock_enrollment(
        &mut self,
        key: QuickUnlockKey,
    ) -> Result<QuickUnlockEnrollment, ApiError> {
        self.open()?;
        if !self.quick_unlock_available || self.clock.continuous_ms().is_none() {
            return Err(quick_required());
        }
        let snapshot = self.quick_snapshot(false)?;
        if key.binding() != snapshot.binding {
            return Err(not_now());
        }
        let request = self
            .quick_unlock
            .begin_enrollment_after_master(snapshot.binding, self.clock.continuous_ms())
            .map_err(quick_error)?;
        self.quick_pending = Some(snapshot.deadline);
        Ok(QuickUnlockEnrollment {
            key,
            snapshot,
            request,
            limits: self.limits,
        })
    }

    pub fn finish_quick_unlock_enrollment(
        &mut self,
        completion: QuickUnlockEnrollmentCompletion,
    ) -> Result<(), ApiError> {
        self.tick();
        self.validate_quick_snapshot(&completion.snapshot)?;
        self.quick_pending = None;
        if let Err(error) = completion.verified {
            self.invalidate_quick_unlock_registration()?;
            self.quick_cleanup_needed = true;
            return Err(error);
        }
        self.quick_unlock
            .complete_enrollment(
                completion.request,
                completion.snapshot.binding,
                self.clock.continuous_ms(),
            )
            .map_err(quick_error)?;
        self.settings.quick_unlock_binding = Some(completion.snapshot.binding.to_bytes());
        if let Err(error) = self.write_settings() {
            self.settings.quick_unlock_binding = None;
            self.quick_unlock.invalidate_registration();
            self.quick_cleanup_needed = true;
            return Err(error);
        }
        self.quick_unlock_reason = None;
        Ok(())
    }

    pub fn prepare_quick_unlock(&mut self) -> Result<QuickUnlockJob, ApiError> {
        self.tick();
        self.guard_check()?;
        if !matches!(self.phase, Phase::Locked)
            || self.native_pending.is_some()
            || self.quick_pending.is_some()
        {
            return Err(not_now());
        }
        if !self.quick_unlock_available {
            return Err(quick_required());
        }
        let snapshot = self.quick_snapshot(true)?;
        let request = self
            .quick_unlock
            .begin_unlock(snapshot.binding, self.clock.continuous_ms())
            .map_err(quick_error)?;
        self.quick_pending = Some(snapshot.deadline);
        Ok(QuickUnlockJob {
            snapshot,
            request,
            limits: self.limits,
        })
    }

    pub fn finish_quick_unlock(
        &mut self,
        completion: QuickUnlockCompletion,
    ) -> Result<(), ApiError> {
        self.tick();
        self.validate_quick_snapshot(&completion.snapshot)?;
        self.quick_pending = None;
        let open = match completion.open {
            Ok(open) => open,
            Err(error) => {
                self.invalidate_quick_unlock_registration()?;
                return Err(error);
            }
        };
        self.quick_unlock
            .complete_unlock(
                completion.request,
                completion.snapshot.binding,
                self.clock.continuous_ms(),
            )
            .map_err(quick_error)?;
        self.session_generation = self.session_generation.saturating_add(1);
        // Biometry is not a Master Password check and must never erase password-attempt backoff.
        self.session.quick_unlocked(self.clock.now_ms());
        self.reset_guard();
        self.set_phase(Phase::Unlocked(open));
        self.purge_trash();
        Ok(())
    }

    fn quick_snapshot(&self, locked: bool) -> Result<Snapshot, ApiError> {
        let bytes = self.dir.read_vault().map_err(store_error)?;
        let binding = vault_format::quick_unlock_binding(&bytes, &self.limits)
            .map_err(|e| format_error(e, CredKind::Password))?;
        let counter = if locked {
            None
        } else {
            Some(self.open()?.doc.change_counter())
        };
        Ok(Snapshot {
            epoch: self.native_epoch,
            generation: self.session_generation,
            locked,
            counter,
            bytes,
            binding,
            deadline: self.clock.now_ms().saturating_add(QUICK_OPERATION_MS),
        })
    }

    fn validate_quick_snapshot(&mut self, snapshot: &Snapshot) -> Result<(), ApiError> {
        if snapshot.epoch != self.native_epoch || snapshot.generation != self.session_generation {
            return Err(not_now());
        }
        let phase_matches = if snapshot.locked {
            matches!(self.phase, Phase::Locked)
        } else {
            self.open()
                .is_ok_and(|open| Some(open.doc.change_counter()) == snapshot.counter)
        };
        if self.quick_pending.is_none()
            || snapshot.epoch != self.native_epoch
            || snapshot.generation != self.session_generation
            || !phase_matches
            || self.clock.now_ms() >= snapshot.deadline
            || self.session.sharing_guard_up()
            || self.dir.read_vault().map_err(store_error)? != snapshot.bytes
        {
            self.invalidate_native();
            return Err(not_now());
        }
        Ok(())
    }
}

fn quick_format_error(error: vault_format::Error) -> ApiError {
    if error == vault_format::Error::WrongCredential {
        quick_required()
    } else {
        format_error(error, CredKind::Password)
    }
}

fn quick_error(error: Refused) -> ApiError {
    if error == Refused::StaleRequest {
        not_now()
    } else {
        quick_required()
    }
}

fn quick_required() -> ApiError {
    err(
        "invalid_input",
        "Touch ID is unavailable. Use your Master Password.",
    )
}
