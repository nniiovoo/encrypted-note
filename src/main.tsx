import { Component, StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { lock } from "./api";
import { App } from "./App";
import { mac } from "./components";
import { strings as s } from "./strings";
import "./styles.css";

// Mac: leaves room for the traffic lights, which sit over the page.
document.documentElement.dataset.platform = mac ? "mac" : "other";

// No OS context menu anywhere: it would be an unprotected native window (ADR-0002) and
// offers Writing Tools / spelling services on secret text.
window.addEventListener("contextmenu", (event) => event.preventDefault());

/** If a screen throws while it is drawn, React takes every screen away: a blank window, no Lock
 *  button or shortcut, and the Vault still Unlocked behind it. So lock at once and say so.
 *  Everything App kept in memory (the open Note, a draft) goes with it. */
class LockOnCrash extends Component<{ children: ReactNode }, { crashed: boolean }> {
  override state = { crashed: false };

  static getDerivedStateFromError() {
    return { crashed: true };
  }

  override componentDidCatch() {
    lock().catch(() => {});
  }

  override render() {
    if (!this.state.crashed) return this.props.children;
    return (
      <main className="app">
        <div className="center tight">
          <h1>{s.crashTitle}</h1>
          <p>{s.crashText}</p>
          <button className="primary big" onClick={() => this.setState({ crashed: false })}>
            {s.crashContinue}
          </button>
        </div>
      </main>
    );
  }
}

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <LockOnCrash>
        <App />
      </LockOnCrash>
    </StrictMode>,
  );
}
