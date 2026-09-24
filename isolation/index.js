// Tauri isolation pattern (ADR-0003): every IPC message from the screens passes through this
// sandboxed iframe before it reaches the Rust core. Only our own app commands and the event
// plugin are allowed through; anything else (e.g. injected by a compromised npm package trying
// a plugin command) is dropped.
const ALLOWED_PREFIXES = ["plugin:event|"];

window.__TAURI_ISOLATION_HOOK__ = (payload) => {
  const cmd = payload && typeof payload.cmd === "string" ? payload.cmd : "";
  const isPlugin = cmd.startsWith("plugin:");
  if (isPlugin && !ALLOWED_PREFIXES.some((prefix) => cmd.startsWith(prefix))) {
    return null;
  }
  return payload;
};
