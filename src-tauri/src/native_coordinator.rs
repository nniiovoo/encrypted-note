//! One native operation at a time. Credentials, Wallet readouts and Recovery Kits stay in Rust.

use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError, TryLockError, mpsc};
use std::time::{Duration, Instant};

use notes::Kind;
use tauri::{AppHandle, Emitter, Manager};
use zeroize::Zeroizing;

use crate::commands::{self, CoreState, STATUS_EVENT};
use crate::core::native::{NativeAction, NativeCredentials, NativeOperation, NativeResult};
use crate::core::{ApiError, AppCore, AppStatus, ShowMode};
use crate::native_credential::{
    self, Authorization, Prompt, PromptError, Reply, Reveal, RevealMode, ShowCheck, TimeLeft,
    TimeLimit,
};

const OPERATION_TIMEOUT: Duration = Duration::from_secs(120);
/// A new-password panel also ends after this long with no real key or click.
const NO_INPUT_MS: u64 = session::DEFAULT_IDLE_MINUTES as u64 * 60_000;
const WARN_MS: u64 = 60_000;
/// At a one-minute Auto-lock a 60-second warning would never go away.
const WARN_SHORT_IDLE_MS: u64 = 30_000;

#[derive(Default)]
pub struct CoordinatorState {
    sequence: AtomicU64,
    current: AtomicU64,
    active: Mutex<Option<Active>>,
    /// The step whose end the owner hears about: zero for plain Notes, the Kit and readouts.
    reporting: AtomicU64,
    /// The step that an OS event, the Sharing Guard or a limit stopped, not the owner.
    interrupted: AtomicU64,
    /// The step under the no-input rule and when it runs out. Atomics, so the UI thread never waits.
    input_token: AtomicU64,
    input_by: AtomicU64,
    input_origin: OnceLock<Instant>,
}

#[derive(Clone)]
struct Active {
    token: u64,
    id: u64,
    deadline: Instant,
    stage: Stage,
}

#[derive(Clone)]
enum Stage {
    Calibration {
        epoch: u64,
        generation: u64,
        phase: &'static str,
    },
    Entry,
    RecoveryKit {
        epoch: u64,
    },
    Reveal(RevealIdentity),
}

#[derive(Clone)]
struct RevealIdentity {
    epoch: u64,
    generation: u64,
    note_id: String,
    field: String,
}

impl RevealIdentity {
    fn matches(&self, status: &AppStatus, epoch: u64) -> bool {
        self.epoch == epoch && self.same_field(status)
    }

    fn same_field(&self, status: &AppStatus) -> bool {
        status.phase == "unlocked"
            && !status.sharing_guard.up
            && status.session_generation == self.generation
            && status
                .reveal
                .as_ref()
                .is_some_and(|shown| shown.note_id == self.note_id && shown.field == self.field)
    }
}

impl CoordinatorState {
    fn reserve(&self, deadline: Instant) -> Result<u64, ApiError> {
        let token = self
            .sequence
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_add(1))
            .map_err(|_| unavailable())?
            + 1;
        self.current
            .compare_exchange(0, token, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| busy_error())?;
        if !self.publish(token, 0, deadline, Stage::Entry) {
            return Err(cancelled());
        }
        Ok(token)
    }

    fn live(&self, token: u64) -> bool {
        self.current.load(Ordering::SeqCst) == token
    }

    fn publish(&self, token: u64, id: u64, deadline: Instant, stage: Stage) -> bool {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.live(token) {
            return false;
        }
        *active = Some(Active {
            token,
            id,
            deadline,
            stage,
        });
        true
    }

    fn snapshot(&self, token: u64) -> Option<Active> {
        self.active
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .filter(|active| active.token == token && self.live(token))
            .cloned()
    }

    fn try_snapshot(&self, token: u64) -> Snapshot {
        match self.active.try_lock() {
            Ok(active) => active
                .as_ref()
                .filter(|active| active.token == token && self.live(token))
                .cloned()
                .map_or(Snapshot::Denied, Snapshot::Active),
            Err(TryLockError::WouldBlock) => Snapshot::Retry,
            Err(TryLockError::Poisoned(_)) => Snapshot::Denied,
        }
    }

    fn stop(&self, token: u64) -> Option<Active> {
        // Keep the old panel ID until the next lease can publish its metadata.
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        self.current
            .compare_exchange(token, 0, Ordering::SeqCst, Ordering::SeqCst)
            .ok()?;
        if active.as_ref().is_some_and(|active| active.token == token) {
            active.take()
        } else {
            None
        }
    }

    fn idle(&self) -> bool {
        self.current.load(Ordering::SeqCst) == 0
    }

    fn stop_reporting(&self, token: u64) {
        let _ = self
            .reporting
            .compare_exchange(token, 0, Ordering::SeqCst, Ordering::SeqCst);
    }

    fn input_clock_ms(&self, at: Instant) -> u64 {
        let origin = *self.input_origin.get_or_init(|| at);
        u64::try_from(at.saturating_duration_since(origin).as_millis()).unwrap_or(u64::MAX)
    }

    fn expect_input(&self, token: u64, now: Instant) {
        let by = self.input_clock_ms(now).saturating_add(NO_INPUT_MS);
        self.input_by.store(by, Ordering::SeqCst);
        self.input_token.store(token, Ordering::SeqCst);
    }

    fn owner_input(&self, token: u64, now: Instant) {
        if self.input_token.load(Ordering::SeqCst) == token {
            let by = self.input_clock_ms(now).saturating_add(NO_INPUT_MS);
            self.input_by.store(by, Ordering::SeqCst);
        }
    }

    fn end_input_rule(&self, token: u64) {
        let _ = self
            .input_token
            .compare_exchange(token, 0, Ordering::SeqCst, Ordering::SeqCst);
    }

    fn input_left(&self, token: u64, now: Instant) -> Option<Duration> {
        (self.input_token.load(Ordering::SeqCst) == token).then(|| {
            let by = self.input_by.load(Ordering::SeqCst);
            Duration::from_millis(by.saturating_sub(self.input_clock_ms(now)))
        })
    }

    fn input_expired(&self, token: u64, now: Instant) -> bool {
        self.input_left(token, now)
            .is_some_and(|left| left.is_zero())
    }
}

enum Snapshot {
    Active(Active),
    Retry,
    Denied,
}

struct Lease {
    app: AppHandle,
    token: u64,
    deadline: Instant,
}

impl Drop for Lease {
    fn drop(&mut self) {
        cancel_token(&self.app, self.token);
    }
}

#[derive(Clone, Copy)]
enum EntryKind {
    Password,
    Recovery,
    NewPassword,
    PasswordChange,
}

fn reserve(app: &AppHandle) -> Result<Lease, ApiError> {
    let deadline = Instant::now() + OPERATION_TIMEOUT;
    let token = app.state::<CoordinatorState>().reserve(deadline)?;
    Ok(Lease {
        app: app.clone(),
        token,
        deadline,
    })
}

/// Reserve before calibration so a cancelled setup cannot reopen its password panel later.
pub async fn create(app: AppHandle) -> Result<NativeResult, ApiError> {
    let lease = reserve(&app)?;
    let token = lease.token;
    let deadline = lease.deadline;
    app.state::<CoordinatorState>()
        .reporting
        .store(token, Ordering::SeqCst);
    let prepare_app = app.clone();
    core_work(&app, move |core| {
        check_live(&prepare_app, token)?;
        core.create_vault_allowed()?;
        let status = core.app_status();
        let state = prepare_app.state::<CoordinatorState>();
        if !state.publish(
            token,
            0,
            deadline,
            Stage::Calibration {
                epoch: core.native_operation_epoch(),
                generation: status.session_generation,
                phase: status.phase,
            },
        ) {
            return Err(ended(&state, token));
        }
        Ok(())
    })
    .await?;
    let params = tauri::async_runtime::spawn_blocking(|| {
        vault_format::calibrate(&vault_format::Limits::PRODUCTION, vault_format::measure_kdf)
    })
    .await
    .map_err(|_| unavailable())?;
    check_live(&app, token)?;
    run_leased(app, NativeAction::CreateVault { params }, lease).await
}

/// No submitted credential or native readout becomes a command result.
pub async fn run(app: AppHandle, action: NativeAction) -> Result<NativeResult, ApiError> {
    if matches!(action, NativeAction::ConfirmRecoveryKit) {
        return Err(cancelled());
    }
    let lease = reserve(&app)?;
    run_leased(app, action, lease).await
}

async fn run_leased(
    app: AppHandle,
    action: NativeAction,
    lease: Lease,
) -> Result<NativeResult, ApiError> {
    let token = lease.token;
    let entry_kind = match &action {
        NativeAction::CreateVault { .. } | NativeAction::SetNewPassword => EntryKind::NewPassword,
        NativeAction::ChangePassword { .. } => EntryKind::PasswordChange,
        NativeAction::Unlock { recovery: true }
        | NativeAction::InspectBackup { recovery: true }
        | NativeAction::InspectSafetyCopy { recovery: true, .. } => EntryKind::Recovery,
        _ => EntryKind::Password,
    };
    let reveal_mode = match &action {
        NativeAction::ShowWalletField {
            mode: ShowMode::Held,
            ..
        } => RevealMode::Held,
        _ => RevealMode::Timed,
    };
    let begin_app = app.clone();
    let (operation, entry_deadline) = core_work(&app, move |core| {
        check_live(&begin_app, token)?;
        let state = begin_app.state::<CoordinatorState>();
        let active = state.snapshot(token).ok_or_else(|| ended(&state, token))?;
        if !active_matches(&active, core) {
            return Err(ended(&state, token));
        }
        let operation = core.begin_native(action)?;
        let now = Instant::now();
        let entry_deadline = now + Duration::from_millis(operation.limit_ms);
        if !state.publish(token, operation.id, entry_deadline, Stage::Entry) {
            core.cancel_native(operation.id);
            return Err(ended(&state, token));
        }
        if matches!(
            entry_kind,
            EntryKind::NewPassword | EntryKind::PasswordChange
        ) {
            state.expect_input(token, now);
        }
        Ok((operation, entry_deadline))
    })
    .await?;
    let credentials = if operation.requires_credentials {
        app.state::<CoordinatorState>()
            .reporting
            .store(token, Ordering::SeqCst);
        let prompt = entry_prompt(&operation, entry_kind);
        let reply = ask_until(&lease, operation.id, prompt, entry_deadline).await?;
        let entered = credentials(entry_kind, reply)?;
        allow_commit_time(&app.state::<CoordinatorState>(), token, &operation)?;
        entered
    } else {
        NativeCredentials::None
    };
    let mut operation = operation;
    let mut credentials = credentials;
    loop {
        let prepare_app = app.clone();
        let id = operation.id;
        let job = core_work(&app, move |core| {
            check_live(&prepare_app, token)?;
            core.prepare_native(id, credentials)
        })
        .await?;
        let completion = tauri::async_runtime::spawn_blocking(move || job.run())
            .await
            .map_err(|_| unavailable())?;
        let finish_app = app.clone();
        let outcome = core_work(&app, move |core| {
            check_live(&finish_app, token)?;
            let result = core.finish_native(completion)?;
            let state = finish_app.state::<CoordinatorState>();
            match result {
                NativeResult::Field(value) => {
                    let status = core.app_status();
                    let shown = status.reveal.ok_or_else(cancelled)?;
                    let identity = RevealIdentity {
                        epoch: core.native_operation_epoch(),
                        generation: status.session_generation,
                        note_id: shown.note_id,
                        field: shown.field,
                    };
                    // Its own time, so typing the password doesn't shorten what it unlocked.
                    let readout_deadline = Instant::now() + OPERATION_TIMEOUT;
                    if !state.publish(token, id, readout_deadline, Stage::Reveal(identity)) {
                        core.hide_field();
                        return Err(ended(&state, token));
                    }
                    Ok(Outcome::Reveal {
                        value,
                        deadline: readout_deadline,
                    })
                }
                NativeResult::RecoveryKit(mut kit) => {
                    if let Some(recovery) = kit.recovery_key.take() {
                        let next = match core.begin_native(NativeAction::ConfirmRecoveryKit) {
                            Ok(next) => next,
                            Err(error) => {
                                core.lock();
                                return Err(error);
                            }
                        };
                        let kit_deadline = Instant::now() + Duration::from_millis(next.limit_ms);
                        if !state.publish(
                            token,
                            next.id,
                            kit_deadline,
                            Stage::RecoveryKit {
                                epoch: core.native_operation_epoch(),
                            },
                        ) {
                            core.lock();
                            return Err(ended(&state, token));
                        }
                        Ok(Outcome::RecoveryKit {
                            recovery,
                            operation: next,
                            deadline: kit_deadline,
                        })
                    } else {
                        state.stop(token);
                        Ok(Outcome::Return(NativeResult::Unit))
                    }
                }
                result => {
                    state.stop(token);
                    Ok(Outcome::Return(result))
                }
            }
        })
        .await?;
        match outcome {
            Outcome::Return(result) => return Ok(result),
            Outcome::Reveal { value, deadline } => {
                app.state::<CoordinatorState>().stop_reporting(token);
                reveal(&lease, &operation, value, reveal_mode, deadline).await?;
                return Ok(NativeResult::Unit);
            }
            Outcome::RecoveryKit {
                recovery,
                operation: next,
                deadline: kit_deadline,
            } => {
                app.state::<CoordinatorState>().stop_reporting(token);
                let groups = last_groups(&recovery)?;
                let rotation = operation.action == "rotate_keys";
                let prompt = Prompt::RecoveryKit {
                    title: "Write down your Recovery Key".to_owned(),
                    detail: kit_detail(rotation).to_owned(),
                    recovery,
                    rotation,
                };
                let reply = ask_until(&lease, next.id, prompt, kit_deadline).await?;
                if !matches!(reply, Reply::RecoveryKitConfirmed) {
                    return Err(cancelled());
                }
                credentials = NativeCredentials::KitConfirmation(groups);
                operation = next;
            }
        }
    }
}

enum Outcome {
    Return(NativeResult),
    Reveal {
        value: Zeroizing<String>,
        deadline: Instant,
    },
    RecoveryKit {
        recovery: Zeroizing<String>,
        operation: NativeOperation,
        deadline: Instant,
    },
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

fn check_live(app: &AppHandle, token: u64) -> Result<(), ApiError> {
    check_state(&app.state::<CoordinatorState>(), token)
}

fn check_state(state: &CoordinatorState, token: u64) -> Result<(), ApiError> {
    if state
        .snapshot(token)
        .is_some_and(|active| Instant::now() < active.deadline)
    {
        Ok(())
    } else {
        Err(ended(state, token))
    }
}

/// How a stopped step is reported. A stop the owner asked for, or a step that isn't reporting,
/// stays silent.
fn ended(state: &CoordinatorState, token: u64) -> ApiError {
    let reporting = state.reporting.load(Ordering::SeqCst) == token;
    if reporting && (state.interrupted.load(Ordering::SeqCst) == token || state.live(token)) {
        interrupted()
    } else {
        cancelled()
    }
}

fn prompt_failure(state: &CoordinatorState, token: u64, error: PromptError) -> ApiError {
    match error {
        // The Cancel button or Escape: nothing else has stopped this step.
        PromptError::Cancelled if state.live(token) => cancelled(),
        PromptError::Cancelled | PromptError::Interrupted => ended(state, token),
        PromptError::Busy => busy_error(),
        PromptError::Unavailable => unavailable(),
    }
}

/// After a valid submit on a long entry, key derivation and the commit get their own time, and
/// the owner has nothing left to type. Once, here: publishing later could overwrite the Kit stage.
fn allow_commit_time(
    state: &CoordinatorState,
    token: u64,
    operation: &NativeOperation,
) -> Result<(), ApiError> {
    if u128::from(operation.limit_ms) <= OPERATION_TIMEOUT.as_millis() {
        return Ok(());
    }
    state.end_input_rule(token);
    let active = state.snapshot(token).ok_or_else(|| ended(state, token))?;
    let deadline = active.deadline.max(Instant::now() + OPERATION_TIMEOUT);
    if state.publish(token, operation.id, deadline, Stage::Entry) {
        Ok(())
    } else {
        Err(ended(state, token))
    }
}

async fn ask_until(
    lease: &Lease,
    id: u64,
    prompt: Prompt,
    deadline: Instant,
) -> Result<Reply, ApiError> {
    let (tx, rx) = mpsc::channel();
    let app = lease.app.clone();
    let token = lease.token;
    lease
        .app
        .run_on_main_thread(move || {
            if !can_present(&app, token, id, deadline) {
                let _ = tx.send(Err(PromptError::Interrupted));
                return;
            }
            let Some(window) = app.get_webview_window("main") else {
                let _ = tx.send(Err(PromptError::Unavailable));
                return;
            };
            let failed = tx.clone();
            if let Err(error) = native_credential::present(
                &window,
                id,
                prompt,
                activity_hook(&app, token, deadline),
                time_probe(&app, token),
                may_show_hook(&app),
                Box::new(move |reply| {
                    let _ = tx.send(reply);
                }),
            ) {
                let _ = failed.send(Err(error));
            }
        })
        .map_err(|_| unavailable())?;
    let reply = {
        let answer = receive(rx, deadline).await?;
        let state = lease.app.state::<CoordinatorState>();
        answer
            .ok_or_else(|| ended(&state, token))?
            .map_err(|error| prompt_failure(&state, token, error))?
    };
    check_live(&lease.app, lease.token)?;
    Ok(reply)
}

async fn reveal(
    lease: &Lease,
    operation: &NativeOperation,
    value: Zeroizing<String>,
    mode: RevealMode,
    deadline: Instant,
) -> Result<(), ApiError> {
    let (tx, rx) = mpsc::channel();
    let app = lease.app.clone();
    let token = lease.token;
    let id = operation.id;
    let title = format!("Your {}", field_name(operation.field.as_deref()));
    let detail = reveal_detail(operation);
    let value = if operation.kind == Some(Kind::SeedPhrase)
        && operation.field.as_deref() == Some("words")
    {
        seed_grid(&value)
    } else {
        value
    };
    lease
        .app
        .run_on_main_thread(move || {
            if !can_present(&app, token, id, deadline) {
                let _ = tx.send(Err(PromptError::Interrupted));
                return;
            }
            let Some(window) = app.get_webview_window("main") else {
                let _ = tx.send(Err(PromptError::Unavailable));
                return;
            };
            let extend_app = app.clone();
            let failed = tx.clone();
            if let Err(error) = native_credential::present_reveal(
                &window,
                id,
                Reveal {
                    title,
                    detail,
                    value,
                    mode,
                    closes_at: deadline,
                },
                Box::new(move || extend(&extend_app, token, deadline)),
                activity_hook(&app, token, deadline),
                // A timed readout counts down in its own line.
                match mode {
                    RevealMode::Held => time_probe(&app, token),
                    RevealMode::Timed => Box::new(|| TimeLeft::Quiet),
                },
                Box::new(move || {
                    let _ = tx.send(Ok(()));
                }),
            ) {
                let _ = failed.send(Err(error));
            }
        })
        .map_err(|_| unavailable())?;
    match receive(rx, deadline).await? {
        Some(Ok(())) => {}
        Some(Err(PromptError::Busy)) => return Err(busy_error()),
        Some(Err(PromptError::Unavailable)) => return Err(unavailable()),
        Some(Err(PromptError::Cancelled | PromptError::Interrupted)) | None => {
            return Err(cancelled());
        }
    }
    let app = lease.app.clone();
    core_work(&lease.app, move |core| {
        check_live(&app, token)?;
        core.hide_field();
        app.state::<CoordinatorState>().stop(token);
        Ok(())
    })
    .await
}

/// `None` when the deadline came first.
async fn receive<T: Send + 'static>(
    rx: mpsc::Receiver<T>,
    deadline: Instant,
) -> Result<Option<T>, ApiError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    tauri::async_runtime::spawn_blocking(move || rx.recv_timeout(remaining).ok())
        .await
        .map_err(|_| unavailable())
}

/// The UI thread must never wait for the core or a KDF worker.
fn can_present(app: &AppHandle, token: u64, id: u64, deadline: Instant) -> bool {
    authorize_callback(
        &app.state::<CoreState>().0,
        &app.state::<CoordinatorState>(),
        token,
        deadline,
        Some(id),
        false,
    ) != Authorization::Denied
}

fn retry_if_live(state: &CoordinatorState, token: u64, deadline: Instant) -> Authorization {
    if state.live(token) && Instant::now() < deadline {
        Authorization::Retry
    } else {
        Authorization::Denied
    }
}

fn authorize_callback(
    core: &Mutex<AppCore>,
    state: &CoordinatorState,
    token: u64,
    deadline: Instant,
    id: Option<u64>,
    genuine_owner_activity: bool,
) -> Authorization {
    let mut core = match core.try_lock() {
        Ok(core) => core,
        Err(TryLockError::WouldBlock) => return retry_if_live(state, token, deadline),
        Err(TryLockError::Poisoned(_)) => return Authorization::Denied,
    };
    authorize_core_callback(
        &mut core,
        state,
        token,
        deadline,
        id,
        genuine_owner_activity,
    )
}

/// Polling and key repeat may check a readout, but only fresh owner input resets Auto-lock.
/// Tick first: cached status can still describe a reveal after its idle/reveal deadline.
fn authorize_core_callback(
    core: &mut AppCore,
    state: &CoordinatorState,
    token: u64,
    deadline: Instant,
    id: Option<u64>,
    genuine_owner_activity: bool,
) -> Authorization {
    core.tick();
    let active = match state.try_snapshot(token) {
        Snapshot::Active(active) => active,
        Snapshot::Retry => return retry_if_live(state, token, deadline),
        Snapshot::Denied => return Authorization::Denied,
    };
    if !active_authorized(&active, core, state, deadline, id) {
        return Authorization::Denied;
    }
    if genuine_owner_activity {
        core.activity();
        state.owner_input(active.token, Instant::now());
    }
    if active_authorized(&active, core, state, deadline, id) {
        Authorization::Allowed
    } else {
        Authorization::Denied
    }
}

fn active_authorized(
    active: &Active,
    core: &AppCore,
    state: &CoordinatorState,
    deadline: Instant,
    id: Option<u64>,
) -> bool {
    let now = Instant::now();
    state.live(active.token)
        && id.is_none_or(|id| id == active.id)
        && now < deadline
        && now < active.deadline
        && !state.input_expired(active.token, now)
        && active_matches(active, core)
}

fn active_matches(active: &Active, core: &AppCore) -> bool {
    let status = core.app_status();
    if status.sharing_guard.up {
        return false;
    }
    match &active.stage {
        Stage::Calibration {
            epoch,
            generation,
            phase,
        } => {
            *epoch == core.native_operation_epoch()
                && *generation == status.session_generation
                && *phase == status.phase
        }
        Stage::Entry | Stage::RecoveryKit { .. } => {
            active.id == 0 || core.native_pending_id() == Some(active.id)
        }
        Stage::Reveal(identity) => identity.matches(&status, core.native_operation_epoch()),
    }
}

fn activity_hook(app: &AppHandle, token: u64, deadline: Instant) -> native_credential::Activity {
    let app = app.clone();
    Box::new(move |genuine_owner_activity| {
        authorize_callback(
            &app.state::<CoreState>().0,
            &app.state::<CoordinatorState>(),
            token,
            deadline,
            None,
            genuine_owner_activity,
        )
    })
}

fn time_probe(app: &AppHandle, token: u64) -> native_credential::TimeProbe {
    let app = app.clone();
    Box::new(move || {
        time_left(
            &app.state::<CoreState>().0,
            &app.state::<CoordinatorState>(),
            token,
            Instant::now(),
        )
    })
}

/// For the time row. Read only: it never ticks, so asking can't lock the app or end a readout,
/// and it never counts as activity.
fn time_left(
    core: &Mutex<AppCore>,
    state: &CoordinatorState,
    token: u64,
    now: Instant,
) -> TimeLeft {
    let Snapshot::Active(active) = state.try_snapshot(token) else {
        return TimeLeft::Busy;
    };
    let Ok(core) = core.try_lock() else {
        return TimeLeft::Busy;
    };
    let status = core.app_status();
    let idle_minutes = status.settings.idle_minutes;
    let mut panel = active.deadline.saturating_duration_since(now);
    if let Some(input) = state.input_left(token, now) {
        panel = panel.min(input);
    }
    let panel_ms = u64::try_from(panel.as_millis()).unwrap_or(u64::MAX);
    let (left_ms, limit, warn_ms) = match status.lock_in_ms {
        Some(lock_ms) if lock_ms < panel_ms => (
            lock_ms,
            TimeLimit::AutoLock,
            if idle_minutes == 1 {
                WARN_SHORT_IDLE_MS
            } else {
                WARN_MS
            },
        ),
        _ => (panel_ms, TimeLimit::Panel, WARN_MS),
    };
    if left_ms <= warn_ms {
        TimeLeft::Left {
            seconds: left_ms.div_ceil(1_000),
            limit,
        }
    } else if matches!(active.stage, Stage::RecoveryKit { .. }) {
        TimeLeft::KitCalm { idle_minutes }
    } else {
        TimeLeft::Quiet
    }
}

fn may_show_hook(app: &AppHandle) -> native_credential::MayShow {
    let app = app.clone();
    Box::new(move || may_show(&app.state::<CoreState>().0))
}

/// Never waits. Only a clear answer is Allowed; a poisoned core is a no, and closes the panel too.
fn may_show(core: &Mutex<AppCore>) -> ShowCheck {
    match core.try_lock() {
        Ok(core) if core.sharing_app_detected() => ShowCheck::Refused,
        Ok(_) => ShowCheck::Allowed,
        Err(TryLockError::WouldBlock) => ShowCheck::Busy,
        Err(TryLockError::Poisoned(_)) => ShowCheck::Refused,
    }
}

fn extend(app: &AppHandle, token: u64, deadline: Instant) -> Authorization {
    extend_callback(
        &app.state::<CoreState>().0,
        &app.state::<CoordinatorState>(),
        token,
        deadline,
        |core| {
            let _ = app.emit(STATUS_EVENT, commands::status(core));
        },
    )
}

fn extend_callback(
    core: &Mutex<AppCore>,
    state: &CoordinatorState,
    token: u64,
    deadline: Instant,
    on_extended: impl FnOnce(&AppCore),
) -> Authorization {
    let mut core = match core.try_lock() {
        Ok(core) => core,
        Err(TryLockError::WouldBlock) => return retry_if_live(state, token, deadline),
        Err(TryLockError::Poisoned(_)) => return Authorization::Denied,
    };
    core.tick();
    if !core
        .app_status()
        .reveal
        .is_some_and(|shown| shown.can_extend)
    {
        return Authorization::Denied;
    }
    let active = match state.try_snapshot(token) {
        Snapshot::Active(active) => active,
        Snapshot::Retry => return retry_if_live(state, token, deadline),
        Snapshot::Denied => return Authorization::Denied,
    };
    if !matches!(&active.stage, Stage::Reveal(_))
        || !active_authorized(&active, &core, state, deadline, None)
        || core.extend_reveal().is_err()
        || !active_authorized(&active, &core, state, deadline, None)
    {
        return Authorization::Denied;
    }
    on_extended(&core);
    Authorization::Allowed
}

/// Called after tick/guard updates while the caller already holds CoreState.
pub fn sync(app: &AppHandle, core: &mut AppCore) {
    let state = app.state::<CoordinatorState>();
    let token = state.current.load(Ordering::SeqCst);
    let Some(active) = state.snapshot(token) else {
        return;
    };
    let now = Instant::now();
    if now < active.deadline && !state.input_expired(token, now) && active_matches(&active, core) {
        return;
    }
    state.interrupted.store(token, Ordering::SeqCst);
    if let Some(active) = state.stop(token) {
        cleanup_core(core, &active, state.idle());
        close_panel(app, active.id);
    }
}

/// Invalidate callbacks immediately; core cleanup and UI dismissal never wait on the main thread.
pub fn cancel(app: &AppHandle) {
    let token = app
        .state::<CoordinatorState>()
        .current
        .load(Ordering::SeqCst);
    if token != 0 {
        cancel_token(app, token);
    }
}

/// Like `cancel`, for what the owner didn't ask for: sleep, screen lock, another app taking focus.
/// The waiting step then says it was interrupted.
pub fn interrupt(app: &AppHandle) {
    let state = app.state::<CoordinatorState>();
    let token = state.current.load(Ordering::SeqCst);
    if token != 0 {
        state.interrupted.store(token, Ordering::SeqCst);
        cancel_token(app, token);
    }
}

pub fn busy(app: &AppHandle) -> bool {
    !app.state::<CoordinatorState>().idle()
}

fn cancel_token(app: &AppHandle, token: u64) {
    let Some(active) = app.state::<CoordinatorState>().stop(token) else {
        return;
    };
    close_panel(app, active.id);
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        commands::with_core(&app, |core| {
            cleanup_core(core, &active, app.state::<CoordinatorState>().idle());
        });
    });
}

fn cleanup_core(core: &mut AppCore, active: &Active, coordinator_idle: bool) {
    if let Stage::RecoveryKit { epoch } = active.stage
        && (core.native_pending_id() == Some(active.id) || core.native_operation_epoch() == epoch)
        && core.app_status().phase == "confirm_recovery_kit"
    {
        // A later reservation does not own this still-unsaved candidate.
        core.lock();
        return;
    }
    if active.id != 0 {
        core.cancel_native(active.id);
    }
    if !coordinator_idle {
        return;
    }
    match &active.stage {
        Stage::Reveal(identity) if identity.same_field(&core.app_status()) => core.hide_field(),
        Stage::RecoveryKit { .. } if core.app_status().phase == "confirm_recovery_kit" => {
            core.lock()
        }
        _ => (),
    }
}

fn close_panel(app: &AppHandle, id: u64) {
    if id != 0 {
        let _ = app.run_on_main_thread(move || {
            native_credential::cancel(id);
        });
    }
}

fn entry_prompt(operation: &NativeOperation, kind: EntryKind) -> Prompt {
    let title = entry_title(operation, kind);
    let detail = entry_detail(operation, kind);
    match kind {
        EntryKind::NewPassword | EntryKind::PasswordChange => Prompt::NewPassword {
            title,
            detail,
            current_password: matches!(kind, EntryKind::PasswordChange),
        },
        EntryKind::Password | EntryKind::Recovery => {
            let recovery = matches!(kind, EntryKind::Recovery);
            Prompt::Credential {
                title,
                detail,
                label: if recovery {
                    "Recovery Key"
                } else {
                    "Master Password"
                }
                .to_owned(),
                recovery,
            }
        }
    }
}

fn kind_name(kind: Option<Kind>) -> &'static str {
    match kind {
        Some(Kind::SeedPhrase) => "Seed Phrase",
        Some(Kind::PrivateKey) => "Private Key",
        Some(Kind::Login) => "Login",
        Some(Kind::ApiKey) => "API Key",
        Some(Kind::Text) => "Text",
        None => "Note",
    }
}

fn field_name(field: Option<&str>) -> &'static str {
    match field {
        Some("words") => "Seed Phrase",
        Some("passphrase") => "Passphrase",
        Some("key") => "Private Key",
        _ => "Hidden Field",
    }
}

fn entry_title(operation: &NativeOperation, entry: EntryKind) -> String {
    let kind = kind_name(operation.kind);
    let field = field_name(operation.field.as_deref());
    let recovery = matches!(entry, EntryKind::Recovery);
    match operation.action {
        "create_vault" => "Create your Master Password".to_owned(),
        "unlock" if recovery => "Unlock with your Recovery Key".to_owned(),
        "unlock" => "Unlock your Vault".to_owned(),
        "set_new_password" => "Set a new Master Password".to_owned(),
        "create_note" => format!("Save new {kind}"),
        "update_note" => format!("Save changes to {kind}"),
        "trash_note" => format!("Move {kind} to Trash"),
        "delete_forever" => format!("Delete {kind} forever"),
        "empty_trash" => "Empty Trash".to_owned(),
        "show_wallet_field" => format!("Show {field}"),
        "show_and_copy_wallet_field" => format!("Show and copy {field}"),
        "copy_field" => format!("Copy {field}"),
        "inspect_backup" if recovery => "Open a Backup with your Recovery Key".to_owned(),
        "inspect_backup" => "Open a Backup".to_owned(),
        "inspect_safety_copy" if recovery => "Open a Safety Copy with your Recovery Key".to_owned(),
        "inspect_safety_copy" => "Open a Safety Copy".to_owned(),
        "change_password" => "Change Master Password".to_owned(),
        "rotate_keys" => "Change Master Password and Recovery Key".to_owned(),
        "set_settings" => "Let the app lock less often?".to_owned(),
        "enroll_quick_unlock" => "Turn on Touch ID for this Vault".to_owned(),
        "agent_store" => format!("Save new {kind}"),
        "agent_edit" => format!("Change the details of this {kind}"),
        "agent_replace" => format!("Replace the value of this {kind}"),
        "agent_trash" => format!("Move {kind} to Trash"),
        _ => "Confirm encrypted-note action".to_owned(),
    }
}

fn entry_detail(operation: &NativeOperation, entry: EntryKind) -> String {
    let recovery = matches!(entry, EntryKind::Recovery);
    let mut detail = match operation.action {
        "create_vault" => {
            "You'll type this to open your Vault. This app doesn't save it. If you forget it, only your Recovery Key can replace it. Use at least 15 characters; 5 or 6 random words work well. Stay in this window: switching apps clears these boxes."
        }
        "unlock" if recovery => {
            "Type the Recovery Key from your paper. It has 28 characters in 7 groups of 4. Capitals, spaces and dashes don't matter."
        }
        "unlock" => "Type your Master Password. This app doesn't save it.",
        "set_new_password" => {
            "You opened your Vault with your Recovery Key. Choose a new Master Password with at least 15 characters; 5 or 6 random words work well. Your Recovery Key keeps working. Stay in this window: switching apps clears these boxes."
        }
        "change_password" => {
            "Type your current Master Password, then choose a new one with at least 15 characters; 5 or 6 random words work well. If you forget it, only your Recovery Key can replace it. Stay in this window: switching apps clears these boxes."
        }
        "rotate_keys" => {
            "Type your current Master Password, then choose a new one with at least 15 characters. Next you will write down a new Recovery Key. Stay in this window: switching apps clears these boxes."
        }
        "inspect_backup" | "inspect_safety_copy" if recovery => {
            "Type the Recovery Key this copy opens with. Capitals, spaces and dashes don't matter."
        }
        "inspect_backup" | "inspect_safety_copy" => {
            "Type the Master Password this copy opens with. It may be an older one."
        }
        "create_note"
        | "update_note"
        | "trash_note"
        | "delete_forever"
        | "show_wallet_field"
        | "show_and_copy_wallet_field"
        | "copy_field" => "Seed Phrases and Private Keys need your Master Password every time.",
        "empty_trash" => {
            "Your Trash holds at least one Seed Phrase or Private Key. This deletes everything in Trash forever from this copy of your Vault. Type your Master Password to confirm."
        }
        "set_settings" => {
            "This makes the app lock less often. Type your Master Password only if you asked for this."
        }
        "enroll_quick_unlock" => "Type your Master Password. Then you will use Touch ID.",
        // The agent route (ADR-0012): the app can't tell which agent asked, so it says "an agent".
        "agent_store" => {
            "An agent asked to store this key. Seed Phrases and Private Keys need your Master Password every time."
        }
        "agent_edit" => {
            "An agent asked to change the details of this key. Type your Master Password only if you allowed this."
        }
        "agent_replace" => {
            "An agent asked to replace this value. Seed Phrases and Private Keys need your Master Password every time."
        }
        "agent_trash" => {
            "An agent asked to delete this key. Type your Master Password to move it to Trash, where it stays for 30 days."
        }
        _ => "Type your Master Password to continue.",
    }
    .to_owned();
    for line in [target_line(operation), copy_warning(operation)]
        .into_iter()
        .flatten()
    {
        detail.push('\n');
        detail.push_str(&line);
    }
    detail
}

/// The value screen only names its target, and keeps the warning when the value is being copied.
fn reveal_detail(operation: &NativeOperation) -> String {
    let warning = (operation.action == "show_and_copy_wallet_field")
        .then(|| copy_warning(operation))
        .flatten();
    [target_line(operation), warning]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("\n")
}

fn target_line(operation: &NativeOperation) -> Option<String> {
    let title: String = operation
        .target
        .as_ref()?
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect();
    Some(format!("For: {title}"))
}

fn copy_warning(operation: &NativeOperation) -> Option<String> {
    let copying = matches!(
        operation.action,
        "copy_field" | "show_and_copy_wallet_field"
    );
    let wallet = matches!(operation.kind, Some(Kind::SeedPhrase | Kind::PrivateKey));
    (copying && wallet).then(|| {
        format!(
            "Copying a {} can expose your wallet. Other apps can read what you copy, and clipboard-history apps and the app you paste into may keep it. Continue only if you must paste it. This app clears its copy after 30 seconds, unless the clipboard has changed by then.",
            kind_name(operation.kind)
        )
    })
}

fn kit_detail(rotation: bool) -> &'static str {
    if rotation {
        "No file is created: this new key exists only here and on your paper. Stay in this window until you finish. If you switch apps, this step ends, nothing changes, and your old password and old Recovery Key still work. Before you finish, compare all 7 groups with your paper. Capitals, spaces and dashes don't matter when you type it back."
    } else {
        "No file is created: this key exists only here and on your paper. Stay in this window until you finish, because switching apps cancels this step and the key then stops working. Before you finish, compare all 7 groups with your paper. Capitals, spaces and dashes don't matter when you type it back."
    }
}

/// Numbered rows of three, so a word is never split across lines. Display only: the clipboard
/// gets the core's own value.
fn seed_grid(words: &str) -> Zeroizing<String> {
    const COLUMNS: usize = 3;
    const WORD_WIDTH: usize = 8;
    // Reserved up front: growing would leave an unwiped copy behind.
    let capacity = words
        .split_whitespace()
        .map(|word| word.len() + 32)
        .sum::<usize>();
    let mut grid = Zeroizing::new(String::with_capacity(capacity));
    let mut last_width = 0;
    for (index, word) in words.split_whitespace().enumerate() {
        if index % COLUMNS != 0 {
            for _ in last_width..WORD_WIDTH {
                grid.push(' ');
            }
            grid.push_str("  ");
        } else if index > 0 {
            grid.push('\n');
        }
        let _ = write!(grid, "{:>2} {word}", index + 1);
        last_width = word.chars().count();
    }
    grid
}

fn credentials(kind: EntryKind, reply: Reply) -> Result<NativeCredentials, ApiError> {
    Ok(match (kind, reply) {
        (EntryKind::Password, Reply::Credential(value)) => NativeCredentials::Password(value),
        (EntryKind::Recovery, Reply::Credential(value)) => NativeCredentials::RecoveryKey(value),
        (EntryKind::NewPassword, Reply::NewPassword { current: None, new }) => {
            NativeCredentials::NewPassword { password: new }
        }
        (
            EntryKind::PasswordChange,
            Reply::NewPassword {
                current: Some(current),
                new,
            },
        ) => NativeCredentials::PasswordChange {
            current,
            password: new,
        },
        _ => return Err(cancelled()),
    })
}

fn last_groups(recovery: &str) -> Result<Zeroizing<String>, ApiError> {
    let mut groups = recovery.rsplit('-');
    let last = groups.next().ok_or_else(unavailable)?;
    let previous = groups.next().ok_or_else(unavailable)?;
    if last.len() != 4 || previous.len() != 4 || !last.is_ascii() || !previous.is_ascii() {
        return Err(unavailable());
    }
    let mut confirmation = Zeroizing::new(String::with_capacity(8));
    confirmation.push_str(previous);
    confirmation.push_str(last);
    Ok(confirmation)
}

fn cancelled() -> ApiError {
    ApiError {
        code: "cancelled",
        message: "That was cancelled. Please try again.".to_owned(),
    }
}
fn interrupted() -> ApiError {
    ApiError {
        code: "interrupted",
        message: "This step closed before it finished, so please do it again. This happens if you switch to another app or window, a screen sharing or recording app starts, the app or computer locks, or the time runs out. Your Notes and password were not changed.".to_owned(),
    }
}
fn busy_error() -> ApiError {
    ApiError {
        code: "invalid_input",
        message: "Another step is still running. Finish it, or wait a moment, then try again."
            .to_owned(),
    }
}
fn unavailable() -> ApiError {
    ApiError {
        code: "internal",
        message: "That step couldn't start or finish. Click this window, then try again. If it keeps happening, quit the app and open it again.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlapping_actions_and_late_callbacks_cannot_reuse_a_lease() {
        let state = CoordinatorState::default();
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let first = state.reserve(deadline).unwrap();
        assert!(state.reserve(deadline).is_err());
        assert!(state.stop(first).is_some());
        assert!(state.idle());
        let second = state.reserve(deadline).unwrap();
        assert!(!state.publish(first, 9, deadline, Stage::Entry));
        assert!(state.stop(first).is_none());
        assert!(state.live(second));
        assert!(state.publish(second, 10, deadline, Stage::RecoveryKit { epoch: 0 }));
        assert_eq!(state.snapshot(second).unwrap().id, 10);
    }

    #[test]
    fn stop_keeps_ownership_until_panel_metadata_can_be_removed() {
        use std::sync::Arc;
        let state = Arc::new(CoordinatorState::default());
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let first = state.reserve(deadline).unwrap();
        assert!(state.publish(first, 41, deadline, Stage::RecoveryKit { epoch: 0 }));
        let metadata = state.active.lock().unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (done_tx, done_rx) = mpsc::channel();
        let worker_state = state.clone();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            done_tx.send(worker_state.stop(first)).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());
        assert_eq!(state.current.load(Ordering::SeqCst), first);
        drop(metadata);
        assert_eq!(
            done_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .unwrap()
                .id,
            41
        );
        worker.join().unwrap();
        let second = state.reserve(deadline).unwrap();
        assert!(state.live(second));
        assert!(state.publish(second, 42, deadline, Stage::Entry));
        assert_eq!(state.snapshot(second).unwrap().id, 42);
    }

    struct FakeClock(std::sync::Arc<AtomicU64>);
    impl crate::core::Clock for FakeClock {
        fn now_ms(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
        fn unix_secs(&self) -> i64 {
            1_790_000_000
        }
    }
    struct FakeClipboard;
    impl crate::core::Clipboard for FakeClipboard {
        fn write(&mut self, _: &str) -> Result<u64, ApiError> {
            Ok(1)
        }
        fn clear_if(&mut self, _: u64) -> bool {
            true
        }
    }
    fn fake_core() -> (AppCore, tempfile::TempDir) {
        let (core, temp, _) = fake_core_with_clock();
        (core, temp)
    }
    fn fake_core_with_clock() -> (AppCore, tempfile::TempDir, std::sync::Arc<AtomicU64>) {
        let temp = tempfile::tempdir().unwrap();
        let now = std::sync::Arc::new(AtomicU64::new(1_000));
        let core = AppCore::new(crate::core::Config {
            data_dir: temp.path().join("Vault.noindex"),
            home_dir: temp.path().to_owned(),
            limits: vault_format::Limits::TESTING,
            clock: Box::new(FakeClock(now.clone())),
            clipboard: Box::new(FakeClipboard),
            activity_anywhere_supported: false,
        })
        .unwrap();
        (core, temp, now)
    }
    fn setup_fake_vault(core: &mut AppCore) -> Zeroizing<String> {
        core.create_vault(
            Zeroizing::new("fake-lantern-quartz-mosaic-velvet-orbit".to_owned()),
            vault_format::KdfParams::TESTING,
        )
        .unwrap()
        .recovery_key
        .unwrap()
    }

    #[test]
    fn no_vault_context_changes_cancel_calibration_even_without_a_phase_change() {
        let (mut core, _temp) = fake_core();
        let status = core.app_status();
        let active = Active {
            token: 1,
            id: 0,
            deadline: Instant::now() + OPERATION_TIMEOUT,
            stage: Stage::Calibration {
                epoch: core.native_operation_epoch(),
                generation: status.session_generation,
                phase: status.phase,
            },
        };
        assert!(active_matches(&active, &core));
        core.hide_field();
        assert_eq!(core.app_status().phase, status.phase);
        assert_eq!(
            core.app_status().session_generation,
            status.session_generation
        );
        assert!(!active_matches(&active, &core));
    }

    #[test]
    fn a_later_reservation_cannot_orphan_an_owned_recovery_candidate() {
        let (mut core, _temp) = fake_core();
        let _fake_key = setup_fake_vault(&mut core);
        let operation = core.begin_native(NativeAction::ConfirmRecoveryKit).unwrap();
        let active = Active {
            token: 1,
            id: operation.id,
            deadline: Instant::now() + OPERATION_TIMEOUT,
            stage: Stage::RecoveryKit {
                epoch: core.native_operation_epoch(),
            },
        };
        cleanup_core(&mut core, &active, false);
        assert_eq!(core.app_status().phase, "no_vault");
        assert!(core.native_pending_id().is_none());
    }

    #[test]
    fn a_document_mutation_invalidates_and_clears_the_native_readout() {
        use std::collections::BTreeMap;
        let (mut core, _temp) = fake_core();
        let fake_key = setup_fake_vault(&mut core);
        core.confirm_recovery_kit(&last_groups(&fake_key).unwrap())
            .unwrap();
        let id = core
            .create_note(
                crate::core::NoteInput {
                    kind: Kind::Text,
                    title: "Fake note".to_owned(),
                    visible: BTreeMap::new(),
                    hidden: BTreeMap::from([(
                        "body".to_owned(),
                        Zeroizing::new("fake-body-only".to_owned()),
                    )]),
                },
                None,
            )
            .unwrap();
        let _fake_value = core.show_field(&id, "body").unwrap();
        let identity = RevealIdentity {
            epoch: core.native_operation_epoch(),
            generation: core.app_status().session_generation,
            note_id: id.clone(),
            field: "body".to_owned(),
        };
        assert!(identity.matches(&core.app_status(), core.native_operation_epoch()));
        core.set_favorite(&id, true).unwrap();
        assert!(identity.same_field(&core.app_status()));
        assert!(!identity.matches(&core.app_status(), core.native_operation_epoch()));
        cleanup_core(
            &mut core,
            &Active {
                token: 1,
                id: 0,
                deadline: Instant::now() + OPERATION_TIMEOUT,
                stage: Stage::Reveal(identity),
            },
            true,
        );
        assert!(core.app_status().reveal.is_none());
    }

    fn authorized_for_test(
        core: &mut AppCore,
        state: &CoordinatorState,
        token: u64,
        id: Option<u64>,
        genuine_owner_activity: bool,
    ) -> bool {
        let Some(active) = state.snapshot(token) else {
            return false;
        };
        authorize_core_callback(
            core,
            state,
            token,
            active.deadline,
            id,
            genuine_owner_activity,
        ) == Authorization::Allowed
    }

    fn fake_wallet_readout(core: &mut AppCore, state: &CoordinatorState) -> u64 {
        fake_wallet_readout_mode(core, state, ShowMode::Held)
    }

    fn fake_wallet_readout_mode(
        core: &mut AppCore,
        state: &CoordinatorState,
        mode: ShowMode,
    ) -> u64 {
        use std::collections::BTreeMap;
        let fake_key = setup_fake_vault(core);
        core.confirm_recovery_kit(&last_groups(&fake_key).unwrap())
            .unwrap();
        // Keep the idle deadline earlier than the independent two-minute held-reveal backstop.
        core.set_settings(
            serde_json::from_value(serde_json::json!({
                "idle_minutes": 1,
                "lock_on_app_switch": false
            }))
            .unwrap(),
        )
        .unwrap();
        let id = core
            .create_note(
                crate::core::NoteInput {
                    kind: Kind::PrivateKey,
                    title: "Fake Wallet only".to_owned(),
                    visible: BTreeMap::new(),
                    hidden: BTreeMap::from([(
                        "key".to_owned(),
                        Zeroizing::new("fake-private-key-only".to_owned()),
                    )]),
                },
                Some(Zeroizing::new(
                    "fake-lantern-quartz-mosaic-velvet-orbit".to_owned(),
                )),
            )
            .unwrap();
        let _fake_value = core
            .show_wallet_field(
                &id,
                "key",
                Zeroizing::new("fake-lantern-quartz-mosaic-velvet-orbit".to_owned()),
                mode,
            )
            .unwrap();
        let status = core.app_status();
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let token = state.reserve(deadline).unwrap();
        assert!(state.publish(
            token,
            71,
            deadline,
            Stage::Reveal(RevealIdentity {
                epoch: core.native_operation_epoch(),
                generation: status.session_generation,
                note_id: id,
                field: "key".to_owned(),
            }),
        ));
        token
    }

    #[test]
    fn fresh_input_cannot_repaint_a_wallet_readout_after_the_idle_deadline() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        let idle_ms = core.app_status().lock_in_ms.unwrap();
        now.fetch_add(idle_ms, Ordering::SeqCst);
        // Simulate a native event arriving before the periodic background tick.
        assert!(active_matches(&state.snapshot(token).unwrap(), &core));
        assert!(!authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            true
        ));
        assert_eq!(core.app_status().phase, "locked");
        assert!(core.app_status().reveal.is_none());
    }

    #[test]
    fn polling_and_key_repeat_never_extend_a_held_wallet_readouts_idle_time() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        let idle_ms = core.app_status().lock_in_ms.unwrap();
        now.fetch_add(idle_ms - 1_000, Ordering::SeqCst);
        assert!(authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            false
        ));
        assert_eq!(core.app_status().lock_in_ms, Some(1_000));
        now.fetch_add(1_000, Ordering::SeqCst);
        assert!(!authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            false
        ));
        assert_eq!(core.app_status().phase, "locked");
        assert!(core.app_status().reveal.is_none());
    }

    #[test]
    fn fresh_owner_input_extends_idle_time_only_for_the_current_live_readout() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        let idle_ms = core.app_status().lock_in_ms.unwrap();
        now.fetch_add(idle_ms - 1_000, Ordering::SeqCst);
        assert!(authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            true
        ));
        assert_eq!(core.app_status().lock_in_ms, Some(idle_ms));
        state.stop(token);
        now.fetch_add(1_000, Ordering::SeqCst);
        assert!(!authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            true
        ));
        assert_eq!(core.app_status().lock_in_ms, Some(idle_ms - 1_000));
    }

    #[test]
    fn held_backstop_hides_the_readout_even_when_owner_activity_keeps_the_vault_open() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        for _ in 0..2 {
            now.fetch_add(50_000, Ordering::SeqCst);
            assert!(authorized_for_test(
                &mut core,
                &state,
                token,
                Some(71),
                true
            ));
        }
        now.fetch_add(20_000, Ordering::SeqCst);
        assert!(active_matches(&state.snapshot(token).unwrap(), &core));
        assert!(!authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            true
        ));
        assert_eq!(core.app_status().phase, "unlocked");
        assert!(core.app_status().reveal.is_none());
        assert_eq!(core.app_status().lock_in_ms, Some(40_000));
    }

    #[test]
    fn temporary_core_contention_retries_without_counting_owner_activity() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        let deadline = state.snapshot(token).unwrap().deadline;
        now.fetch_add(1_000, Ordering::SeqCst);
        let core = Mutex::new(core);
        let held = core.lock().unwrap();
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, Some(71), true),
            Authorization::Retry
        );
        assert_eq!(held.app_status().lock_in_ms, Some(59_000));
        assert_eq!(
            authorize_callback(&core, &state, token, Instant::now(), Some(71), true),
            Authorization::Denied
        );
        drop(held);
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, Some(71), false),
            Authorization::Allowed
        );
        assert_eq!(core.lock().unwrap().app_status().lock_in_ms, Some(59_000));
    }

    #[test]
    fn temporary_metadata_contention_retries_but_a_cancelled_lease_is_denied() {
        let (core, _temp) = fake_core();
        let core = Mutex::new(core);
        let state = CoordinatorState::default();
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let token = state.reserve(deadline).unwrap();
        let metadata = state.active.lock().unwrap();
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, None, true),
            Authorization::Retry
        );
        assert_eq!(
            authorize_callback(&core, &state, token, Instant::now(), None, true),
            Authorization::Denied
        );
        // Cancellation remains observable even when its metadata cleanup has not completed.
        state.current.store(0, Ordering::SeqCst);
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, None, true),
            Authorization::Denied
        );
        drop(metadata);
    }

    #[test]
    fn poisoned_core_or_metadata_is_denied_instead_of_retried() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        let (core, _temp) = fake_core();
        let core = Mutex::new(core);
        let state = CoordinatorState::default();
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let token = state.reserve(deadline).unwrap();
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let _held = core.lock().unwrap();
                panic!("fictional core poisoning for a regression test");
            }))
            .is_err()
        );
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, None, false),
            Authorization::Denied
        );
        let (core, _other_temp) = fake_core();
        let core = Mutex::new(core);
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let _held = state.active.lock().unwrap();
                panic!("fictional metadata poisoning for a regression test");
            }))
            .is_err()
        );
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, None, false),
            Authorization::Denied
        );
    }

    #[test]
    fn core_contention_does_not_close_or_consume_a_native_extension() {
        use std::cell::Cell;
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout_mode(&mut core, &state, ShowMode::Accessible);
        let deadline = state.snapshot(token).unwrap().deadline;
        let notifications = Cell::new(0);
        now.fetch_add(5_000, Ordering::SeqCst);
        let core = Mutex::new(core);
        let held = core.lock().unwrap();
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| notifications.set(1)),
            Authorization::Retry
        );
        let shown = held.app_status().reveal.unwrap();
        assert_eq!(shown.extensions_left, Some(3));
        assert_eq!(shown.expires_in_ms, Some(15_000));
        assert_eq!(notifications.get(), 0);
        assert_eq!(
            extend_callback(&core, &state, token, Instant::now(), |_| notifications
                .set(1)),
            Authorization::Denied
        );
        drop(held);
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| notifications.set(1)),
            Authorization::Allowed
        );
        let shown = core.lock().unwrap().app_status().reveal.unwrap();
        assert_eq!(shown.extensions_left, Some(2));
        assert_eq!(shown.expires_in_ms, Some(20_000));
        assert_eq!(notifications.get(), 1);
    }

    #[test]
    fn metadata_contention_retries_an_extension_but_never_revives_an_expired_readout() {
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout_mode(&mut core, &state, ShowMode::Accessible);
        let deadline = state.snapshot(token).unwrap().deadline;
        let core = Mutex::new(core);
        let metadata = state.active.lock().unwrap();
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| panic!("not extended")),
            Authorization::Retry
        );
        assert_eq!(
            core.lock()
                .unwrap()
                .app_status()
                .reveal
                .unwrap()
                .extensions_left,
            Some(3)
        );
        now.fetch_add(20_000, Ordering::SeqCst);
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| panic!("not extended")),
            Authorization::Denied
        );
        assert!(core.lock().unwrap().app_status().reveal.is_none());
        drop(metadata);
    }

    #[test]
    fn native_extension_limit_remains_a_denial_after_three_successful_extensions() {
        let (mut core, _temp, _now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout_mode(&mut core, &state, ShowMode::Accessible);
        let deadline = state.snapshot(token).unwrap().deadline;
        let core = Mutex::new(core);
        for _ in 0..3 {
            assert_eq!(
                extend_callback(&core, &state, token, deadline, |_| ()),
                Authorization::Allowed
            );
        }
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| panic!("not extended")),
            Authorization::Denied
        );
        assert_eq!(
            core.lock()
                .unwrap()
                .app_status()
                .reveal
                .unwrap()
                .extensions_left,
            Some(0)
        );
    }

    #[test]
    fn touch_id_enrollment_prompt_names_the_action() {
        let operation = NativeOperation {
            id: 1,
            action: "enroll_quick_unlock",
            target: None,
            kind: None,
            field: None,
            requires_credentials: true,
            limit_ms: 120_000,
        };
        let Prompt::Credential { title, label, .. } = entry_prompt(&operation, EntryKind::Password)
        else {
            panic!("Touch ID enrollment requires native credential entry");
        };
        assert_eq!(title, "Turn on Touch ID for this Vault");
        assert_eq!(label, "Master Password");
    }

    fn fake_operation(
        action: &'static str,
        kind: Option<Kind>,
        field: Option<&str>,
    ) -> NativeOperation {
        NativeOperation {
            id: 1,
            action,
            target: kind.map(|_| "Fake Wallet".to_owned()),
            kind,
            field: field.map(str::to_owned),
            requires_credentials: true,
            limit_ms: 120_000,
        }
    }

    #[test]
    fn wallet_copy_warning_is_owned_by_the_native_prompt() {
        const WARNING: &str = "can expose your wallet. Other apps can read what you copy";
        for (kind, name) in [
            (Kind::SeedPhrase, "Copying a Seed Phrase"),
            (Kind::PrivateKey, "Copying a Private Key"),
        ] {
            for action in ["copy_field", "show_and_copy_wallet_field"] {
                let operation = fake_operation(action, Some(kind), Some("words"));
                let detail = entry_detail(&operation, EntryKind::Password);
                assert!(
                    detail.contains(WARNING) && detail.contains(name),
                    "{detail}"
                );
                assert!(detail.contains("\nFor: Fake Wallet\n"), "{detail}");
                assert!(detail.contains("clears its copy after 30 seconds"));
            }
            let readout = fake_operation("show_and_copy_wallet_field", Some(kind), Some("words"));
            assert!(reveal_detail(&readout).contains(WARNING));
            let show = fake_operation("show_wallet_field", Some(kind), Some("words"));
            assert!(!entry_detail(&show, EntryKind::Password).contains(WARNING));
            assert_eq!(reveal_detail(&show), "For: Fake Wallet");
        }
        let login = fake_operation("copy_field", Some(Kind::Login), Some("password"));
        assert!(!entry_detail(&login, EntryKind::Password).contains(WARNING));
    }

    #[test]
    fn every_action_has_a_plain_title() {
        let title = |action, kind, field| {
            entry_title(&fake_operation(action, kind, field), EntryKind::Password)
        };
        let seed = Some(Kind::SeedPhrase);
        assert_eq!(
            title("create_vault", None, None),
            "Create your Master Password"
        );
        assert_eq!(title("unlock", None, None), "Unlock your Vault");
        assert_eq!(
            title("set_new_password", None, None),
            "Set a new Master Password"
        );
        assert_eq!(title("create_note", seed, None), "Save new Seed Phrase");
        assert_eq!(
            title("update_note", Some(Kind::PrivateKey), None),
            "Save changes to Private Key"
        );
        assert_eq!(
            title("trash_note", Some(Kind::Login), None),
            "Move Login to Trash"
        );
        assert_eq!(
            title("delete_forever", Some(Kind::ApiKey), None),
            "Delete API Key forever"
        );
        assert_eq!(
            title("create_note", Some(Kind::Text), None),
            "Save new Text"
        );
        assert_eq!(title("empty_trash", None, None), "Empty Trash");
        assert_eq!(
            title("show_wallet_field", seed, Some("words")),
            "Show Seed Phrase"
        );
        assert_eq!(
            title("show_and_copy_wallet_field", seed, Some("passphrase")),
            "Show and copy Passphrase"
        );
        assert_eq!(
            title("copy_field", Some(Kind::PrivateKey), Some("key")),
            "Copy Private Key"
        );
        assert_eq!(
            title("copy_field", seed, Some("other")),
            "Copy Hidden Field"
        );
        assert_eq!(title("inspect_backup", None, None), "Open a Backup");
        assert_eq!(
            title("inspect_safety_copy", None, None),
            "Open a Safety Copy"
        );
        assert_eq!(
            title("change_password", None, None),
            "Change Master Password"
        );
        assert_eq!(
            title("rotate_keys", None, None),
            "Change Master Password and Recovery Key"
        );
        assert_eq!(
            title("set_settings", None, None),
            "Let the app lock less often?"
        );
        assert_eq!(
            title("enroll_quick_unlock", None, None),
            "Turn on Touch ID for this Vault"
        );
        assert_eq!(title("agent_store", seed, None), "Save new Seed Phrase");
        assert_eq!(
            title("agent_edit", Some(Kind::PrivateKey), None),
            "Change the details of this Private Key"
        );
        assert_eq!(
            title("agent_replace", Some(Kind::ApiKey), None),
            "Replace the value of this API Key"
        );
        assert_eq!(
            title("agent_trash", Some(Kind::Login), None),
            "Move Login to Trash"
        );
        assert_eq!(
            title("fake_action", None, None),
            "Confirm encrypted-note action"
        );
        for (action, expected) in [
            ("unlock", "Unlock with your Recovery Key"),
            ("inspect_backup", "Open a Backup with your Recovery Key"),
            (
                "inspect_safety_copy",
                "Open a Safety Copy with your Recovery Key",
            ),
        ] {
            let operation = fake_operation(action, None, None);
            assert_eq!(entry_title(&operation, EntryKind::Recovery), expected);
        }
    }

    #[test]
    fn panel_text_tells_the_owner_what_to_do_without_promising_too_much() {
        let trash = entry_detail(
            &fake_operation("empty_trash", None, None),
            EntryKind::Password,
        );
        assert!(trash.contains("Seed Phrase or Private Key") && trash.contains("forever"));
        let unlock = fake_operation("unlock", None, None);
        assert!(entry_detail(&unlock, EntryKind::Recovery).contains("28 characters in 7 groups"));
        assert_eq!(
            entry_detail(&unlock, EntryKind::Password),
            "Type your Master Password. This app doesn't save it."
        );
        for action in [
            "create_vault",
            "unlock",
            "set_new_password",
            "create_note",
            "update_note",
            "trash_note",
            "delete_forever",
            "empty_trash",
            "show_wallet_field",
            "show_and_copy_wallet_field",
            "copy_field",
            "inspect_backup",
            "inspect_safety_copy",
            "change_password",
            "rotate_keys",
            "set_settings",
            "enroll_quick_unlock",
            "agent_store",
            "agent_edit",
            "agent_replace",
            "agent_trash",
            "fake_action",
        ] {
            let operation = fake_operation(action, Some(Kind::SeedPhrase), Some("words"));
            for entry in [EntryKind::Password, EntryKind::Recovery] {
                let text = entry_detail(&operation, entry).to_lowercase();
                assert!(
                    !text.contains("only you ") && !text.contains("only you."),
                    "{text}"
                );
                assert!(!text.contains("nobody can reset"), "{text}");
                assert!(!text.contains("native"), "{text}");
            }
        }
        for rotation in [false, true] {
            assert!(
                kit_detail(rotation)
                    .contains("Before you finish, compare all 7 groups with your paper.")
            );
        }
        assert!(!kit_detail(true).contains("stops working"));
        assert!(kit_detail(true).contains("old Recovery Key still work"));
    }

    #[test]
    fn target_line_stays_one_short_line() {
        let mut operation = fake_operation("trash_note", Some(Kind::Login), None);
        operation.target = Some(format!("Fake\nname\u{7}{}", "x".repeat(200)));
        let line = target_line(&operation).unwrap();
        assert!(line.starts_with("For: Fakename"));
        assert_eq!(line.chars().count(), "For: ".len() + 120);
        assert!(!line.contains('\n'));
        operation.target = None;
        assert_eq!(target_line(&operation), None);
        assert_eq!(reveal_detail(&operation), "");
    }

    fn fake_words(count: usize) -> String {
        (1..=count)
            .map(|n| format!("fake{n:02}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn seed_phrase_readout_is_numbered_rows_with_no_split_words() {
        for count in [12, 24] {
            let words = fake_words(count);
            let grid = seed_grid(&words);
            let rows: Vec<&str> = grid.lines().collect();
            assert_eq!(rows.len(), count / 3);
            assert!(rows.iter().all(|row| row.chars().count() <= 48));
            let cells: Vec<&str> = grid.split_whitespace().collect();
            let numbers: Vec<String> = cells.iter().step_by(2).map(|n| (*n).to_owned()).collect();
            let expected: Vec<String> = (1..=count).map(|n| n.to_string()).collect();
            assert_eq!(numbers, expected);
            let rejoined: Vec<&str> = cells.iter().skip(1).step_by(2).copied().collect();
            assert_eq!(rejoined.join(" "), words);
            assert!(grid.capacity() >= grid.len());
        }
        let grid = seed_grid("zoo   fake\nabandon wrong");
        assert_eq!(
            grid.as_str(),
            " 1 zoo        2 fake       3 abandon\n 4 wrong"
        );
        assert_eq!(seed_grid("").as_str(), "");
    }

    fn reporting_state(deadline: Instant) -> (CoordinatorState, u64) {
        let state = CoordinatorState::default();
        let token = state.reserve(deadline).unwrap();
        state.reporting.store(token, Ordering::SeqCst);
        (state, token)
    }

    #[test]
    fn only_a_stop_the_owner_did_not_ask_for_says_so() {
        let deadline = Instant::now() + OPERATION_TIMEOUT;

        // An OS event or the Sharing Guard marks it before stopping.
        let (state, token) = reporting_state(deadline);
        state.interrupted.store(token, Ordering::SeqCst);
        assert!(state.stop(token).is_some());
        assert_eq!(check_state(&state, token).unwrap_err().code, "interrupted");
        assert_eq!(
            prompt_failure(&state, token, PromptError::Cancelled).code,
            "interrupted"
        );
        assert_eq!(
            prompt_failure(&state, token, PromptError::Interrupted).code,
            "interrupted"
        );

        // An owner route doesn't mark it.
        let (state, token) = reporting_state(deadline);
        assert!(state.stop(token).is_some());
        assert_eq!(check_state(&state, token).unwrap_err().code, "cancelled");
        for error in [PromptError::Cancelled, PromptError::Interrupted] {
            assert_eq!(prompt_failure(&state, token, error).code, "cancelled");
        }

        // The Cancel button or Escape, with nothing else going on.
        let (state, token) = reporting_state(deadline);
        assert_eq!(
            prompt_failure(&state, token, PromptError::Cancelled).code,
            "cancelled"
        );
        // The panel closing by itself.
        assert_eq!(
            prompt_failure(&state, token, PromptError::Interrupted).code,
            "interrupted"
        );
        assert_eq!(
            prompt_failure(&state, token, PromptError::Busy).code,
            "invalid_input"
        );
        assert_eq!(
            prompt_failure(&state, token, PromptError::Unavailable).code,
            "internal"
        );
    }

    #[test]
    fn a_step_that_is_not_reporting_stays_cancelled() {
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let (state, token) = reporting_state(deadline);
        state.stop_reporting(token);
        assert_eq!(ended(&state, token).code, "cancelled");
        state.interrupted.store(token, Ordering::SeqCst);
        assert!(state.stop(token).is_some());
        assert_eq!(ended(&state, token).code, "cancelled");
        assert_eq!(
            prompt_failure(&state, token, PromptError::Interrupted).code,
            "cancelled"
        );
        // A late step can't silence a newer one.
        let other = state.reserve(deadline).unwrap();
        state.reporting.store(other, Ordering::SeqCst);
        state.stop_reporting(token);
        assert_eq!(state.reporting.load(Ordering::SeqCst), other);
    }

    #[test]
    fn running_out_of_time_while_reporting_is_an_interruption() {
        let now = Instant::now();
        let (state, token) = reporting_state(now);
        assert_eq!(check_state(&state, token).unwrap_err().code, "interrupted");
        // A reply after the limit isn't revived, and a live step is left alone.
        assert!(state.publish(token, 1, now + OPERATION_TIMEOUT, Stage::Entry));
        assert!(check_state(&state, token).is_ok());
        let message = interrupted().message;
        assert!(message.contains("Your Notes and password were not changed."));
    }

    #[test]
    fn a_new_password_panel_needs_real_input_to_stay_open() {
        let (mut core, _temp) = fake_core();
        let state = CoordinatorState::default();
        let now = Instant::now();
        let cap = now + Duration::from_secs(10 * 60);
        let token = state.reserve(cap).unwrap();
        state.expect_input(token, now);
        assert_eq!(
            state.input_left(token, now),
            Some(Duration::from_secs(5 * 60))
        );
        assert!(!state.input_expired(token, now + Duration::from_millis(NO_INPUT_MS - 1)));
        assert!(state.input_expired(token, now + Duration::from_millis(NO_INPUT_MS)));

        // Key repeat and programmatic edits reach the hook as false: they change nothing.
        let by = state.input_by.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(5));
        assert!(authorized_for_test(&mut core, &state, token, None, false));
        assert_eq!(state.input_by.load(Ordering::SeqCst), by);

        // A real key or click restarts it.
        assert!(authorized_for_test(&mut core, &state, token, None, true));
        assert!(state.input_by.load(Ordering::SeqCst) > by);

        // Run out: the panel ends, and input after that doesn't bring it back.
        let at_limit = state.input_clock_ms(Instant::now());
        state.input_by.store(at_limit, Ordering::SeqCst);
        assert!(!authorized_for_test(&mut core, &state, token, None, false));
        assert!(!authorized_for_test(&mut core, &state, token, None, true));
        assert_eq!(state.input_by.load(Ordering::SeqCst), at_limit);
    }

    #[test]
    fn typing_never_outlasts_the_cap_of_a_new_password_panel() {
        let (mut core, _temp) = fake_core();
        let state = CoordinatorState::default();
        let now = Instant::now();
        let token = state.reserve(now + OPERATION_TIMEOUT).unwrap();
        state.expect_input(token, now);
        assert!(authorized_for_test(&mut core, &state, token, None, true));
        assert!(state.publish(token, 0, Instant::now(), Stage::Entry));
        assert!(!authorized_for_test(&mut core, &state, token, None, true));
    }

    #[test]
    fn other_steps_have_no_input_rule() {
        let (mut core, _temp) = fake_core();
        let state = CoordinatorState::default();
        let now = Instant::now();
        let token = state.reserve(now + OPERATION_TIMEOUT).unwrap();
        assert_eq!(state.input_left(token, now), None);
        assert!(!state.input_expired(token, now + Duration::from_secs(3_600)));
        assert!(authorized_for_test(&mut core, &state, token, None, true));
        // Another step's rule doesn't reach this one.
        state.expect_input(token + 1, now);
        assert_eq!(state.input_left(token, now), None);
    }

    #[test]
    fn commit_time_is_given_once_and_only_to_long_entries() {
        let now = Instant::now();
        let soon = now + Duration::from_secs(1);
        let later = now + Duration::from_secs(9 * 60);

        let (state, token) = reporting_state(soon);
        let mut operation = fake_operation("unlock", None, None);
        // A password prompt gets no extended window.
        assert!(allow_commit_time(&state, token, &operation).is_ok());
        assert_eq!(state.snapshot(token).unwrap().deadline, soon);

        // A long entry sent at its last moment does, and nothing more is expected from the owner.
        operation.limit_ms = 300_000;
        state.expect_input(token, now);
        assert!(allow_commit_time(&state, token, &operation).is_ok());
        assert!(state.snapshot(token).unwrap().deadline >= now + OPERATION_TIMEOUT);
        assert_eq!(state.input_left(token, now), None);

        // The later limit is kept.
        assert!(state.publish(token, 1, later, Stage::Entry));
        assert!(allow_commit_time(&state, token, &operation).is_ok());
        assert_eq!(state.snapshot(token).unwrap().deadline, later);

        // A stopped step isn't brought back.
        state.interrupted.store(token, Ordering::SeqCst);
        assert!(state.stop(token).is_some());
        assert_eq!(
            allow_commit_time(&state, token, &operation)
                .unwrap_err()
                .code,
            "interrupted"
        );
        assert!(state.idle());
    }

    #[test]
    fn time_row_reads_the_sooner_limit_and_changes_nothing() {
        let (mut core, _temp, now_ms) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        let core = Mutex::new(core);
        let now = Instant::now();
        let at = |seconds| now + Duration::from_secs(seconds);
        let left = |seconds, limit| TimeLeft::Left { seconds, limit };

        // Auto-lock at one minute warns only in its last 30 seconds.
        assert!(state.publish(token, 71, at(600), Stage::Entry));
        assert_eq!(time_left(&core, &state, token, now), TimeLeft::Quiet);
        now_ms.fetch_add(31_000, Ordering::SeqCst);
        assert_eq!(
            time_left(&core, &state, token, now),
            left(29, TimeLimit::AutoLock)
        );
        // Asking neither locks the app nor counts as activity.
        now_ms.fetch_add(29_000, Ordering::SeqCst);
        assert_eq!(
            time_left(&core, &state, token, now),
            left(0, TimeLimit::AutoLock)
        );
        assert_eq!(core.lock().unwrap().app_status().phase, "unlocked");
        assert_eq!(core.lock().unwrap().app_status().lock_in_ms, Some(0));
    }

    #[test]
    fn time_row_warns_about_the_panel_and_stays_calm_on_the_kit() {
        let (core, _temp) = fake_core();
        let core = Mutex::new(core);
        let state = CoordinatorState::default();
        let now = Instant::now();
        let at = |seconds| now + Duration::from_secs(seconds);
        let token = state.reserve(at(61)).unwrap();
        assert_eq!(time_left(&core, &state, token, now), TimeLeft::Quiet);
        assert!(state.publish(token, 0, at(60), Stage::Entry));
        assert_eq!(
            time_left(&core, &state, token, now),
            TimeLeft::Left {
                seconds: 60,
                limit: TimeLimit::Panel
            }
        );
        // A partial second still shows, rounded up.
        assert!(state.publish(token, 0, now + Duration::from_millis(500), Stage::Entry));
        assert_eq!(
            time_left(&core, &state, token, now),
            TimeLeft::Left {
                seconds: 1,
                limit: TimeLimit::Panel
            }
        );

        // The no-input limit counts as the panel's own.
        assert!(state.publish(token, 0, at(600), Stage::Entry));
        state.expect_input(token, now);
        assert_eq!(
            time_left(
                &core,
                &state,
                token,
                now + Duration::from_millis(NO_INPUT_MS - 45_000)
            ),
            TimeLeft::Left {
                seconds: 45,
                limit: TimeLimit::Panel
            }
        );
        state.end_input_rule(token);

        assert!(state.publish(token, 0, at(600), Stage::RecoveryKit { epoch: 0 }));
        assert_eq!(
            time_left(&core, &state, token, now),
            TimeLeft::KitCalm { idle_minutes: 5 }
        );
        assert!(state.publish(token, 0, at(5), Stage::RecoveryKit { epoch: 0 }));
        assert_eq!(
            time_left(&core, &state, token, now),
            TimeLeft::Left {
                seconds: 5,
                limit: TimeLimit::Panel
            }
        );
    }

    #[test]
    fn time_row_is_left_alone_while_anything_is_busy() {
        let (core, _temp) = fake_core();
        let core = Mutex::new(core);
        let state = CoordinatorState::default();
        let now = Instant::now();
        let token = state.reserve(now + OPERATION_TIMEOUT).unwrap();
        {
            let _held = core.lock().unwrap();
            assert_eq!(time_left(&core, &state, token, now), TimeLeft::Busy);
        }
        {
            let _held = state.active.lock().unwrap();
            assert_eq!(time_left(&core, &state, token, now), TimeLeft::Busy);
        }
        assert_eq!(time_left(&core, &state, token, now), TimeLeft::Quiet);
        assert!(state.stop(token).is_some());
        assert_eq!(time_left(&core, &state, token, now), TimeLeft::Busy);
    }

    #[test]
    fn showing_typed_text_is_refused_while_a_sharing_app_runs_even_after_a_dismissal() {
        let (mut core, _temp) = fake_core();
        let zoom = [guard::RunningApp {
            bundle_id: Some("us.zoom.xos".into()),
            exe_name: Some("Zoom.exe".into()),
        }];
        core.guard_update(&zoom);
        core.dismiss_sharing_guard();
        assert!(!core.app_status().sharing_guard.up);
        let core = Mutex::new(core);
        assert_eq!(may_show(&core), ShowCheck::Refused);
        core.lock().unwrap().guard_update(&[]);
        assert_eq!(may_show(&core), ShowCheck::Allowed);
        // A locked core says nothing about sharing apps, so it isn't a refusal.
        let held = core.lock().unwrap();
        assert_eq!(may_show(&core), ShowCheck::Busy);
        drop(held);
        assert_eq!(may_show(&core), ShowCheck::Allowed);
        // A poisoned core stays a no.
        std::thread::scope(|scope| {
            let poison = scope.spawn(|| {
                let _held = core.lock().unwrap();
                std::panic::resume_unwind(Box::new("fake panic"));
            });
            assert!(poison.join().is_err());
        });
        assert_eq!(may_show(&core), ShowCheck::Refused);
    }

    #[test]
    fn a_wallet_readout_keeps_its_ceilings_and_an_ended_one_is_not_revived() {
        // Timed: 20 seconds and three resets are 80 seconds at the very most.
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout_mode(&mut core, &state, ShowMode::Accessible);
        let deadline = state.snapshot(token).unwrap().deadline;
        let core = Mutex::new(core);
        for _ in 0..3 {
            now.fetch_add(19_999, Ordering::SeqCst);
            // Owner input keeps the one-minute Auto-lock of this fixture away.
            assert_eq!(
                authorize_callback(&core, &state, token, deadline, Some(71), true),
                Authorization::Allowed
            );
            assert_eq!(
                extend_callback(&core, &state, token, deadline, |_| ()),
                Authorization::Allowed
            );
        }
        now.fetch_add(19_999, Ordering::SeqCst);
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, Some(71), true),
            Authorization::Allowed
        );
        now.fetch_add(1, Ordering::SeqCst);
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, Some(71), true),
            Authorization::Denied
        );
        assert!(core.lock().unwrap().app_status().reveal.is_none());
        assert_eq!(
            authorize_callback(&core, &state, token, deadline, Some(71), true),
            Authorization::Denied
        );
        assert_eq!(
            extend_callback(&core, &state, token, deadline, |_| panic!("not extended")),
            Authorization::Denied
        );

        // Held: its own two minutes end it, whatever the panel's limit says.
        let (mut core, _temp, now) = fake_core_with_clock();
        let state = CoordinatorState::default();
        let token = fake_wallet_readout(&mut core, &state);
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        assert!(state.publish(token, 71, deadline, state.snapshot(token).unwrap().stage));
        for _ in 0..2 {
            now.fetch_add(59_999, Ordering::SeqCst);
            assert!(authorized_for_test(
                &mut core,
                &state,
                token,
                Some(71),
                true
            ));
        }
        now.fetch_add(2, Ordering::SeqCst);
        assert!(!authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            true
        ));
        assert!(core.app_status().reveal.is_none());
        assert!(!authorized_for_test(
            &mut core,
            &state,
            token,
            Some(71),
            true
        ));
    }

    #[test]
    fn cancelling_the_kit_still_rolls_back_and_stays_cancelled() {
        let (mut core, _temp) = fake_core();
        let _fake_key = setup_fake_vault(&mut core);
        let operation = core.begin_native(NativeAction::ConfirmRecoveryKit).unwrap();
        let deadline = Instant::now() + OPERATION_TIMEOUT;
        let (state, token) = reporting_state(deadline);
        state.stop_reporting(token);
        let epoch = core.native_operation_epoch();
        assert!(state.publish(token, operation.id, deadline, Stage::RecoveryKit { epoch }));
        assert_eq!(
            prompt_failure(&state, token, PromptError::Cancelled).code,
            "cancelled"
        );
        let active = state.stop(token).unwrap();
        assert_eq!(check_state(&state, token).unwrap_err().code, "cancelled");
        cleanup_core(&mut core, &active, state.idle());
        assert_eq!(core.app_status().phase, "no_vault");
        assert!(core.native_pending_id().is_none());
    }

    #[test]
    fn kit_confirmation_is_derived_only_inside_rust() {
        let fake = "FAKE-FAKE-FAKE-FAKE-FAKE-ABCD-EF01";
        assert_eq!(last_groups(fake).unwrap().as_str(), "ABCDEF01");
        assert!(last_groups("fake").is_err());
        assert!(last_groups("FAKE-LONGER").is_err());
    }

    #[test]
    fn credential_reply_shape_is_bound_to_the_validated_action() {
        assert!(
            credentials(
                EntryKind::PasswordChange,
                Reply::Credential(Zeroizing::new("fake".into()))
            )
            .is_err()
        );
        assert!(credentials(EntryKind::Password, Reply::RecoveryKitConfirmed).is_err());
        assert!(matches!(
            credentials(
                EntryKind::Recovery,
                Reply::Credential(Zeroizing::new("fake-recovery".into()))
            ),
            Ok(NativeCredentials::RecoveryKey(_))
        ));
    }
}
