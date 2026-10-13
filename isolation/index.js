// Tauri isolation pattern (ADR-0003): every IPC message from the screens passes through this
// sandboxed iframe before it reaches the Rust core. Only our own app commands, the event plugin
// and the two window-drag commands are allowed through; a plugin command outside that list is
// dropped.
//
// This is a second check, not the boundary. Only the payload is encrypted here; the command
// name travels beside it, so script in the page could have one command approved and send
// another. The boundary is the capability file (src-tauri/capabilities/main.json), which Rust
// enforces: never grant a plugin permission there on the strength of this hook.
const ALLOWED_PREFIXES = ["plugin:event|"];
// Tauri's drag-region script: moving the window, and double-click to zoom, from the Mac title bar
// strip (the title bar is drawn over the page).
const ALLOWED_COMMANDS = ["plugin:window|start_dragging", "plugin:window|internal_toggle_maximize"];

window.__TAURI_ISOLATION_HOOK__ = (payload) => {
  const cmd = payload && typeof payload.cmd === "string" ? payload.cmd : "";
  const isPlugin = cmd.startsWith("plugin:");
  if (isPlugin && !ALLOWED_COMMANDS.includes(cmd) && !ALLOWED_PREFIXES.some((prefix) => cmd.startsWith(prefix))) {
    return null;
  }
  return payload;
};
