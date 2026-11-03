// Screens outside the Unlocked Vault: Welcome, Create Master Password, Recovery Kit, lock screen,
// and setting a new Master Password after a Recovery Key unlock.

import { useEffect, useRef, useState } from "react";
import { createVault, lock, quickUnlock, setNewPassword, unlock, vaultLocation, type AppStatus, type SafetyCopy } from "./api";
import { AppIcon, Confirm, ErrorLine, formatDate, mac, Modal, SafetyCopiesNote, seconds, useAction, useSafetyCopies } from "./components";
import { Icon, type IconName } from "./icons";
import { ReplaceVault } from "./Settings";
import { strings as s } from "./strings";

// One per s.welcomeCards entry, in order.
const WELCOME_ICONS: IconName[] = ["computer", "key", "eyeSlash", "shield"];

/** Where first run is. The open dialog is not one: it is never brought back. */
export type SetupStep = "welcome" | "create";

const LockLink = ({ label = s.lock }: { label?: string }) => (
  <button className="link" onClick={() => lock().catch(() => {})}>
    {label}
  </button>
);

/** Where the Vault is, for someone who can't open it and needs the file. Text only. */
function VaultFolder() {
  const [folder, setFolder] = useState<string | null>(null);
  useEffect(() => void vaultLocation().then(setFolder, () => {}), []);
  return (
    folder && (
      <>
        <p className="hint">{s.storage.folderHere}</p>
        <p className="hint path">{folder}</p>
      </>
    )
  );
}

export function NoVault({ vaultMissing, onOperationOutcome, afterKit, initialStep, onStep }: {
  /** The settings file says a Vault was here, and its file is gone. */
  vaultMissing: boolean;
  onOperationOutcome: (message: string) => void;
  afterKit: (notSaved: string, e?: unknown) => void;
  /** Read when this screen appears. */
  initialStep: SetupStep;
  onStep: (step: SetupStep) => void;
}) {
  const { busy, error, run } = useAction();
  const [step, setStep] = useState<SetupStep | "open">(initialStep);
  // No Vault file, but Safety Copies: this is not a first run. Say so, before the owner
  // concludes their Notes are gone and creates a new Vault over them. Until the first list
  // arrives, say it is still looking: that is not the same as none.
  const safety = useSafetyCopies();
  const copies = safety.copies ?? [];
  const looking = safety.copies === null && !safety.error;
  // Nothing to warn about, so the first button goes straight to the password box.
  const noCopies = safety.copies !== null && safety.copies.length === 0 && !safety.error;
  const [copy, setCopy] = useState<SafetyCopy | null>(null);
  useEffect(safety.reload, []);
  const go = (next: SetupStep) => {
    setStep(next);
    onStep(next);
  };
  const begin = () =>
    run(async () => {
      try {
        await createVault();
        afterKit(s.kitCancelled);
      } catch (e) {
        afterKit(s.kitCancelled, e);
        throw e;
      }
    });
  if (step === "create")
    return (
      <div className="center narrow">
        <h1>{s.createTitle}</h1>
        {copies.length > 0 && <p className="warn">{s.vaultWasHereText}</p>}
        {vaultMissing && copies.length === 0 && <p className="warn">{s.vaultMissingText}</p>}
        {safety.error && copies.length === 0 && (
          <>
            <p className="warn">{s.safetyCopiesUncheckedText}</p>
            <ErrorLine error={safety.error} />
            <button disabled={busy} onClick={safety.reload}>{s.tryAgain}</button>
          </>
        )}
        <p>{s.createIntro}</p>
        <p>{s.setupWhereStored}</p>
        <p className="hint">{s.nativeSetupHint}</p>
        <ErrorLine error={error} />
        <button className="primary big" disabled={busy || looking} onClick={begin}>{busy ? s.settingUp : s.createVault}</button>
        <button className="link" onClick={() => go("welcome")}>
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
      {looking && <p className="hint">{s.lookingForSafetyCopies}</p>}
      {copies.length > 0 && (
        <section className="card stack">
          <h2>{s.vaultWasHereTitle}</h2>
          <p>{s.vaultWasHereText}</p>
          <VaultFolder />
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
      {/* Before Create, so the owner looks for a Backup first. Safety Copies say it already. */}
      {vaultMissing && copies.length === 0 && !looking && (
        <section className="card stack">
          <h2>{s.vaultMissingTitle}</h2>
          <p>{s.vaultMissingText}</p>
          <VaultFolder />
          <button className="primary" disabled={busy} onClick={() => setStep("open")}>
            {s.openBackup}
          </button>
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
      <ErrorLine error={error} />
      {/* Off while looking, so a Safety Copy is never skipped. */}
      <button className={vaultMissing ? "big" : "primary big"} disabled={busy || looking} onClick={() => (noCopies && !vaultMissing ? begin() : go("create"))}>
        {busy ? s.settingUp : s.createMyVault}
      </button>
      <p className="hint welcome-next">{s.welcomeNext}</p>
      <button className="link centered" disabled={busy} onClick={() => setStep("open")}>
        {s.haveVault}
      </button>
      <div className="features">
        {s.welcomeCards.map(([title, text], i) => (
          <section className="feature" key={title}>
            <Icon name={WELCOME_ICONS[i]!} />
            <h2>{title}</h2>
            <p>{text}</p>
          </section>
        ))}
      </div>
      {step === "open" && <ReplaceVault onClose={() => setStep("welcome")} onOperationOutcome={onOperationOutcome} />}
      {copy && <ReplaceVault copy={copy} onClose={() => setCopy(null)} onOperationOutcome={onOperationOutcome} />}
    </div>
  );
}

/** For a button that takes the keyboard by itself. Keys still on their way from the last screen or
 *  app don't press it: for the first second a click made by the keyboard (it has no click count) is
 *  ignored. Call `armed` whenever it takes the keyboard. */
function useStrayKeyGuard() {
  const since = useRef(0);
  const armed = () => {
    since.current = Date.now();
  };
  useEffect(() => {
    window.addEventListener("focus", armed);
    return () => window.removeEventListener("focus", armed);
  }, []);
  return { armed, strayKey: (e: { detail: number }) => e.detail === 0 && Date.now() - since.current < 1000 };
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
  const [askRecovery, setAskRecovery] = useState(false);
  const [restore, setRestore] = useState<"choose" | "backup" | null>(null);
  const safety = useSafetyCopies();
  const copies = safety.copies ?? [];
  const [copy, setCopy] = useState<SafetyCopy | null>(null);
  const { busy, error, run } = useAction();
  const wait = status.retry_in_ms ? seconds(status.retry_in_ms) : 0;
  const touchFirst = status.quick_unlock.enabled && status.quick_unlock.can_unlock;
  const primary = useRef<HTMLButtonElement>(null);
  const recovery = useRef<HTMLButtonElement>(null);
  const lastRoute = useRef<"primary" | "recovery">("primary");
  const { armed, strayKey } = useStrayKeyGuard();
  // The keyboard waits on the route just tried. After a failed Recovery Key it stays on that link,
  // so the next Return can't open the password box and count the key as a wrong password.
  useEffect(() => {
    if (busy || wait > 0 || restore || copy || askRecovery) return;
    armed();
    (lastRoute.current === "recovery" ? recovery : primary).current?.focus();
  }, [busy, wait > 0, touchFirst, restore, copy, askRecovery]);
  const withPassword = () => {
    lastRoute.current = "primary";
    run(() => unlock("password"));
  };
  const withTouchId = () => {
    lastRoute.current = "primary";
    run(quickUnlock);
  };
  const withRecoveryKey = () => {
    lastRoute.current = "recovery";
    run(() => unlock("recovery"));
  };
  const key = s.returnKey(mac);
  return (
    <div className="center tight">
      <AppIcon large />
      <h1>{s.appName}</h1>
      <div className="stack">
        <p className="hint">{touchFirst ? s.lockHintTouchId(key) : s.lockHintPassword(key)}</p>
        <ErrorLine error={error} />
        {wait > 0 && <p className="hint">{s.tryAgainIn(wait)}</p>}
        {/* Off during a wait, for Touch ID too: Rust doesn't delay it, so this is what holds it back.
            Touch ID starts only from a press here, never by itself. */}
        <button
          ref={primary}
          className="primary big"
          disabled={busy || wait > 0}
          onKeyDown={(e) => e.repeat && e.preventDefault()}
          onClick={(e) => {
            if (strayKey(e)) return;
            if (touchFirst) withTouchId();
            else withPassword();
          }}
        >
          {busy ? s.unlocking : touchFirst ? s.unlockWithTouchId : s.unlockWithPassword}
        </button>
        {touchFirst && <button disabled={busy || wait > 0} onClick={withPassword}>{s.unlockWithPassword}</button>}
        {status.quick_unlock.enabled && !touchFirst && <button disabled>{s.unlockWithTouchId}</button>}
        {status.quick_unlock.enabled && !status.quick_unlock.can_unlock && status.quick_unlock.reason && <p className="hint">{status.quick_unlock.reason}</p>}
        {status.quick_unlock.file_changed && <p className="warn">{s.quickUnlockFileChanged}</p>}
      </div>
      <button
        ref={recovery}
        className={status.show_recovery_hint ? "link emphasis" : "link"}
        disabled={busy || wait > 0}
        onKeyDown={(e) => e.repeat && e.preventDefault()}
        onClick={(e) => {
          if (strayKey(e)) return;
          // Starting this route turns Touch ID off, even on cancel, so that is asked first.
          if (status.quick_unlock.enabled) setAskRecovery(true);
          else withRecoveryKey();
        }}
      >
        {s.forgotUseKit}
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
      {askRecovery && (
        <Confirm
          title={s.useRecoveryKeyTitle}
          text={s.useRecoveryKeyText}
          confirmLabel={s.useRecoveryKeyConfirm}
          onConfirm={() => {
            setAskRecovery(false);
            withRecoveryKey();
          }}
          onClose={() => setAskRecovery(false)}
        />
      )}
      {restore === "choose" && (
        <Modal title={s.restoreVault} onClose={() => setRestore(null)}>
          <p>{s.restoreVaultText}</p>
          <VaultFolder />
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
          <div className="actions">
            <button onClick={() => setRestore(null)}>{s.close}</button>
          </div>
        </Modal>
      )}
      {restore === "backup" && <ReplaceVault retryInMs={status.retry_in_ms} onClose={() => setRestore(null)} onOperationOutcome={onOperationOutcome} />}
      {copy && <ReplaceVault copy={copy} retryInMs={status.retry_in_ms} onClose={() => setCopy(null)} onOperationOutcome={onOperationOutcome} />}
    </div>
  );
}

export function NewPasswordScreen() {
  const { busy, error, run } = useAction();
  const button = useRef<HTMLButtonElement>(null);
  const { armed, strayKey } = useStrayKeyGuard();
  // The only way forward, so the keyboard waits on it, also after Escape in the password box.
  useEffect(() => {
    if (busy) return;
    armed();
    button.current?.focus();
  }, [busy]);
  return (
    <div className="center narrow">
      <h1>{s.setNewPasswordTitle}</h1>
      <p>{s.setNewPasswordIntro}</p>
      <p className="hint">{s.lockHintPassword(s.returnKey(mac))}</p>
      <ErrorLine error={error} />
      <button
        ref={button}
        className="primary big"
        disabled={busy}
        onKeyDown={(e) => e.repeat && e.preventDefault()}
        onClick={(e) => {
          if (!strayKey(e)) run(setNewPassword);
        }}
      >
        {busy ? s.checking : s.createVault}
      </button>
      <LockLink />
    </div>
  );
}
