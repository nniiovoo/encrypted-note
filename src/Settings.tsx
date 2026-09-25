// Settings: Auto-lock, Backups, opening a Backup or Safety Copy, Change Master Password, help.

import { useEffect, useState, type FormEvent } from "react";
import {
  changePassword,
  inspectBackup,
  inspectSafetyCopy,
  listSafetyCopies,
  pickBackupDestination,
  pickBackupToOpen,
  replaceWithBackup,
  restoreSafetyCopy,
  setSettings,
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

export function Settings({ status, onKit }: { status: AppStatus; onKit: (recoveryKey: string) => void }) {
  const [lockRule, setLockRule] = useState({
    idle_minutes: status.settings.idle_minutes,
    lock_on_app_switch: status.settings.lock_on_app_switch,
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

  const change = (next: Partial<typeof lockRule>) => {
    const merged = { ...lockRule, ...next };
    setLockRule(merged);
    run(() => setSettings(merged));
  };

  return (
    <section className="page stack">
      <section className="section">
        <h2>{s.autoLockTitle}</h2>
        <div className="card stack">
          <Choice label={s.autoLockAfter} options={MINUTES} value={lockRule.idle_minutes} onChange={(m) => change({ idle_minutes: m })} />
          <label className="switch-row">
            {s.lockOnSwitch}
            {/* `switch`: WebKit draws a Mac switch; elsewhere it stays a checkbox. */}
            <input
              type="checkbox"
              {...{ switch: "" }}
              checked={lockRule.lock_on_app_switch}
              onChange={(e) => change({ lock_on_app_switch: e.target.checked })}
            />
          </label>
          <p className="hint">{s.alwaysLocks}</p>
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

      {flow === "password" && <ChangePassword onKit={onKit} onClose={closeFlow} />}
      {flow === "open" && <ReplaceVault onClose={closeFlow} />}
      {typeof flow === "object" && flow && <ReplaceVault copy={flow} onClose={closeFlow} />}
    </section>
  );
}

function BackUp({ status }: { status: AppStatus }) {
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
        <ErrorLine error={error} />
        <p className="hint">{s.exfatTip}</p>
      </div>
      {cloud && (
        <Modal title={s.cloudTitle(cloud)} onClose={() => setCloud(null)}>
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
export function ReplaceVault({ copy, onClose }: { copy?: SafetyCopy; onClose: () => void }) {
  const [picked, setPicked] = useState<string | null>(null);
  const [useKey, setUseKey] = useState(false);
  const [secret, setSecret] = useState("");
  const [preview, setPreview] = useState<VaultPreview | null>(null);
  const { busy, error, run } = useAction();

  function inspect(e: FormEvent) {
    e.preventDefault();
    const credential = useKey ? { recovery_key: secret } : { master_password: secret };
    setSecret("");
    run(async () => setPreview(copy ? await inspectSafetyCopy(copy.id, credential) : await inspectBackup(credential)));
  }

  let body;
  if (preview)
    body = (
      <>
        <p>{s.previewNotes(preview.note_count)}</p>
        <p>{s.previewChanged(formatDate(preview.changed_at))}</p>
        {preview.older_than_current && <p className="warn">{s.olderWarning(!!copy)}</p>}
        {preview.older_than_current !== null && <p className="hint">{s.keptAsSafetyCopy}</p>}
        <ErrorLine error={error} />
        <div className="actions">
          <button onClick={onClose}>{s.cancel}</button>
          <button
            className="primary"
            disabled={busy}
            onClick={() => run(copy ? restoreSafetyCopy : replaceWithBackup).then((ok) => ok && onClose())}
          >
            {preview.older_than_current === null ? s.useThisVault : s.replaceVault}
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
    <Modal title={copy ? s.safetyCopiesTitle : s.openBackupTitle} onClose={onClose}>
      {body}
    </Modal>
  );
}

function ChangePassword({ onKit, onClose }: { onKit: (recoveryKey: string) => void; onClose: () => void }) {
  const [rotate, setRotate] = useState(false);
  const [current, setCurrent] = useState("");
  const [done, setDone] = useState(false);

  async function submit(password: string) {
    const c = current;
    setCurrent("");
    const { recovery_key } = await changePassword(c, password, rotate);
    if (recovery_key) onKit(recovery_key);
    else setDone(true);
  }

  return (
    <Modal title={s.changePasswordTitle} onClose={onClose}>
      {done ? (
        <div className="stack">
          <p className="ok">{s.passwordChanged}</p>
          <p>{s.oldCopiesAfterChange}</p>
          <button className="primary" onClick={onClose}>
            {s.done}
          </button>
        </div>
      ) : (
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
      )}
    </Modal>
  );
}
