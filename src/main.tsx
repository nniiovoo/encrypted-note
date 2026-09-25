import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { mac } from "./components";
import "./styles.css";

// Mac: leaves room for the traffic lights, which sit over the page.
document.documentElement.dataset.platform = mac ? "mac" : "other";

// No OS context menu anywhere: it would be an unprotected native window (ADR-0002) and
// offers Writing Tools / spelling services on secret text.
window.addEventListener("contextmenu", (event) => event.preventDefault());

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
