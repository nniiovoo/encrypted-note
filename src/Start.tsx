// Screens outside the Unlocked Vault: Welcome, Create Master Password, Recovery Kit, lock screen,
// and setting a new Master Password after a Recovery Key unlock.

import { useEffect, useState } from "react";
import { createVault, lock, quickUnlock, setNewPassword, unlock, type AppStatus, type SafetyCopy } from "./api";
import { AppIcon, ErrorLine, formatDate, Modal, SafetyCopiesNote, seconds, useAction, useSafetyCopies } from "./components";
import { Icon, type IconName } from "./icons";
import { ReplaceVault } from "./Settings";
import { strings as s } from "./strings";

// One per s.welcomeCards entry, in order.
const WELCOME_ICONS: IconName[] = ["computer", "key", "eyeSlash", "shield"];

const LockLink = ({ label = s.lock }: { label?: string }) => (
  <button className="link" onClick={() => lock().catch(() => {})}>
    {label}
  </button>
);

export function NoVault({ onOperationOutcome }: { onOperationOutcome: (message: string) => void }) {
  const { busy, error, run } = useAction();
  const [step, setStep] = useState<"welcome" | "create" | "open">("welcome");
  // No Vault file, but Safety Copies: this is not a first run. Say so, before the owner
  // concludes their Notes are gone and creates a new Vault over them. Until the first list
  // arrives, say it is still looking: that is not the same as none.
  const safety = useSafetyCopies();
  const copies = safety.copies ?? [];
  const looking = safety.copies === null && !safety.error;
  const [copy, setCopy] = useState<SafetyCopy | null>(null);
  useEffect(safety.reload, []);
  if (step === "create")
    return (
      <div className="center narrow">
        <h1>{s.createTitle}</h1>
        <p>{s.createIntro}</p>
        <p className="hint">{s.nativeSetupHint}</p>
        <ErrorLine error={error} />
        <button className="primary big" disabled={busy} onClick={() => run(createVault)}>{busy ? s.settingUp : s.createVault}</button>
        <button className="link" onClick={() => setStep("welcome")}>
          {s.back}
        </button>
      </div>
    );
  return (
    <div className="center narrow">
      <header className="hero">
        <AppIcon large />
        <h1>{s.appName}</h1>
        <p className="lead">{s.welcomeLead}</p>
      </header>
      <div className="features">
        {s.welcomeCards.map(([title, text], i) => (
          <section className="feature" key={title}>
            <Icon name={WELCOME_ICONS[i]!} />
            <h2>{title}</h2>
            <p>{text}</p>
          </section>
        ))}
      </div>
      {looking && <p className="hint">{s.lookingForSafetyCopies}</p>}
      {copies.length > 0 && (
        <section className="card stack">
          <h2>{s.vaultWasHereTitle}</h2>
          <p>{s.vaultWasHereText}</p>
          <ul className="rows">
            {copies.map((item) => (
              <li key={item.id} className="trash-row">
                <span>{formatDate(item.created_at)}</span>
                <button onClick={() => setCopy(item)}>{s.restore}</button>
              </li>
            ))}
          </ul>
        </section>
      )}
      {safety.error && copies.length === 0 && (
        <section className="card stack">
          <h2>{s.safetyCopiesUncheckedTitle}</h2>
          <p>{s.safetyCopiesUncheckedText}</p>
          <ErrorLine error={safety.error} />
          <button onClick={safety.reload}>{s.tryAgain}</button>
        </section>
      )}
      <button className="primary big" onClick={() => setStep("create")}>
        {s.createMyVault}
      </button>
      <button className="link centered" onClick={() => setStep("open")}>
        {s.haveVault}
      </button>
      {step === "open" && <ReplaceVault onClose={() => setStep("welcome")} onOperationOutcome={onOperationOutcome} />}
      {copy && <ReplaceVault copy={copy} onClose={() => setCopy(null)} onOperationOutcome={onOperationOutcome} />}
    </div>
  );
}

/** The one-time Recovery Key never enters the page. */
export const NativeRecoveryKit = () => (
  <div className="center narrow">
    <h1>{s.kitTitle}</h1>
    <p>{s.nativeKitHint}</p>
    <LockLink label={s.lockAndCancel} />
  </div>
);

/** No Note titles or counts here. */
export function Locked({ status, onOperationOutcome }: { status: AppStatus; onOperationOutcome: (message: string) => void }) {
  const [useKey, setUseKey] = useState(false);
  const [restore, setRestore] = useState<"choose" | "backup" | null>(null);
  const safety = useSafetyCopies();
  const copies = safety.copies ?? [];
  const [copy, setCopy] = useState<SafetyCopy | null>(null);
  const { busy, error, run } = useAction();
  const wait = status.retry_in_ms ? seconds(status.retry_in_ms) : 0;
  return (
    <div className="center tight">
      <AppIcon large />
      <h1>{s.appName}</h1>
      <div className="stack">
        <p className="hint">{s.nativeCredentialHint}</p>
        <ErrorLine error={error} />
        {wait > 0 && <p className="hint">{s.tryAgainIn(wait)}</p>}
        <button className="primary big" disabled={busy || wait > 0} onClick={() => run(() => unlock(useKey ? "recovery" : "password"))}>
          {busy ? s.unlocking : useKey ? s.unlockWithRecoveryKey : s.unlockWithPassword}
        </button>
        {status.quick_unlock.enabled && <button disabled={busy || !status.quick_unlock.can_unlock || wait > 0} onClick={() => run(quickUnlock)}>{s.unlockWithTouchId}</button>}
        {status.quick_unlock.enabled && !status.quick_unlock.can_unlock && status.quick_unlock.reason && <p className="hint">{status.quick_unlock.reason}</p>}
      </div>
      <button
        className={status.show_recovery_hint && !useKey ? "link emphasis" : "link"}
        onClick={() => {
          setUseKey(!useKey);
        }}
      >
        {useKey ? s.useMasterPassword : s.forgotUseKit}
      </button>
      <button
        className="link"
        onClick={() => {
          setRestore("choose");
          safety.reload();
        }}
      >
        {s.restoreVault}
      </button>
      {restore === "choose" && (
        <Modal title={s.restoreVault} onClose={() => setRestore(null)}>
          <p>{s.restoreVaultText}</p>
          <button className="primary" onClick={() => setRestore("backup")}>{s.openBackup}</button>
          {copies.length > 0 && (
            <ul className="rows">
              {copies.map((item) => (
                <li key={item.id} className="trash-row">
                  <span>{formatDate(item.created_at)}</span>
                  <button onClick={() => { setRestore(null); setCopy(item); }}>{s.restore}</button>
                </li>
              ))}
            </ul>
          )}
          <SafetyCopiesNote {...safety} />
        </Modal>
      )}
      {restore === "backup" && <ReplaceVault onClose={() => setRestore(null)} onOperationOutcome={onOperationOutcome} />}
      {copy && <ReplaceVault copy={copy} onClose={() => setCopy(null)} onOperationOutcome={onOperationOutcome} />}
    </div>
  );
}

export function NewPasswordScreen() {
  const { busy, error, run } = useAction();
  return (
    <div className="center narrow">
      <h1>{s.setNewPasswordTitle}</h1>
      <p>{s.setNewPasswordIntro}</p>
      <p className="hint">{s.nativeCredentialHint}</p>
      <ErrorLine error={error} />
      <button className="primary big" disabled={busy} onClick={() => run(setNewPassword)}>{busy ? s.saving : s.saveNewPassword}</button>
      <LockLink />
    </div>
  );
}
