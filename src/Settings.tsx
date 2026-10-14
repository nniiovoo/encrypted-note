// Settings: Auto-lock, Backups, opening a Backup or Safety Copy, Change Master Password, help.

import { useEffect, useId, useState, type FormEvent } from "react";
import {
  changePassword,
  discardInspection,
  forgetBackupDestination,
  inspectBackup,
  inspectSafetyCopy,
  listSafetyCopies,
  pickBackupDestination,
  pickBackupToOpen,
  replaceWithBackup,
  restoreSafetyCopy,
  setSettings,
  vaultLocation,
  writeBackup,
  type AppStatus,
  type BackupWritten,
  type CloudProvider,
  type SafetyCopy,
  type VaultPreview,
} from "./api";
import { Choice, ErrorLine, Field, formatDate, Modal, NewPassword, SecretInput, useAction } from "./components";
import { strings as s } from "./strings";

const MINUTES = [1, 2, 5, 10, 15, 30, 60].map((m) => [m, s.minutes(m)] as const);
const CLIPBOARD_SECONDS = [30, 60].map((n) => [n, s.clipboardSeconds(n)] as const);

export function Settings({ status, onKit, onOperationOutcome }: { status: AppStatus; onKit: (recoveryKey: string) => void; onOperationOutcome: (message: string) => void }) {
  // Everything setSettings takes: each change is sent together with the rest.
  const [chosen, setChosen] = useState({
    idle_minutes: status.settings.idle_minutes,
    lock_on_app_switch: status.settings.lock_on_app_switch,
    clipboard_clear_seconds: status.settings.clipboard_clear_seconds ?? 30,
    count_activity_anywhere: status.settings.count_activity_anywhere ?? false,
  });
  const [copies, setCopies] = useState<SafetyCopy[]>([]);
  const [flow, setFlow] = useState<"password" | "open" | SafetyCopy | null>(null);
  const { error, run } = useAction();
  const loadCopies = () => void listSafetyCopies().then(setCopies, () => {});
  useEffect(loadCopies, []);
  const closeFlow = () => {
    setFlow(null);
    loadCopies();
  };

  const change = (next: Partial<typeof chosen>) => {
    const merged = { ...chosen, ...next };
    setChosen(merged);
    run(() => setSettings(merged));
  };

  return (
    <section className="page stack">
      <section className="section">
        <h2>{s.autoLockTitle}</h2>
        <div className="card stack">
          <Choice label={s.autoLockAfter} options={MINUTES} value={chosen.idle_minutes} onChange={(m) => change({ idle_minutes: m })} />
          <Switch
            label={s.lockOnSwitch}
            hint={s.lockOnSwitchHint}
            checked={chosen.lock_on_app_switch}
            onChange={(on) => change({ lock_on_app_switch: on })}
          />
          {/* Only where the computer can tell when it was last used: elsewhere the core keeps it off. */}
          {status.settings.count_activity_anywhere_supported && (
            <Switch
              label={s.countActivityAnywhere}
              hint={s.countActivityAnywhereHint(chosen.lock_on_app_switch)}
              checked={chosen.count_activity_anywhere}
              onChange={(on) => change({ count_activity_anywhere: on })}
            />
          )}
          <p className="hint">{s.alwaysLocks}</p>
          <div className="setting">
            <Choice
              label={s.clipboardClearAfter}
              options={CLIPBOARD_SECONDS}
              value={chosen.clipboard_clear_seconds}
              onChange={(n) => change({ clipboard_clear_seconds: n })}
            />
            <p className="hint">{s.walletCopiesClearAfter30}</p>
          </div>
          <ErrorLine error={error} />
        </div>
      </section>

      <BackUp status={status} />

      <section className="section">
        <h2>{s.openBackupTitle}</h2>
        <div className="card stack">
          <p>{s.openBackupText}</p>
          <button onClick={() => setFlow("open")}>{s.openBackup}</button>
        </div>
      </section>

      <section className="section">
        <h2>{s.safetyCopiesTitle}</h2>
        <div className="card stack">
          <p className="hint">{s.safetyCopiesText}</p>
          {copies.length > 0 && (
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
          {copies.length === 0 && <p className="hint">{s.noSafetyCopies}</p>}
        </div>
      </section>

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

      {flow === "password" && <ChangePassword onKit={onKit} onClose={closeFlow} onOperationOutcome={onOperationOutcome} />}
      {flow === "open" && <ReplaceVault onClose={closeFlow} onOperationOutcome={onOperationOutcome} />}
      {typeof flow === "object" && flow && <ReplaceVault copy={flow} onClose={closeFlow} onOperationOutcome={onOperationOutcome} />}
    </section>
  );
}

/** A setting drawn as a row with a switch, and one line under it saying what it does. */
function Switch(props: { label: string; hint: string; checked: boolean; onChange: (on: boolean) => void }) {
  const hint = useId();
  return (
    <div className="setting">
      <label className="switch-row">
        {props.label}
        {/* `switch`: WebKit draws a Mac switch; elsewhere it stays a checkbox. */}
        <input
          type="checkbox"
          {...{ switch: "" }}
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

function BackUp({ status }: { status: AppStatus }) {
  const [cloud, setCloud] = useState<CloudProvider | null>(null);
  const [written, setWritten] = useState<BackupWritten | null>(null);
  // Where this computer's Vault is, to read: text only, nothing opens it.
  const [folder, setFolder] = useState<string | null>(null);
  useEffect(() => void vaultLocation().then(setFolder, () => {}), []);
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
    <section className="section">
      <h2>{s.backupsTitle}</h2>
      <div className="card stack">
        <p>{last ? s.lastBackup(formatDate(last)) : s.neverBackedUp}</p>
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
        {folder && <p className="hint path">{s.vaultFolder(folder)}</p>}
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

/** Open a Backup (first run or Settings) or restore a Safety Copy: pick, credential, preview, replace. */
export function ReplaceVault({ copy, onClose, onOperationOutcome }: { copy?: SafetyCopy; onClose: () => void; onOperationOutcome: (message: string) => void }) {
  const [picked, setPicked] = useState<string | null>(null);
  const [useKey, setUseKey] = useState(false);
  const [secret, setSecret] = useState("");
  const [preview, setPreview] = useState<VaultPreview | null>(null);
  const { busy, error, run } = useAction();
  const [committing, setCommitting] = useState(false);
  // However this dialog goes away (Cancel, Escape, a lock, another screen), Rust drops the
  // decrypted copy it kept for "Replace". After a replace there is nothing left to drop.
  useEffect(() => () => void discardInspection().catch(() => {}), []);

  function inspect(e: FormEvent) {
    e.preventDefault();
    const credential = useKey ? { recovery_key: secret } : { master_password: secret };
    setSecret("");
    run(async () => setPreview(copy ? await inspectSafetyCopy(copy.id, credential) : await inspectBackup(credential)));
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
      <form onSubmit={inspect} className="stack">
        {picked && <p className="hint">{picked}</p>}
        <Choice
          label={s.unlockWith}
          options={[["password", s.masterPassword], ["key", s.recoveryKey]]}
          value={useKey ? "key" : "password"}
          onChange={(v) => setUseKey(v === "key")}
        />
        <Field label={useKey ? s.recoveryKey : s.masterPassword}>
          <SecretInput value={secret} onChange={(e) => setSecret(e.target.value)} autoFocus />
        </Field>
        <ErrorLine error={error} />
        <div className="actions">
          <button type="button" onClick={onClose}>
            {s.cancel}
          </button>
          <button className="primary" disabled={busy || !secret}>
            {busy ? s.checking : s.openIt}
          </button>
        </div>
      </form>
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

function ChangePassword({ onKit, onClose, onOperationOutcome }: { onKit: (recoveryKey: string) => void; onClose: () => void; onOperationOutcome: (message: string) => void }) {
  const [rotate, setRotate] = useState(false);
  const [current, setCurrent] = useState("");
  const [committing, setCommitting] = useState(false);

  async function submit(password: string) {
    const c = current;
    setCurrent("");
    setCommitting(true);
    try {
      const { recovery_key } = await changePassword(c, password, rotate);
      // With Key Rotation nothing is saved until the new Recovery Kit is confirmed, so nothing
      // is announced yet: the app says how it ended when that step is left.
      if (recovery_key) onKit(recovery_key);
      else onOperationOutcome(s.passwordChangeCompleted);
      onClose();
    } catch (error) {
      onOperationOutcome(s.passwordChangeFailed);
      throw error;
    } finally {
      setCommitting(false);
    }
  }

  return (
    <Modal title={s.changePasswordTitle} onClose={committing ? () => {} : onClose}>
      {
        <NewPassword
          submitLabel={rotate ? s.changeAndRotate : s.changePasswordTitle}
          busyLabel={s.saving}
          onSubmit={submit}
          ready={!!current}
        >
          <Choice
            label={s.whyChange}
            options={[["new", s.justNew], ["rotate", s.someoneKnows]]}
            value={rotate ? "rotate" : "new"}
            onChange={(v) => setRotate(v === "rotate")}
          />
          <p className="hint">{rotate ? s.rotateExplained : s.justNewExplained}</p>
          <Field label={s.currentPassword}>
            <SecretInput value={current} onChange={(e) => setCurrent(e.target.value)} autoFocus />
          </Field>
        </NewPassword>
      }
    </Modal>
  );
}
