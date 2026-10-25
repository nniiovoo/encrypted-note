import { useEffect, useRef, useState } from "react";
import { activity, appStatus, cancelNativeOperation, dismissSharingGuard, lock, onStatusChanged, type AppStatus } from "./api";
import { cmdKey, ErrorLine, mac, TitlebarDrag, useAction } from "./components";
import { Home, START, type Place } from "./Home";
import { Icon } from "./icons";
import type { Draft } from "./Note";
import { Locked, NativeRecoveryKit, NewPasswordScreen, NoVault } from "./Start";
import { strings as s } from "./strings";

export function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [operationOutcome, setOperationOutcome] = useState<string | null>(null);
  // Where the owner was in Home (sidebar filter, open Note id, sort choice), so an Auto-lock or
  // the Shield brings them back to it. Memory only; never a title, the search text or a draft.
  const [place, setPlace] = useState(START);
  // An Editor with unsaved changes. The Shield takes Home away while the Vault stays Unlocked,
  // and a half-typed Note (a key its provider shows only once, say) must not go with it. Memory
  // only, and gone on any lock. A ref: nothing here is drawn from it, and typing in the Editor
  // must not redraw the whole window.
  const draft = useRef<Draft | null>(null);
  // Counts the outcomes reported from outside Home (see below). Home is keyed by it, so a Home
  // that is already drawn starts again instead of keeping the place it was given.
  const [restarts, setRestarts] = useState(0);
  const lockedOnPurpose = useRef(false);
  const phase = status?.phase;
  const forgetPlace = () => setPlace((p) => ({ ...START, sort: p.sort }));
  // A Backup or Safety Copy was installed, or the password changed: start afresh.
  const outcome = (message: string) => {
    forgetPlace();
    draft.current = null;
    setOperationOutcome(message);
  };
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
    const cancelOnUnload = () => void cancelNativeOperation().catch(() => {});
    window.addEventListener("beforeunload", cancelOnUnload);
    poll();
    const timer = setInterval(poll, 1000);
    // Activity for Auto-lock, at most once per 5 s. A key that is merely held down (auto-repeat)
    // is not someone using the app: a stuck or leaned-on key must not keep Auto-lock away.
    let last = 0;
    const onActivity = (e: Event) => {
      if (e instanceof KeyboardEvent && e.repeat) return;
      if (Date.now() - last < 5000) return;
      last = Date.now();
      activity().catch(() => {});
    };
    const events = ["pointerdown", "keydown", "wheel"] as const;
    events.forEach((e) => window.addEventListener(e, onActivity, true));
    return () => {
      unlisten.then((u) => u());
      clearInterval(timer);
      window.removeEventListener("beforeunload", cancelOnUnload);
      events.forEach((e) => window.removeEventListener(e, onActivity, true));
    };
  }, []);

  useEffect(() => {
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
      {status && status.lock_events_unavailable.length > 0 && (
        <p className="warn" role="alert">{s.lockEventsUnavailable(status.lock_events_unavailable.map((event) => s.lockEventNames[event]).join(", "))}</p>
      )}
      {operationOutcome && <p className="warn" role="status">{operationOutcome}<button className="small" onClick={() => setOperationOutcome(null)}>{s.dismiss}</button></p>}
      {status && (
        <Screen
          status={status}
          place={place}
          onPlace={setPlace}
          restarts={restarts}
          // A reader, not the draft itself: a prop would go on holding the draft after a lock
          // has emptied the ref.
          readDraft={() => draft.current}
          onDraft={(now) => {
            draft.current = now;
          }}
          onLock={lockNow}
          onOperationOutcome={outcome}
          // The status that unlocks can arrive before the outcome does, and then Home is already
          // drawn from the place it was given: it starts again. Not for an outcome from Settings,
          // which is inside Home and would be closed by that.
          onOutcomeOutside={(message) => {
            outcome(message);
            setRestarts((n) => n + 1);
          }}
        />
      )}
    </main>
  );
}

function Screen({ status, place, onPlace, restarts, readDraft, onDraft, onLock, onOperationOutcome, onOutcomeOutside }: {
  status: AppStatus;
  place: Place;
  onPlace: (place: Place) => void;
  /** Keys Home: when it changes, Home starts again from `place`. */
  restarts: number;
  /** Read by Home once, when it appears; see Home. */
  readDraft: () => Draft | null;
  onDraft: (draft: Draft | null) => void;
  onLock: () => void;
  /** Reported from Settings, inside Home. */
  onOperationOutcome: (message: string) => void;
  /** Reported from the Locked or first-run screen. */
  onOutcomeOutside: (message: string) => void;
}) {
  // Native credential operations follow the same Sharing Guard as the app.
  const shield = status.sharing_guard.up;
  // What was drawn before this, so the Shield can tell an unlock from an interruption.
  const now = shield ? "shield" : status.phase;
  const [seen, setSeen] = useState({ now, before: now });
  if (seen.now !== now) setSeen({ now, before: seen.now });
  if (shield) return <Shield apps={status.sharing_guard.apps} afterUnlock={status.phase !== "locked" && seen.before === "locked"} />;
  switch (status.phase) {
    case "no_vault":
      return <NoVault onOperationOutcome={onOutcomeOutside} />;
    case "confirm_recovery_kit":
      return <NativeRecoveryKit />;
    case "locked":
      return <Locked status={status} onOperationOutcome={onOutcomeOutside} />;
    case "needs_new_password":
      return <NewPasswordScreen />;
    case "unlocked":
      return (
        <Home
          key={restarts}
          status={status}
          place={place}
          onPlace={onPlace}
          readDraft={readDraft}
          onDraft={onDraft}
          onLock={onLock}
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
