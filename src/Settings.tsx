// Settings: where the keys are stored, Backups (saving one, opening one, Safety Copies), Auto-lock,
// Touch ID, Change Master Password, help, the optional Agent Companion (ADR-0009) and the optional
// route for agents storing keys (ADR-0012), which shares nothing with the Companion.

import { useEffect, useId, useRef, useState, type ReactNode, type Ref } from "react";
import {
  agentRouteStart,
  agentRouteStatus,
  agentRouteStop,
  appStatus,
  changePassword,
  cancelNativeOperation,
  companionSetup,
  companionStart,
  companionStatus,
  companionStop,
  disableQuickUnlock,
  enableQuickUnlock,
  discardInspection,
  forgetBackupDestination,
  inspectBackup,
  inspectSafetyCopy,
  pickBackupDestination,
  pickBackupToOpen,
  replaceWithBackup,
  restoreSafetyCopy,
  revealVaultFolder,
  setAgentAccount,
  setAgentToolHash,
  setSettings,
  vaultLocation,
  writeBackup,
  type AgentRouteView,
  type AppStatus,
  type BackupWritten,
  type CloudProvider,
  type CompanionView,
  type SafetyCopy,
  type Settings as SettingsView,
  type VaultPreview,
} from "./api";
import { Choice, ErrorLine, Field, formatDate, Modal, noAssist, SafetyCopiesNote, seconds, useAction, useSafetyCopies } from "./components";
import { strings as s } from "./strings";

const MINUTES = [1, 2, 5, 10, 15, 30, 60].map((m) => [m, s.minutes(m)] as const);
const CLIPBOARD_SECONDS = [30, 60].map((n) => [n, s.clipboardSeconds(n)] as const);

export function Settings({ status, onOperationOutcome, afterKit, scrollTo, onScrolled }: {
  status: AppStatus;
  onOperationOutcome: (message: string) => void;
  afterKit: (notSaved: string, e?: unknown) => void;
  scrollTo: "backups" | null;
  onScrolled: () => void;
}) {
  const [chosen, setChosen] = useState(() => lockChoices(status.settings));
  const [saving, setSaving] = useState(false);
  const submitting = useRef(false);
  const latest = useRef(status);
  latest.current = status;
  useEffect(() => setChosen(lockChoices(status.settings)), [status.settings.idle_minutes, status.settings.lock_on_app_switch, status.settings.clipboard_clear_seconds, status.settings.count_activity_anywhere]);
  const safety = useSafetyCopies();
  const { copies } = safety;
  const [flow, setFlow] = useState<"password" | "open" | SafetyCopy | null>(null);
  const { error, run } = useAction();
  useEffect(safety.reload, []);
  const closeFlow = () => {
    setFlow(null);
    safety.reload();
  };
  const companion = useCompanion();
  // Where this computer's Vault is, as text to show and copy. Settled once the answer came or
  // failed: until then the section above Backups can still change height.
  const [folder, setFolder] = useState<string | null>(null);
  const [folderSettled, setFolderSettled] = useState(false);
  useEffect(() => void vaultLocation().then(setFolder, () => {}).finally(() => setFolderSettled(true)), []);

  // Not scrollIntoView: that also scrolls .app and could push warnings above the page out of sight.
  const page = useRef<HTMLElement>(null);
  const backups = useRef<HTMLElement>(null);
  useEffect(() => {
    if (scrollTo !== "backups" || !folderSettled || !page.current || !backups.current) return;
    page.current.scrollTop += backups.current.getBoundingClientRect().top - page.current.getBoundingClientRect().top;
    // The Companion's first answer can add a row above Backups, so scroll once more after it.
    if (companion.settled) onScrolled();
  }, [scrollTo, folderSettled, companion.settled]);

  const save = async (settings: typeof chosen, generation: number) => {
    if (submitting.current) return;
    submitting.current = true;
    setSaving(true);
    try {
      setChosen(lockChoices(await setSettings({ ...settings, session_generation: generation })));
    } catch (error) {
      const actual = await appStatus().catch(() => latest.current);
      setChosen(lockChoices(actual.settings));
      throw error;
    } finally {
      submitting.current = false;
      setSaving(false);
    }
  };
  const change = (next: Partial<typeof chosen>) => {
    if (submitting.current) return;
    run(() => save({ ...chosen, ...next }, status.session_generation));
  };
  const disabled = saving;

  return (
    <section ref={page} className="page stack">
      <WhereStored status={status} folder={folder} lockOnSwitch={chosen.lock_on_app_switch} clipboardSeconds={chosen.clipboard_clear_seconds} view={companion.view} />

      <BackUp status={status} sectionRef={backups}>
        <p>{s.openBackupText}</p>
        <button onClick={() => setFlow("open")}>{s.openBackup}</button>
        <h3>{s.safetyCopiesTitle}</h3>
        <p className="hint">{s.safetyCopiesText}</p>
        {copies && copies.length > 0 && (
          <ul className="rows">
            {copies.map((c) => (
              <li key={c.id} className="trash-row">
                <span>
                  {formatDate(c.created_at)} · {s.safetyKinds[c.kind]}
                </span>
                <button onClick={() => setFlow(c)}>{s.restore}</button>
              </li>
            ))}
          </ul>
        )}
        <SafetyCopiesNote {...safety} />
      </BackUp>

      <section className="section">
        <h2>{s.autoLockTitle}</h2>
        <div className="card stack">
          <Choice label={s.autoLockAfter} options={MINUTES} value={chosen.idle_minutes} disabled={disabled} onChange={(m) => change({ idle_minutes: m })} />
          <Switch
            label={s.lockOnSwitch}
            hint={s.lockOnSwitchHint}
            disabled={disabled}
            checked={chosen.lock_on_app_switch}
            onChange={(on) => change({ lock_on_app_switch: on })}
          />
          {/* Only where the computer can tell when it was last used: elsewhere the core keeps it off. */}
          {status.settings.count_activity_anywhere_supported && (
            <Switch
              label={s.countActivityAnywhere}
              hint={s.countActivityAnywhereHint(chosen.lock_on_app_switch)}
              disabled={disabled}
              checked={chosen.count_activity_anywhere}
              onChange={(on) => change({ count_activity_anywhere: on })}
            />
          )}
          <p className="hint">{status.lock_events_unavailable.length ? s.lockEventsDegraded : s.alwaysLocks(status.platform === "mac")}</p>
          <p className="hint">{s.autoLockMoreTime(!!status.settings.count_activity_anywhere_supported)}</p>
          <p className="hint">{s.autoLockReauthLimit}</p>
          <div className="setting">
            <Choice
              label={s.clipboardClearAfter}
              options={CLIPBOARD_SECONDS}
              disabled={disabled}
              value={chosen.clipboard_clear_seconds}
              onChange={(n) => change({ clipboard_clear_seconds: n })}
            />
            <p className="hint">{s.walletCopiesClearAfter30}</p>
          </div>
          <ErrorLine error={error} />
        </div>
      </section>

      {status.platform === "mac" && <QuickUnlock status={status} />}

      <section className="section">
        <h2>{s.changePasswordTitle}</h2>
        <div className="card stack">
          <p>{s.changePasswordText}</p>
          <button onClick={() => setFlow("password")}>{s.changePasswordTitle}</button>
        </div>
      </section>

      <details className="card">
        <summary>{s.privacyTitle}</summary>
        {s.privacyTips.map((t) => (
          <p key={t}>{t}</p>
        ))}
      </details>

      <details className="card">
        <summary>{s.limitsTitle}</summary>
        <ul>
          {s.limits.map((t) => (
            <li key={t}>{t}</li>
          ))}
        </ul>
        {s.limitsAdvice.map((t) => (
          <p key={t}>{t}</p>
        ))}
      </details>

      <AgentCompanion companion={companion} />

      <AgentKeys />

      {flow === "password" && <ChangePassword retryInMs={status.retry_in_ms} afterKit={afterKit} onClose={closeFlow} onOperationOutcome={onOperationOutcome} />}
      {flow === "open" && <ReplaceVault retryInMs={status.retry_in_ms} onClose={closeFlow} onOperationOutcome={onOperationOutcome} />}
      {typeof flow === "object" && flow && <ReplaceVault copy={flow} retryInMs={status.retry_in_ms} onClose={closeFlow} onOperationOutcome={onOperationOutcome} />}
    </section>
  );
}

const lockChoices = (settings: SettingsView) => ({
  idle_minutes: settings.idle_minutes,
  lock_on_app_switch: settings.lock_on_app_switch,
  clipboard_clear_seconds: settings.clipboard_clear_seconds,
  count_activity_anywhere: settings.count_activity_anywhere,
});

/** A setting drawn as a row with a switch, and one line under it saying what it does. */
function Switch(props: { label: string; hint: string; checked: boolean; disabled?: boolean; onChange: (on: boolean) => void }) {
  const hint = useId();
  return (
    <div className="setting">
      <label className="switch-row">
        {props.label}
        {/* `switch`: WebKit draws a Mac switch; elsewhere it stays a checkbox. */}
        <input
          type="checkbox"
          {...{ switch: "" }}
          disabled={props.disabled}
          checked={props.checked}
          aria-describedby={hint}
          onChange={(e) => props.onChange(e.target.checked)}
        />
      </label>
      <p className="hint" id={hint}>
        {props.hint}
      </p>
    </div>
  );
}

/** One row of Where your keys are stored. Its state is a word, never color alone. */
function StorageRow({ label, word, text, children }: { label: string; word: string; text: string; children?: ReactNode }) {
  return (
    <div>
      <dt>{label}</dt>
      <dd className="setting">
        <p>
          <strong className="status-word">{word}.</strong> {text}
        </p>
        {children}
      </dd>
    </div>
  );
}

/** Where the helper keeps the Agent Key. A fixed string, never helper output (ADR-0009). */
const AgentKeyPath = ({ confirmed }: { confirmed: boolean }) => (
  <>
    <p>{confirmed ? s.storage.keptHere : s.storage.keptHereIfSetUp}</p>
    <p className="path">{s.storage.agentPath}</p>
    {confirmed && <p>{s.storage.agentPathNote}</p>}
  </>
);

/** Where the owner revokes the Agent Key: a fixed address to copy. The WebView opens no address. */
const AgentKeyRevoke = () => (
  <>
    <p>{s.storage.agentRevoke}</p>
    <p className="path">{s.storage.agentRevokePage}</p>
  </>
);

/** Answers "where are my keys?": each key and file, where it is kept, and whether it is encrypted. */
function WhereStored({ status, folder, lockOnSwitch, clipboardSeconds, view }: {
  status: AppStatus;
  folder: string | null;
  lockOnSwitch: boolean;
  clipboardSeconds: number;
  view: CompanionView | null;
}) {
  const t = s.storage;
  const onMac = status.platform === "mac";
  const canReveal = status.platform !== "other";
  const quick = status.quick_unlock;
  const touch = quick.cleanup_failed
    ? { ...t.touchOff, word: t.status.off, text: s.quickUnlockCleanupFailed }
    : quick.enabled
      ? { ...t.touchOn, word: t.status.encrypted }
      : { ...t.touchOff, word: t.status.off };
  const { error, run } = useAction();
  return (
    <section className="section">
      <h2>{t.title}</h2>
      <div className="card stack">
        <p>{t.intro}</p>
        <dl className="storage-rows">
          <StorageRow {...t.notes(onMac)} word={t.status.encrypted} />
          <StorageRow {...t.master} word={t.status.notSaved} />
          <StorageRow {...t.recovery} word={t.status.notSaved} />
          {onMac && <StorageRow {...touch} />}
          {/* The flag can be false while the file is there, and a build without the helper can
              leave one behind, so neither row says there is no file. */}
          {onMac && (
            <StorageRow
              {...(view?.local.credential ? t.agentOn : t.agentUnknown)}
              word={view?.local.credential ? t.status.notEncrypted : t.status.optional}
            >
              <AgentKeyPath confirmed={!!view?.local.credential} />
              {view?.local.credential && <AgentKeyRevoke />}
            </StorageRow>
          )}
        </dl>
        <div className="setting">
          <h3>{t.folderTitle}</h3>
          {onMac ? <p>{t.folderMac}</p> : status.platform === "windows" && <p>{t.folderWindows}</p>}
          {folder && (
            <>
              {/* Its own element, so that a selection copies only the path. */}
              <p className="path">{folder}</p>
              <p>{t.folderCopy}</p>
              {onMac && <p>{t.folderLibrary}</p>}
            </>
          )}
        </div>
        {canReveal && <button onClick={() => run(revealVaultFolder)}>{onMac ? t.revealFinder : t.revealExplorer}</button>}
        <ErrorLine error={error} />
        {canReveal && lockOnSwitch && <p>{t.revealLocks(onMac)}</p>}
        <p>{t.folderLeave}</p>
        <details className="more-details">
          <summary>{t.moreTitle}</summary>
          <div className="stack">
            <dl className="storage-rows">
              <StorageRow {...t.safety} word={t.status.encrypted} />
              <StorageRow {...t.backups} word={t.status.encrypted} />
              <StorageRow {...t.settingsFile(onMac)} word={t.status.notEncrypted} />
              <StorageRow {...t.copied(clipboardSeconds)} word={t.status.notEncrypted} />
              <StorageRow {...t.openNotes} word={t.status.inMemory} />
            </dl>
            <div className="setting">
              <h3>{t.backUpTitle}</h3>
              <p>{t.backUpText}</p>
            </div>
            <div className="setting">
              <h3>{t.leavesTitle}</h3>
              <p>{t.leavesText}</p>
            </div>
          </div>
        </details>
      </div>
    </section>
  );
}

/** Save a Backup. `children`: the rest of the Backups section, opening one and Safety Copies. */
function BackUp({ status, sectionRef, children }: { status: AppStatus; sectionRef: Ref<HTMLElement>; children: ReactNode }) {
  const [cloud, setCloud] = useState<CloudProvider | null>(null);
  const [written, setWritten] = useState<BackupWritten | null>(null);
  const { busy, error, run } = useAction();
  const write = () => {
    setCloud(null);
    run(async () => setWritten(await writeBackup()));
  };
  const start = () => {
    setCloud(null);
    setWritten(null);
    run(async () => {
      const picked = await pickBackupDestination();
      if (picked.cloud) setCloud(picked.cloud);
      else setWritten(await writeBackup());
    });
  };
  const last = status.backup.last_backup_at;
  return (
    <section ref={sectionRef} className="section">
      <h2>{s.backupsTitle}</h2>
      <div className="card stack">
        <p>{s.backupsIntro}</p>
        <p>{last ? s.lastBackup(formatDate(last)) : s.neverBackedUp}</p>
        {last && <p className="hint">{s.lastBackupWhere}</p>}
        <button className="primary" disabled={busy} onClick={start}>
          {busy ? s.backingUp : s.backUpNow}
        </button>
        {written && (
          <p className="ok">
            {s.backupDone(written.note_count)} <span className="hint">{written.display_path}</span>
          </p>
        )}
        {written && !written.durable && <p className="hint">{s.backupEjectHint}</p>}
        <ErrorLine error={error} />
        <p className="hint">{s.exfatTip}</p>
        {children}
      </div>
      {cloud && (
        <Modal
          title={s.cloudTitle(cloud)}
          onClose={() => {
            // Backing out: Rust must not keep the declined folder as the next Backup's place.
            setCloud(null);
            forgetBackupDestination().catch(() => {});
          }}
        >
          <p>{s.cloudWarning(cloud)}</p>
          <div className="actions">
            <button onClick={write}>{s.saveAnyway}</button>
            <button className="primary" onClick={start}>
              {s.chooseAnother}
            </button>
          </div>
        </Modal>
      )}
    </section>
  );
}

/** The Agent Companion's status, for the two sections that show it: one poll, kept running while
 *  its section is folded. Settled once the first answer came or failed. */
function useCompanion() {
  const [view, setView] = useState<CompanionView | null>(null);
  const [settled, setSettled] = useState(false);
  const acting = useRef(false);
  useEffect(() => {
    let alive = true;
    const refresh = () => {
      if (acting.current) return;
      companionStatus()
        .then((next) => alive && !acting.current && setView(next), () => {})
        .finally(() => alive && setSettled(true));
    };
    refresh();
    const timer = window.setInterval(refresh, 5000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);
  return { view, setView, settled, acting };
}

/** Off says why when the app stopped it because the Vault locked. */
const companionStateName = (view: CompanionView) =>
  view.ended_by_lock ? s.companionEndedByLock : s.companionStates[view.state];

/** The optional Agent Companion (ADR-0009): readiness, the approved operation, Start/Stop.
 *  Everything shown comes from fixed fields; Rust never sends a key, token, path or helper text. */
function AgentCompanion({ companion }: { companion: ReturnType<typeof useCompanion> }) {
  const { view, setView, acting } = companion;
  const [setupDone, setSetupDone] = useState(false);
  const [open, setOpen] = useState(false);
  const { busy, error, run } = useAction();
  const act = (call: () => Promise<CompanionView>) => {
    setSetupDone(false);
    acting.current = true;
    run(async () => setView(await call())).finally(() => (acting.current = false));
  };
  // A failed second install can keep the flag true while it reports install_invalid, so the error
  // counts too. Only this reply sets it, never the poll. The chosen file's name is never shown.
  const setUp = () =>
    act(async () => {
      const next = await companionSetup();
      setSetupDone(next.state === "stopped" && !next.error && next.local.credential);
      return next;
    });
  const on = view?.state === "starting" || view?.state === "running" || view?.state === "stopping";
  // The state, On until and Stop must not be folded away, nor anything that went wrong.
  const mustShow = on || view?.state === "error" || !!view?.error || !!error;
  // When it can be folded again it stays open, so what the owner was reading doesn't vanish.
  useEffect(() => {
    if (mustShow) setOpen(true);
  }, [mustShow]);
  const until = (t: number) => new Date(t * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  const parts = Object.keys(s.companionLocal) as (keyof typeof s.companionLocal)[];

  let body;
  if (!view) body = <p className="hint">{s.companionChecking}</p>;
  else if (!view.platform_supported) body = <p className="hint">{s.companionUnsupported}</p>;
  else if (!view.helper_bundled) body = <p className="hint">{s.companionNotBundled}</p>;
  else
    body = (
      <>
        <p>{s.companionIntro}</p>
        <p className="warn">{s.companionCantFinish}</p>
        {/* When folded, the summary line says this. */}
        {mustShow && (
          <p role="status" aria-live="polite">
            <strong>{companionStateName(view)}</strong>
            {view.state === "running" && view.expires_at ? ` · ${s.companionRunningUntil(until(view.expires_at))}` : ""}
          </p>
        )}
        {view.error && <p className="warn">{s.companionErrors[view.error]}</p>}
        <div className="setting">
          <strong>{s.companionLocalTitle}</strong>
          <ul className="rows">
            {parts.map((part) => (
              <li key={part}>
                <span aria-hidden="true">{view.local[part] ? "✓" : "✗"} </span>
                {s.companionLocal[part]}
                <span className="sr-only">{view.local[part] ? " (ready)" : " (not ready)"}</span>
                {!view.local[part] && <p className="hint">{s.companionLocalFix[part]}</p>}
              </li>
            ))}
          </ul>
          <p className="hint">{view.local.ready ? s.companionReady : s.companionNotReady}</p>
        </div>
        <div className="setting">
          <strong>{s.companionChatgptTitle}</strong>
          <p className="hint">{s.companionChatgpt}</p>
        </div>
        <details>
          <summary>{s.companionOperationTitle}</summary>
          {s.companionOperation(view.operation).map((line) => (
            <p key={line}>{line}</p>
          ))}
        </details>
        <div className="setting">
          <p className="hint">{view.local.credential ? s.companionCustody : s.companionCustodyIfSetUp}</p>
          <AgentKeyPath confirmed={view.local.credential} />
          <AgentKeyRevoke />
        </div>
        <details>
          <summary>{s.companionSetupTitle}</summary>
          <ol>
            {s.companionSetupSteps.map((step) => (
              <li key={step}>{step}</li>
            ))}
          </ol>
        </details>
        {setupDone && <p className="warn">{s.companionSetupDone}</p>}
        {!on && <p className="hint">{s.companionStartLocks}</p>}
        <div className="actions">
          <button disabled={busy || on} onClick={setUp}>
            {s.companionSetup}
          </button>
          {on ? (
            <button disabled={busy || view.state === "stopping"} onClick={() => act(companionStop)}>
              {s.companionStop}
            </button>
          ) : (
            <button className="primary" disabled={busy || !view.local.ready} onClick={() => act(companionStart)}>
              {s.companionStart}
            </button>
          )}
        </div>
      </>
    );

  return (
    <section className="section">
      {mustShow ? (
        <>
          <h2>{s.companionTitle}</h2>
          <div className="card stack">
            {body}
            <ErrorLine error={error} />
          </div>
        </>
      ) : (
        <details className="card more-details" open={open} onToggle={(e) => setOpen(e.currentTarget.open)}>
          <summary>
            <span role="status">{s.companionSummary(view ? companionStateName(view) : s.checking)}</span>
          </summary>
          <div className="stack">
            {body}
            <ErrorLine error={error} />
          </div>
        </details>
      )}
    </section>
  );
}

/** Agents storing keys (ADR-0012): the agent's Mac user, the approved tool's code hash
 *  (ADR-0013), Start for at most an hour, Stop, and what is true now. Fixed fields from Rust
 *  only: a state, the time left, the account name and the hash. None of the Agent Companion's
 *  code or state. */
function AgentKeys() {
  const [view, setView] = useState<AgentRouteView | null>(null);
  // What the account box holds, once the owner has typed in it; null: as saved.
  const [typed, setTyped] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  // The same for the tool's code hash.
  const [typedHash, setTypedHash] = useState<string | null>(null);
  const [savedHash, setSavedHash] = useState(false);
  const [open, setOpen] = useState(false);
  const acting = useRef(false);
  const { busy, error, run } = useAction();
  useEffect(() => {
    let alive = true;
    const refresh = () => {
      if (acting.current) return;
      agentRouteStatus().then((next) => alive && !acting.current && setView(next), () => {});
    };
    refresh();
    const timer = window.setInterval(refresh, 5000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, []);
  const act = (call: () => Promise<AgentRouteView>) => {
    setSaved(false);
    setSavedHash(false);
    acting.current = true;
    return run(async () => setView(await call())).finally(() => (acting.current = false));
  };
  const account = typed ?? view?.account ?? "";
  const changedAccount = (account.trim() || null) !== (view?.account ?? null);
  const saveAccount = () =>
    act(() => setAgentAccount(account.trim() || null)).then((ok) => {
      if (!ok) return;
      setTyped(null);
      setSaved(true);
    });
  const hash = typedHash ?? view?.tool_hash ?? "";
  const changedHash = (hash.trim() || null) !== (view?.tool_hash ?? null);
  const saveHash = () =>
    act(() => setAgentToolHash(hash.trim() || null)).then((ok) => {
      if (!ok) return;
      setTypedHash(null);
      setSavedHash(true);
    });
  const on = view?.state === "running";
  // On, and anything that went wrong, is never folded away.
  const mustShow = on || !!error;
  useEffect(() => {
    if (mustShow) setOpen(true);
  }, [mustShow]);
  const state = view ? s.agentKeysStates[view.state] : s.agentKeysChecking;
  const left = on && view.expires_in_ms != null ? Math.max(1, Math.ceil(view.expires_in_ms / 60_000)) : null;

  const body = (
    <>
      <p>{s.agentKeysIntro}</p>
      <p className="hint">{s.agentKeysLeaves}</p>
      {/* When folded, the summary line says this. */}
      {mustShow && (
        <p role="status" aria-live="polite">
          <strong className="status-word">{state}</strong>
          {left !== null && ` · ${s.agentKeysTimeLeft(left)}`}
        </p>
      )}
      <form
        className="setting"
        onSubmit={(e) => {
          e.preventDefault();
          if (changedAccount && !busy) saveAccount();
        }}
      >
        <Field label={s.agentAccount} hint={s.agentAccountHint} check={saved && !changedAccount && <span className="ok" role="status">{s.agentAccountSaved}</span>}>
          <input
            value={account}
            maxLength={31}
            placeholder="enote-agent"
            disabled={!view}
            onChange={(e) => {
              setSaved(false);
              setTyped(e.target.value);
            }}
            {...noAssist}
          />
        </Field>
        <div className="actions">
          <button disabled={busy || !view || !changedAccount}>{s.agentAccountSave}</button>
        </div>
      </form>
      <form
        className="setting"
        onSubmit={(e) => {
          e.preventDefault();
          if (changedHash && !busy) saveHash();
        }}
      >
        <Field label={s.agentToolHash} hint={s.agentToolHashHint} check={savedHash && !changedHash && <span className="ok" role="status">{s.agentAccountSaved}</span>}>
          <input
            value={hash}
            maxLength={60}
            placeholder="CDHash=…"
            disabled={!view}
            onChange={(e) => {
              setSavedHash(false);
              setTypedHash(e.target.value);
            }}
            {...noAssist}
          />
        </Field>
        <div className="actions">
          <button disabled={busy || !view || !changedHash}>{s.agentAccountSave}</button>
        </div>
      </form>
      <p className="hint">{s.agentKeysSetup}</p>
      {!on && <p className="hint">{s.agentKeysStartLocks}</p>}
      <div className="actions">
        {on ? (
          <button disabled={busy} onClick={() => act(agentRouteStop)}>
            {s.agentKeysStop}
          </button>
        ) : (
          <button className="primary" disabled={busy || !view || changedAccount || changedHash} onClick={() => act(agentRouteStart)}>
            {s.agentKeysStart}
          </button>
        )}
      </div>
    </>
  );

  return (
    <section className="section">
      {mustShow ? (
        <>
          <h2>{s.agentKeysTitle}</h2>
          <div className="card stack">
            {body}
            <ErrorLine error={error} />
          </div>
        </>
      ) : (
        <details className="card more-details" open={open} onToggle={(e) => setOpen(e.currentTarget.open)}>
          <summary>
            <span role="status">{s.agentKeysSummary(state)}</span>
          </summary>
          <div className="stack">
            {body}
            <ErrorLine error={error} />
          </div>
        </details>
      )}
    </section>
  );
}

/** Open a Backup (first run or Settings) or restore a Safety Copy: pick, credential, preview, replace. */
export function ReplaceVault({ copy, retryInMs, onClose, onOperationOutcome }: {
  copy?: SafetyCopy;
  retryInMs?: number | null | undefined;
  onClose: () => void;
  onOperationOutcome: (message: string) => void;
}) {
  const wait = retryInMs ? seconds(retryInMs) : 0;
  const [picked, setPicked] = useState<string | null>(null);
  const [useKey, setUseKey] = useState(false);
  const [preview, setPreview] = useState<VaultPreview | null>(null);
  const { busy, error, run } = useAction();
  const [committing, setCommitting] = useState(false);
  // However this dialog goes away (Cancel, Escape, a lock, another screen), Rust drops the
  // decrypted copy it kept for "Replace". After a replace there is nothing left to drop.
  useEffect(() => () => {
    void cancelNativeOperation().catch(() => {});
    void discardInspection().catch(() => {});
  }, []);

  function inspect() {
    const kind = useKey ? "recovery" : "password";
    run(async () => setPreview(copy ? await inspectSafetyCopy(copy.id, kind) : await inspectBackup(kind)));
  }

  // This computer's Vault beside the copy: only when the core could compare the two (the Vault
  // here is Unlocked and the copy is a copy of it). Otherwise the copy is described by itself.
  const here =
    preview && preview.current_note_count != null && preview.current_changed_at != null
      ? { notes: preview.current_note_count, changedAt: preview.current_changed_at }
      : null;
  let body;
  if (preview)
    body = (
      <>
        {here ? (
          <table className="compare">
            <thead>
              <tr>
                <td />
                <th scope="col">{s.compareCopy(!!copy)}</th>
                <th scope="col">{s.compareHere}</th>
              </tr>
            </thead>
            <tbody>
              <tr>
                <th scope="row">{s.compareNotes}</th>
                <td>{preview.note_count}</td>
                <td>{here.notes}</td>
              </tr>
              <tr>
                <th scope="row">{s.compareChanged}</th>
                <td>{formatDate(preview.changed_at)}</td>
                <td>{formatDate(here.changedAt)}</td>
              </tr>
            </tbody>
          </table>
        ) : (
          <>
            <p>{s.previewNotes(preview.note_count)}</p>
            <p>{s.previewChanged(formatDate(preview.changed_at))}</p>
          </>
        )}
        {preview.older_than_current && <p className="warn">{s.olderWarning(!!copy)}</p>}
        {!!preview.only_here && <p className="warn">{s.onlyHereWarning(preview.only_here, !!copy)}</p>}
        {preview.replaces_vault && preview.older_than_current === null && <p className="warn">{s.cannotCompareWarning}</p>}
        {preview.replaces_vault && <p className="hint">{s.keptAsSafetyCopy}</p>}
        {preview.replaces_vault && <p className="warn">{s.restoreCredentialsWarning}</p>}
        <ErrorLine error={error} />
        <div className="actions">
          <button disabled={committing} onClick={onClose}>{s.cancel}</button>
          <button
            className="primary"
            disabled={busy || committing}
            onClick={() => {
              setCommitting(true);
              run(copy ? restoreSafetyCopy : replaceWithBackup).then((ok) => {
                setCommitting(false);
                onOperationOutcome(ok ? (copy ? s.safetyCopyRestoreCompleted : s.backupRestoreCompleted) : s.restoreFailed);
                if (ok) onClose();
              });
            }}
          >
            {preview.replaces_vault ? s.replaceVault : s.useThisVault}
          </button>
        </div>
      </>
    );
  else if (copy || picked)
    body = (
      <div className="stack">
        {picked && <p className="hint">{picked}</p>}
        <Choice
          label={s.unlockWith}
          options={[["password", s.masterPassword], ["key", s.recoveryKey]]}
          value={useKey ? "key" : "password"}
          onChange={(v) => setUseKey(v === "key")}
        />
        <p className="hint">{s.nativeCredentialHint}</p>
        <ErrorLine error={error} />
        {wait > 0 && <p className="hint">{s.tryAgainIn(wait)}</p>}
        <div className="actions">
          <button type="button" onClick={onClose}>
            {s.cancel}
          </button>
          <button className="primary" disabled={busy || wait > 0} onClick={inspect}>
            {busy ? s.checking : s.openIt}
          </button>
        </div>
      </div>
    );
  else
    body = (
      <>
        <p>{s.pickBackupText}</p>
        <ErrorLine error={error} />
        <div className="actions">
          <button onClick={onClose}>{s.cancel}</button>
          <button className="primary" onClick={() => run(async () => setPicked((await pickBackupToOpen()).display_path))}>
            {s.chooseFile}
          </button>
        </div>
      </>
    );

  return (
    <Modal title={copy ? s.safetyCopiesTitle : s.openBackupTitle} onClose={committing ? () => {} : onClose}>
      {body}
    </Modal>
  );
}

function ChangePassword({ retryInMs, afterKit, onClose, onOperationOutcome }: {
  retryInMs: number | null;
  afterKit: (notSaved: string, e?: unknown) => void;
  onClose: () => void;
  onOperationOutcome: (message: string) => void;
}) {
  const [rotate, setRotate] = useState(false);
  const { busy, error, run } = useAction();
  const wait = retryInMs ? seconds(retryInMs) : 0;
  const close = () => {
    if (busy) void cancelNativeOperation().catch(() => {});
    onClose();
  };
  return (
    <Modal title={s.changePasswordTitle} onClose={close}>
      <div className="stack">
        <Choice
          label={s.whyChange}
          options={[["new", s.justNew], ["rotate", s.someoneKnows]]}
          value={rotate ? "rotate" : "new"}
          disabled={busy}
          onChange={(v) => setRotate(v === "rotate")}
        />
        <p className="hint">{rotate ? s.rotateExplained : s.justNewExplained}</p>
        <p className="hint">{s.nativeCredentialHint}</p>
        <ErrorLine error={error} />
        {wait > 0 && <p className="hint">{s.tryAgainIn(wait)}</p>}
        <div className="actions">
          <button onClick={close}>{s.cancel}</button>
          <button className="primary" disabled={busy || wait > 0} onClick={() => run(async () => {
            try {
              await changePassword(rotate);
              afterKit(s.keyRotationCancelled);
            } catch (e) {
              // Only App knows whether the Recovery Key step was reached. A wrong current password
              // never reaches it, so that stays in the ErrorLine above.
              afterKit(s.keyRotationCancelled, e);
              throw e;
            }
            onOperationOutcome(rotate ? s.keyRotationCompleted : s.passwordChangeCompleted);
            onClose();
          })}>{busy ? s.saving : rotate ? s.changeAndRotate : s.changePasswordTitle}</button>
        </div>
      </div>
    </Modal>
  );
}

function QuickUnlock({ status }: { status: AppStatus }) {
  const { busy, error, run } = useAction();
  const quick = status.quick_unlock;
  return (
    <section className="section">
      <h2>{s.quickUnlockTitle}</h2>
      <div className="card stack">
        {/* The button changing is not enough to say it worked. Worked out from the status, so it
            stays true when something else turns Touch ID off. */}
        <p role="status">
          <strong className="status-word">{quick.enabled && !quick.cleanup_failed ? s.touchIdIsOn : s.touchIdIsOff}</strong>
        </p>
        <p>{s.quickUnlockText}</p>
        <p className="hint">{s.quickUnlockWalletLimit}</p>
        <p className="hint">{s.quickUnlockTurnsOff}</p>
        {quick.reason && <p className="hint">{quick.reason}</p>}
        {quick.cleanup_failed && <p className="hint">{s.quickUnlockCleanupFailed}</p>}
        {quick.file_changed && <p className="warn">{s.quickUnlockFileChanged}</p>}
        <button disabled={busy || (!quick.cleanup_failed && !quick.enabled && !quick.available)} onClick={() => run(quick.cleanup_failed || quick.enabled ? disableQuickUnlock : enableQuickUnlock)}>
          {busy ? s.checking : quick.cleanup_failed ? s.retryTouchIdCleanup : quick.enabled ? s.disableTouchId : s.enableTouchId}
        </button>
        <ErrorLine error={error} />
      </div>
    </section>
  );
}
