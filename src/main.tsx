import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import "./styles.css";

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
