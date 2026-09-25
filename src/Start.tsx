// Screens outside the Unlocked Vault: Welcome, Create Master Password, Recovery Kit, lock screen,
// and setting a new Master Password after a Recovery Key unlock.

import { useState, type FormEvent } from "react";
import { confirmRecoveryKit, createVault, lock, setNewPassword, unlock, type AppStatus } from "./api";
import { AppIcon, ErrorLine, Field, NewPassword, SecretInput, seconds, useAction } from "./components";
import { Icon, type IconName } from "./icons";
import { ReplaceVault } from "./Settings";
import { strings as s } from "./strings";

export type Kit = { key: string; rotation: boolean };

// One per s.welcomeCards entry, in order.
const WELCOME_ICONS: IconName[] = ["computer", "key", "eyeSlash", "shield"];

const LockLink = ({ label = s.lock }: { label?: string }) => (
  <button className="link" onClick={() => lock().catch(() => {})}>
    {label}
  </button>
);

export function NoVault({ onKit }: { onKit: (recoveryKey: string) => void }) {
  const [step, setStep] = useState<"welcome" | "create" | "open">("welcome");
  if (step === "create")
    return (
      <div className="center narrow">
        <h1>{s.createTitle}</h1>
        <p>{s.createIntro}</p>
        <NewPassword
          allowShow
          submitLabel={s.createVault}
          busyLabel={s.settingUp}
          onSubmit={async (password) => onKit((await createVault(password)).recovery_key)}
        />
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
      <button className="primary big" onClick={() => setStep("create")}>
        {s.createMyVault}
      </button>
      <button className="link centered" onClick={() => setStep("open")}>
        {s.haveVault}
      </button>
      {step === "open" && <ReplaceVault onClose={() => setStep("welcome")} />}
    </div>
  );
}

export function RecoveryKit({ kit }: { kit: Kit }) {
  const [typed, setTyped] = useState("");
  const [stored, setStored] = useState(false);
  const { busy, error, run } = useAction();
  function submit(e: FormEvent) {
    e.preventDefault();
    const t = typed;
    setTyped("");
    run(() => confirmRecoveryKit(t));
  }
  return (
    <div className="center narrow">
      <section className="kit">
        <h1>{s.kitTitle}</h1>
        <p>{s.kitIntro}</p>
        <p className="recovery-key">
          {kit.key.split("-").map((group, i) => (
            <span key={i}>{group}</span>
          ))}
        </p>
        <p className="warn">{s.kitPdfWarning}</p>
      </section>
      {kit.rotation && <p className="warn">{s.oldCopiesAfterRotation}</p>}
      <form onSubmit={submit} className="stack">
        <Field label={s.retypeLastGroups}>
          <SecretInput value={typed} onChange={(e) => setTyped(e.target.value)} />
        </Field>
        <label className="check">
          <input type="checkbox" checked={stored} onChange={(e) => setStored(e.target.checked)} />
          {s.storedPhysically}
        </label>
        <ErrorLine error={error} />
        <button className="primary big" disabled={busy || !typed || !stored}>
          {s.continue}
        </button>
      </form>
      {kit.rotation && <LockLink label={s.lockAndCancel} />}
    </div>
  );
}

/** The Recovery Key is shown once; after a reload of this screen it's gone. */
export const KitLost = () => (
  <div className="center narrow">
    <h1>{s.kitTitle}</h1>
    <p>{s.kitLost}</p>
    <LockLink label={s.lockAndCancel} />
  </div>
);

/** No Note titles or counts here. */
export function Locked({ status }: { status: AppStatus }) {
  const [useKey, setUseKey] = useState(false);
  const [secret, setSecret] = useState("");
  const { busy, error, run } = useAction();
  const wait = status.retry_in_ms ? seconds(status.retry_in_ms) : 0;
  function submit(e: FormEvent) {
    e.preventDefault();
    const v = secret;
    setSecret("");
    run(() => unlock(useKey ? { recovery_key: v } : { master_password: v }));
  }
  return (
    <div className="center tight">
      <AppIcon large />
      <h1>{s.appName}</h1>
      <form onSubmit={submit} className="stack">
        <Field label={useKey ? s.recoveryKey : s.masterPassword} hint={useKey ? s.recoveryKeyHint : undefined}>
          <SecretInput key={String(useKey)} value={secret} onChange={(e) => setSecret(e.target.value)} autoFocus />
        </Field>
        <ErrorLine error={error} />
        {wait > 0 && <p className="hint">{s.tryAgainIn(wait)}</p>}
        <button className="primary big" disabled={busy || !secret || wait > 0}>
          {busy ? s.unlocking : s.unlock}
        </button>
      </form>
      <button
        className={status.show_recovery_hint && !useKey ? "link emphasis" : "link"}
        onClick={() => {
          setUseKey(!useKey);
          setSecret("");
        }}
      >
        {useKey ? s.useMasterPassword : s.forgotUseKit}
      </button>
    </div>
  );
}

export const NewPasswordScreen = () => (
  <div className="center narrow">
    <h1>{s.setNewPasswordTitle}</h1>
    <p>{s.setNewPasswordIntro}</p>
    <NewPassword submitLabel={s.saveNewPassword} busyLabel={s.saving} onSubmit={setNewPassword} />
    <LockLink />
  </div>
);
