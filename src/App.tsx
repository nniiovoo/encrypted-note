import { useEffect, useState } from "react";
import { activity, appStatus, dismissSharingGuard, lock, onStatusChanged, type AppStatus } from "./api";
import { ErrorLine, TitlebarDrag, useAction } from "./components";
import { Home } from "./Home";
import { Icon } from "./icons";
import { KitLost, Locked, NewPasswordScreen, NoVault, RecoveryKit, type Kit } from "./Start";
import { strings as s } from "./strings";

export function App() {
  const [status, setStatus] = useState<AppStatus | null>(null);
  // The Recovery Key to show once. It is held here only because the screen that received it
  // (setup or Change Master Password) is replaced as soon as the phase moves on.
  const [kit, setKit] = useState<Kit | null>(null);
  const phase = status?.phase;

  useEffect(() => {
    // Rust pushes every change; polling each second also keeps the countdowns ticking. A poll
    // answered after a newer event is stale (it could bring titles back behind the Shield).
    let received = 0;
    const unlisten = onStatusChanged((next) => {
      received++;
      setStatus(next);
    });
    const poll = () => {
      const seen = received;
      appStatus().then((next) => seen === received && setStatus(next), () => {});
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
    if (phase !== "confirm_recovery_kit") setKit(null);
    // Cmd/Ctrl+L works whenever the Vault is open.
    if (phase !== "unlocked" && phase !== "needs_new_password" && phase !== "confirm_recovery_kit") return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "l") {
        e.preventDefault();
        lock().catch(() => {});
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [phase]);

  // Home draws its own drag areas over this strip.
  return (
    <main className="app">
      <TitlebarDrag />
      {status && <Screen status={status} kit={kit} setKit={setKit} />}
    </main>
  );
}

function Screen({ status, kit, setKit }: { status: AppStatus; kit: Kit | null; setKit: (kit: Kit) => void }) {
  // Only where something sensitive is on screen: the welcome and lock screens show nothing secret,
  // and Rust forgets any dismissal when a session starts, so dismissing there would be wasted.
  const sensitive = status.phase !== "no_vault" && status.phase !== "locked";
  if (status.sharing_guard.up && sensitive) return <Shield apps={status.sharing_guard.apps} />;
  switch (status.phase) {
    case "no_vault":
      return <NoVault onKit={(key) => setKit({ key, rotation: false })} />;
    case "confirm_recovery_kit":
      return kit ? <RecoveryKit kit={kit} /> : <KitLost />;
    case "locked":
      return <Locked status={status} />;
    case "needs_new_password":
      return <NewPasswordScreen />;
    case "unlocked":
      return <Home status={status} onKit={(key) => setKit({ key, rotation: true })} />;
  }
}

/** Sharing Guard: nothing else is rendered behind it, not even titles. */
function Shield({ apps }: { apps: string[] }) {
  const { error, run } = useAction();
  return (
    <div className="center tight">
      <Icon name="shield" className="shield-icon" />
      <h1>{s.shieldTitle}</h1>
      <p>{s.shieldApps(apps.join(", "))}</p>
      <p>{s.shieldText}</p>
      <button className="primary big" onClick={() => run(dismissSharingGuard)}>
        {s.notSharing}
      </button>
      <ErrorLine error={error} />
      <p className="hint">{s.shareTip}</p>
    </div>
  );
}
