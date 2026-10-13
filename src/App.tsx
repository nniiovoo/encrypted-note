import { useEffect, useRef, useState } from "react";
import { activity, appStatus, dismissSharingGuard, lock, onStatusChanged, type AppStatus } from "./api";
import { cmdKey, ErrorLine, mac, TitlebarDrag, useAction } from "./components";
import { Home, START, type Place } from "./Home";
import { Icon } from "./icons";
import type { Draft } from "./Note";
import { KitLost, Locked, NewPasswordScreen, NoVault, RecoveryKit, type Kit } from "./Start";
import { strings as s } from "./strings";

export function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  // The Recovery Key to show once. It is held here only because the screen that received it
  // (setup or Change Master Password) is replaced as soon as the phase moves on.
  const [kit, setKit] = useState<Kit | null>(null);
  const [operationOutcome, setOperationOutcome] = useState<string | null>(null);
  // Where the owner was in Home (sidebar filter, open Note id, sort choice), so an Auto-lock or
  // the Shield brings them back to it. Memory only; never a title, the search text or a draft.
  const [place, setPlace] = useState(START);
  // An Editor with unsaved changes. The Shield takes Home away while the Vault stays Unlocked,
  // and a half-typed Note (a key its provider shows only once, say) must not go with it. Memory
  // only, and gone on any lock. A ref: nothing here is drawn from it, and typing in the Editor
  // must not redraw the whole window.
  const draft = useRef<Draft | null>(null);
  const lockedOnPurpose = useRef(false);
  const phase = status?.phase;
  const forgetPlace = () => setPlace((p) => ({ ...START, sort: p.sort }));
  // The Lock button and the Lock shortcut: the owner chose to leave, so the place is not kept.
  const lockNow = () => {
    lockedOnPurpose.current = true;
    draft.current = null;
    lock().catch(() => {
      lockedOnPurpose.current = false;
    });
  };

  useEffect(() => {
    // Rust pushes every change; polling each second also keeps the countdowns ticking. A poll
    // answered after a newer event is stale (it could bring titles back behind the Shield).
    const unlisten = onStatusChanged((next) => {
      setStatus((current) => !current || next.revision > current.revision ? next : current);
    });
    const poll = () => {
      appStatus().then((next) => setStatus((current) => !current || next.revision > current.revision ? next : current), () => {});
    };
    poll();
    const timer = setInterval(poll, 1000);
    // Activity for Auto-lock, at most once per 5 s.
    let last = 0;
    const onActivity = () => {
      if (Date.now() - last < 5000) return;
      last = Date.now();
      activity().catch(() => {});
    };
    const events = ["pointerdown", "keydown", "wheel"] as const;
    events.forEach((e) => window.addEventListener(e, onActivity, true));
    return () => {
      unlisten.then((u) => u());
      clearInterval(timer);
      events.forEach((e) => window.removeEventListener(e, onActivity, true));
    };
  }, []);

  useEffect(() => {
    if (phase !== "confirm_recovery_kit") {
      // A Key Rotation is saved only when the new Recovery Kit is confirmed, which unlocks;
      // leaving that step any other way (a lock, of any kind) threw it away. Say which.
      if (kit?.rotation) setOperationOutcome(phase === "unlocked" ? s.keyRotationCompleted : s.keyRotationCancelled);
      setKit(null);
    }
    // The place survives an Auto-lock and an app-switch lock, but not a lock the owner asked for,
    // nor a Vault that is new, was opened with the Recovery Key or is getting new keys.
    if (phase === "locked" ? lockedOnPurpose.current : phase !== "unlocked") forgetPlace();
    lockedOnPurpose.current = false;
    // A draft survives the Shield only: never a lock of any kind, nor any other way out of Home.
    if (phase !== "unlocked") draft.current = null;
    // Cmd/Ctrl+L works whenever the Vault is open.
    if (phase !== "unlocked" && phase !== "needs_new_password" && phase !== "confirm_recovery_kit") return;
    const onKey = (e: KeyboardEvent) => {
      // Locking is the safe direction, so Mac takes Ctrl+L as well as Cmd+L.
      if ((mac ? e.metaKey || e.ctrlKey : cmdKey(e)) && e.key.toLowerCase() === "l") {
        e.preventDefault();
        lockNow();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase]);

  // Home draws its own drag areas over this strip.
  return (
    <main className="app">
      <TitlebarDrag />
      {status?.save_durability_uncertain && <p className="warn">{s.saveDurabilityWarning}</p>}
      {status?.settings_unreadable && <p className="warn">{s.settingsUnreadable}</p>}
      {operationOutcome && <p className="warn" role="status">{operationOutcome}<button className="small" onClick={() => setOperationOutcome(null)}>{s.dismiss}</button></p>}
      {status && (
        <Screen
          status={status}
          kit={kit}
          setKit={setKit}
          place={place}
          onPlace={setPlace}
          draft={draft.current}
          onDraft={(now) => {
            draft.current = now;
          }}
          onLock={lockNow}
          onOperationOutcome={(message) => {
            // A Backup or Safety Copy was installed, or the password changed: start afresh.
            forgetPlace();
            draft.current = null;
            setOperationOutcome(message);
          }}
        />
      )}
    </main>
  );
}

function Screen({ status, kit, setKit, place, onPlace, draft, onDraft, onLock, onOperationOutcome }: {
  status: AppStatus;
  kit: Kit | null;
  setKit: (kit: Kit) => void;
  place: Place;
  onPlace: (place: Place) => void;
  /** Read by Home when it appears; see Home. */
  draft: Draft | null;
  onDraft: (draft: Draft | null) => void;
  onLock: () => void;
  onOperationOutcome: (message: string) => void;
}) {
  // The first-run screen can show a suggested Master Password in plain text.
  const sensitive = status.phase !== "locked";
  const shield = status.sharing_guard.up && sensitive;
  // What was drawn before this, so the Shield can tell an unlock from an interruption.
  const now = shield ? "shield" : status.phase;
  const [seen, setSeen] = useState({ now, before: now });
  if (seen.now !== now) setSeen({ now, before: seen.now });
  if (shield) return <Shield apps={status.sharing_guard.apps} afterUnlock={seen.before === "locked"} />;
  switch (status.phase) {
    case "no_vault":
      return <NoVault onKit={(key) => setKit({ key, rotation: false })} onOperationOutcome={onOperationOutcome} />;
    case "confirm_recovery_kit":
      return kit ? <RecoveryKit kit={kit} /> : <KitLost />;
    case "locked":
      return <Locked status={status} onOperationOutcome={onOperationOutcome} />;
    case "needs_new_password":
      return <NewPasswordScreen />;
    case "unlocked":
      return (
        <Home
          status={status}
          place={place}
          onPlace={onPlace}
          draft={draft}
          onDraft={onDraft}
          onLock={onLock}
          onKit={(key) => setKit({ key, rotation: true })}
          onOperationOutcome={onOperationOutcome}
        />
      );
  }
}

/** Sharing Guard: nothing else is rendered behind it, not even titles. `afterUnlock`: it is the
 *  first screen after an unlock, when the owner is at the keyboard and the button may take focus.
 *  When it interrupts work it must not: a stray Enter would dismiss it. */
function Shield({ apps, afterUnlock }: { apps: string[]; afterUnlock: boolean }) {
  const { error, run } = useAction();
  const [since] = useState(Date.now);
  return (
    <div className="center tight">
      <Icon name="shield" className="shield-icon" />
      <h1>{s.shieldTitle}</h1>
      <p>{s.shieldApps(apps.join(", "))}</p>
      <p>{s.shieldText}</p>
      <button
        className="primary big"
        autoFocus={afterUnlock}
        onKeyDown={(e) => e.repeat && e.preventDefault()}
        onClick={(e) => {
          // Keys still on their way from the last screen don't dismiss it: for the first second
          // a click made by the keyboard (it has no click count) is ignored.
          if (e.detail === 0 && Date.now() - since < 1000) return;
          run(dismissSharingGuard);
        }}
      >
        {s.notSharing}
      </button>
      <ErrorLine error={error} />
      <p className="hint">{s.shareTip}</p>
    </div>
  );
}
