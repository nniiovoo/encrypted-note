//! Serializes local biometric custody outside the core mutex; no key is an IPC value.
use crate::commands::CoreState;
use crate::core::native::{NativeAction, NativeResult};
use crate::core::{ApiError, AppCore};
use crate::native_credential::{self, Authorization, Biometric, PromptError};
use crate::{biometry, commands, native_coordinator};
use std::path::PathBuf;
use std::sync::{
    Mutex, PoisonError, TryLockError,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use vault_format::QuickUnlockKey;
use zeroize::Zeroizing;

/// Native credential panels are numbered by core operations; Touch ID panels use the top bit,
/// so neither can close the other.
const TOUCH_ID_PANEL: u64 = 1 << 63;
const TOUCH_ID_TIMEOUT: Duration = Duration::from_secs(120);

pub struct QuickRuntime {
    path: PathBuf,
    busy: AtomicBool,
    biometric: AtomicBool,
    cleanup: AtomicBool,
    cancellation: Mutex<Option<biometry::Cancellation>>,
}

impl QuickRuntime {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            busy: AtomicBool::new(false),
            biometric: AtomicBool::new(false),
            cleanup: AtomicBool::new(false),
            cancellation: Mutex::new(None),
        }
    }
}

struct Lease {
    app: AppHandle,
    cancellation: biometry::Cancellation,
    enrollment_incomplete: bool,
    panel: Option<u64>,
}
impl Lease {
    fn begin(app: &AppHandle) -> Result<Self, ApiError> {
        let state = app.state::<QuickRuntime>();
        let mut slot = state
            .cancellation
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if state
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(unavailable());
        }
        let cancellation = biometry::Cancellation::default();
        *slot = Some(cancellation.clone());
        Ok(Self {
            app: app.clone(),
            cancellation,
            enrollment_incomplete: false,
            panel: None,
        })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(id) = self.panel.take() {
            let _ = self.app.run_on_main_thread(move || {
                native_credential::cancel(id);
            });
        }
        commands::with_core(&self.app, |core| {
            if core.quick_unlock_pending() {
                core.cancel_all_native();
            }
        });
        let state = self.app.state::<QuickRuntime>();
        if self.enrollment_incomplete {
            state.cleanup.store(true, Ordering::SeqCst);
        }
        state.biometric.store(false, Ordering::SeqCst);
        state
            .cancellation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        state.busy.store(false, Ordering::SeqCst);
        schedule_cleanup(&self.app);
    }
}

pub fn cancel(app: &AppHandle) {
    if let Some(state) = app.try_state::<QuickRuntime>()
        && let Some(cancellation) = state
            .cancellation
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
    {
        cancellation.cancel();
    }
}

pub fn sync(app: &AppHandle, core: &mut AppCore) {
    let Some(state) = app.try_state::<QuickRuntime>() else {
        return;
    };
    if core.take_quick_unlock_cleanup() {
        state.cleanup.store(true, Ordering::SeqCst);
    }
    if state.biometric.load(Ordering::SeqCst) && !core.quick_unlock_pending() {
        cancel(app);
    }
    schedule_cleanup(app);
}

fn schedule_cleanup(app: &AppHandle) {
    let Some(state) = app.try_state::<QuickRuntime>() else {
        return;
    };
    if !state.cleanup.load(Ordering::SeqCst)
        || state
            .busy
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
    {
        return;
    }
    state.cleanup.store(false, Ordering::SeqCst);
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let result = biometry::remove(&app.state::<QuickRuntime>().path);
        commands::with_core(&app, |core| {
            core.quick_unlock_cleanup_finished(result.is_ok());
        });
        app.state::<QuickRuntime>()
            .busy
            .store(false, Ordering::SeqCst);
        schedule_cleanup(&app);
    });
}

async fn core_work<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(&mut AppCore) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || commands::with_core(&app, work))
        .await
        .map_err(|_| unavailable())?
}

async fn failed(app: &AppHandle, error: biometry::Error) {
    let disable = !matches!(error, biometry::Error::Cancelled);
    let _ = core_work(app, move |core| {
        // A late result must not cancel a new native action after an owner Lock.
        if core.quick_unlock_pending() {
            core.cancel_all_native();
            if disable {
                core.disable_quick_unlock()?;
            }
        }
        Ok(())
    })
    .await;
}

fn panel_id(request: u64) -> u64 {
    TOUCH_ID_PANEL | request
}

/// The Touch ID panel stays only while its exact request is pending, unguarded and in time.
fn touch_id_authorization(
    pending: bool,
    guard_up: bool,
    cancelled: bool,
    expired: bool,
) -> Authorization {
    if pending && !guard_up && !cancelled && !expired {
        Authorization::Allowed
    } else {
        Authorization::Denied
    }
}

/// Runs on the UI thread every tick: never waits for the core, which may be busy committing.
fn touch_id_activity(
    app: &AppHandle,
    cancellation: biometry::Cancellation,
    deadline: Instant,
) -> native_credential::Activity {
    let app = app.clone();
    Box::new(move |_genuine| {
        let expired = Instant::now() >= deadline;
        let state = app.state::<CoreState>();
        let core = match state.0.try_lock() {
            Ok(core) => core,
            Err(TryLockError::WouldBlock) if !cancellation.is_cancelled() && !expired => {
                return Authorization::Retry;
            }
            Err(_) => return Authorization::Denied,
        };
        touch_id_authorization(
            core.quick_unlock_pending(),
            core.app_status().sharing_guard.up,
            cancellation.is_cancelled(),
            expired,
        )
    })
}

/// Show Touch ID inside the protected window before any protected read can start. Swift
/// refuses a read whose request has no embedded view, so the system alert (which takes focus
/// and was cancelled by it) never appears. The panel closes with the lease; closing it any
/// other way (Cancel, Escape, focus loss, Lock, the Sharing Guard, expiry) cancels this exact
/// request, which also invalidates its paired authentication context.
async fn present_touch_id(
    app: &AppHandle,
    lease: &mut Lease,
    enrollment: bool,
) -> Result<(), ApiError> {
    let request = lease.cancellation.request();
    let id = panel_id(request);
    let biometric = Biometric {
        title: if enrollment {
            "Enable Touch ID for this Vault"
        } else {
            "Unlock with Touch ID"
        }
        .to_owned(),
        detail: if enrollment {
            "Touch the sensor to finish enabling Touch ID. Your Master Password keeps working."
        } else {
            "Touch the sensor to unlock your Vault. You can still use your Master Password instead."
        }
        .to_owned(),
        request,
    };
    let activity = touch_id_activity(
        app,
        lease.cancellation.clone(),
        Instant::now() + TOUCH_ID_TIMEOUT,
    );
    let on_close = lease.cancellation.clone();
    let main_app = app.clone();
    let (tx, rx) = mpsc::channel();
    lease.panel = Some(id);
    app.run_on_main_thread(move || {
        let presented = match main_app.get_webview_window("main") {
            Some(window) => native_credential::present_biometric(
                &window,
                id,
                biometric,
                activity,
                Box::new(move || on_close.cancel()),
            ),
            None => Err(PromptError::Unavailable),
        };
        let _ = tx.send(presented);
    })
    .map_err(|_| unavailable())?;
    let presented =
        tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(10)))
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?;
    match presented {
        Ok(()) if !lease.cancellation.is_cancelled() => Ok(()),
        Ok(()) | Err(PromptError::Cancelled) => Err(cancelled()),
        Err(_) => Err(unavailable()),
    }
}

pub async fn enable(app: AppHandle) -> Result<(), ApiError> {
    let mut lease = Lease::begin(&app)?;
    let enrollment =
        match native_coordinator::run(app.clone(), NativeAction::EnrollQuickUnlock).await? {
            NativeResult::QuickUnlockEnrollment(enrollment) => enrollment,
            _ => return Err(unavailable()),
        };
    app.state::<QuickRuntime>()
        .biometric
        .store(true, Ordering::SeqCst);
    present_touch_id(&app, &mut lease, true).await?;
    lease.enrollment_incomplete = true;
    let cancel = lease.cancellation.clone();
    let worker_app = app.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        if cancel.is_cancelled() {
            return Err(biometry::Error::Cancelled);
        }
        let state = worker_app.state::<QuickRuntime>();
        if state.cleanup.swap(false, Ordering::SeqCst) {
            let result = biometry::remove(&state.path);
            commands::with_core(&worker_app, |core| {
                core.quick_unlock_cleanup_finished(result.is_ok());
            });
            result?;
        }
        let payload = enrollment
            .key
            .with_keychain_payload(|bytes| Zeroizing::new(bytes.to_vec()));
        let returned = biometry::enroll(&state.path, payload, &cancel)?;
        if cancel.is_cancelled() {
            return Err(biometry::Error::Cancelled);
        }
        let returned_key = QuickUnlockKey::from_keychain_payload(returned)
            .map_err(|_| biometry::Error::InvalidPayload)?;
        Ok(enrollment.verify(returned_key))
    })
    .await
    .map_err(|_| unavailable())?;
    let result = match outcome {
        Ok(completion) if !lease.cancellation.is_cancelled() => {
            let cancel = lease.cancellation.clone();
            core_work(&app, move |core| {
                if cancel.is_cancelled() {
                    return Err(cancelled());
                }
                core.finish_quick_unlock_enrollment(completion)
            })
            .await
        }
        Ok(_) => {
            failed(&app, biometry::Error::Cancelled).await;
            Err(cancelled())
        }
        Err(error) => {
            failed(&app, error).await;
            Err(map_error(error))
        }
    };
    if result.is_ok() {
        lease.enrollment_incomplete = false;
    }
    result
}

pub async fn unlock(app: AppHandle) -> Result<(), ApiError> {
    let mut lease = Lease::begin(&app)?;
    let job = core_work(&app, AppCore::prepare_quick_unlock).await?;
    app.state::<QuickRuntime>()
        .biometric
        .store(true, Ordering::SeqCst);
    present_touch_id(&app, &mut lease, false).await?;
    let cancel = lease.cancellation.clone();
    let path = app.state::<QuickRuntime>().path.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        let payload = biometry::read(&path, &cancel)?;
        if cancel.is_cancelled() {
            return Err(biometry::Error::Cancelled);
        }
        let key = QuickUnlockKey::from_keychain_payload(payload)
            .map_err(|_| biometry::Error::InvalidPayload)?;
        Ok(job.open(key))
    })
    .await
    .map_err(|_| unavailable())?;
    match outcome {
        Ok(completion) if !lease.cancellation.is_cancelled() => {
            let cancel = lease.cancellation.clone();
            core_work(&app, move |core| {
                if cancel.is_cancelled() {
                    return Err(cancelled());
                }
                core.finish_quick_unlock(completion)
            })
            .await
        }
        Ok(_) => {
            failed(&app, biometry::Error::Cancelled).await;
            Err(cancelled())
        }
        Err(error) => {
            failed(&app, error).await;
            Err(map_error(error))
        }
    }
}

pub async fn disable(app: AppHandle) -> Result<(), ApiError> {
    cancel(&app);
    core_work(&app, AppCore::disable_quick_unlock).await
}

fn map_error(error: biometry::Error) -> ApiError {
    if error == biometry::Error::Cancelled {
        cancelled()
    } else {
        unavailable()
    }
}
fn cancelled() -> ApiError {
    ApiError {
        code: "cancelled",
        message: "Touch ID was cancelled. Nothing was opened.".to_owned(),
    }
}
fn unavailable() -> ApiError {
    ApiError {
        code: "invalid_input",
        message: "Touch ID is unavailable. Use your Master Password.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{TOUCH_ID_PANEL, panel_id, touch_id_authorization};
    use crate::biometry::Cancellation;
    use crate::native_credential::Authorization;

    #[test]
    fn touch_id_panel_stays_only_for_its_pending_unguarded_live_request() {
        assert_eq!(
            touch_id_authorization(true, false, false, false),
            Authorization::Allowed
        );
        for (pending, guard_up, cancelled, expired) in [
            (false, false, false, false),
            (true, true, false, false),
            (true, false, true, false),
            (true, false, false, true),
            (false, true, true, true),
        ] {
            assert_eq!(
                touch_id_authorization(pending, guard_up, cancelled, expired),
                Authorization::Denied,
                "pending={pending} guard={guard_up} cancelled={cancelled} expired={expired}"
            );
        }
    }

    #[test]
    fn touch_id_panels_never_share_ids_with_native_operation_panels() {
        let first = Cancellation::default();
        let second = Cancellation::default();
        for operation in [1, 2, 1000, u64::MAX >> 1] {
            assert_ne!(panel_id(first.request()), operation);
        }
        assert_ne!(panel_id(first.request()), panel_id(second.request()));
        assert_eq!(panel_id(first.request()) & TOUCH_ID_PANEL, TOUCH_ID_PANEL);
    }
}
