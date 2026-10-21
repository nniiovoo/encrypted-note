//! Serializes local biometric custody outside the core mutex; no key is an IPC value.
use crate::core::native::{NativeAction, NativeResult};
use crate::core::{ApiError, AppCore};
use crate::{biometry, commands, native_coordinator};
use std::path::PathBuf;
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicBool, Ordering},
};
use tauri::{AppHandle, Manager};
use vault_format::QuickUnlockKey;
use zeroize::Zeroizing;

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
        })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.cancellation.cancel();
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
        if result.is_err() {
            commands::with_core(&app, |core| core.quick_unlock_cleanup_failed());
        }
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

pub async fn enable(app: AppHandle) -> Result<(), ApiError> {
    let mut lease = Lease::begin(&app)?;
    let enrollment =
        match native_coordinator::run(app.clone(), NativeAction::EnrollQuickUnlock).await? {
            NativeResult::QuickUnlockEnrollment(enrollment) => enrollment,
            _ => return Err(unavailable()),
        };
    lease.enrollment_incomplete = true;
    app.state::<QuickRuntime>()
        .biometric
        .store(true, Ordering::SeqCst);
    let cancel = lease.cancellation.clone();
    let worker_app = app.clone();
    let outcome = tauri::async_runtime::spawn_blocking(move || {
        if cancel.is_cancelled() {
            return Err(biometry::Error::Cancelled);
        }
        let state = worker_app.state::<QuickRuntime>();
        if state.cleanup.swap(false, Ordering::SeqCst) {
            biometry::remove(&state.path)?;
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
    let lease = Lease::begin(&app)?;
    let job = core_work(&app, AppCore::prepare_quick_unlock).await?;
    app.state::<QuickRuntime>()
        .biometric
        .store(true, Ordering::SeqCst);
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
