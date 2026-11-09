// Dev only, loaded by /fixture.html, which `vite build` never bundles. Answers IPC with fictional
// data so a browser can draw the screens: `npm run dev`, then
// /fixture.html?phase=unlocked&companion=running (add &guard for the Sharing Guard).
// phase: no_vault, confirm_recovery_kit, locked, needs_new_password, unlocked.
// companion: unsupported, unavailable, stopped, starting, running, stopping, expired, error,
// and ended_by_lock (stopped because the Vault locked).
// Notices: &missing (no_vault: the Vault's files are gone), &older (an older copy is open),
// &touchid_changed (Touch ID turned off because the Vault file changed).
// Agents storing keys (ADR-0012): agent=off, running, ended_by_lock or expired (Settings);
// agent_request=store, store_wallet, edit, edit_wallet, replace or delete (a request waiting for
// the owner); &agent_note (the fake API key is marked "added by an agent"; the requests that
// name a Note mark theirs).
// It never reaches Rust, so it proves nothing about native panels, locks or the real helper.
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AgentPendingView, AgentRouteState, AgentRouteView, AppStatus, CompanionState, CompanionView, Filter, NoteSummary, NoteView, Phase } from "./api";

const query = new URLSearchParams(location.search);
const now = 1_790_000_000;
const guardUp = query.has("guard");

const status: AppStatus = {
  session_generation: 1,
  revision: 1,
  phase: (query.get("phase") ?? "unlocked") as Phase,
  platform: "mac",
  capture_hiding_active: true,
  lock_events_unavailable: [],
  sharing_guard: { up: guardUp, apps: guardUp ? ["Fake Share"] : [] },
  lock_in_ms: 600_000,
  clipboard_clear_in_ms: null,
  copies: 0,
  clipboard_source: null,
  reveal: null,
  failed_attempts: 0,
  retry_in_ms: null,
  show_recovery_hint: false,
  backup: { last_backup_at: null, reminder_due: false, unbacked_changes: false },
  purge_trash_failed: false,
  save_durability_uncertain: false,
  settings_unreadable: false,
  vault_missing: query.has("missing"),
  older_copy_opened: query.has("older"),
  quick_unlock: { available: false, enabled: false, can_unlock: false, cleanup_failed: false, file_changed: query.has("touchid_changed"), reason: null },
  settings: {
    idle_minutes: 10,
    lock_on_app_switch: true,
    seed_explainer_seen: true,
    clipboard_clear_seconds: 30,
    count_activity_anywhere: false,
    count_activity_anywhere_supported: true,
  },
};

// Every value is fake. scripts/check.sh looks for "Fake login" in dist/ to prove this file never
// ships, so keep that title.
const agentRequest = query.get("agent_request");
const views: NoteView[] = [
  {
    id: "n1",
    kind: "login",
    title: "Fake login",
    visible: { website: "https://login.example.test", username: "test-user@example.test" },
    hidden: [["password", true]],
    favorite: true,
    created_at: now,
    updated_at: now,
    deleted_at: null,
    added_by_agent: false,
  },
  {
    id: "n2",
    kind: "api_key",
    title: "Fake API key",
    // A variable name, so Copy as NAME=value and the line under it are drawn.
    visible: { service: "Example Test Service", env_var: "FAKE_API_KEY" },
    hidden: [["key", true]],
    favorite: false,
    created_at: now,
    updated_at: now,
    deleted_at: null,
    added_by_agent: false,
  },
  {
    id: "n3",
    kind: "seed_phrase",
    title: "Fake wallet",
    visible: { wallet: "Test Wallet", word_count: "12" },
    hidden: [["words", true]],
    favorite: false,
    created_at: now,
    updated_at: now,
    deleted_at: null,
    added_by_agent: false,
  },
  {
    id: "n4",
    kind: "text",
    title: "Fake text",
    visible: {},
    hidden: [["body", true]],
    favorite: false,
    created_at: now,
    updated_at: now,
    deleted_at: null,
    added_by_agent: false,
  },
];

// The fake API key is the agent's, and so is the fake wallet when a wallet request names it.
const agentNote = agentRequest === "edit_wallet" ? "n3" : "n2";
const marked = query.has("agent_note") || (!!agentRequest && !agentRequest.startsWith("store"));
for (const view of views) view.added_by_agent = marked && view.id === agentNote;

const summaries: NoteSummary[] = views.map(({ id, kind, title, favorite, updated_at, deleted_at, added_by_agent }) => ({
  id,
  kind,
  title,
  subtitle: null,
  favorite,
  updated_at,
  deleted_at,
  expires: null,
  has_hidden: true,
  added_by_agent: !!added_by_agent,
}));

const companionQuery = query.get("companion") ?? "stopped";
const endedByLock = companionQuery === "ended_by_lock";
const companionState = (endedByLock ? "stopped" : companionQuery) as CompanionState;
const companion: CompanionView = {
  platform_supported: companionState !== "unsupported",
  helper_bundled: companionState !== "unsupported" && companionState !== "unavailable",
  state: companionState,
  error: companionState === "error" ? "internal" : null,
  expires_at: now + 3600,
  ended_by_lock: endedByLock,
  local: { ready: true, runtime: true, plan: true, credential: true, handoff: true, agent: true, code: true },
  operation: {
    name: "generate_text",
    destination: "api.openai.com",
    model: "fake-model",
    max_input_bytes: 1000,
    max_output_tokens: 100,
    concurrent: 1,
    per_minute: 1,
    per_hour: 1,
    grant_minutes: 60,
  },
};

// Agents storing keys: the Settings view, and one fake request. Every title and detail is fake.
const agentQuery = query.get("agent") ?? "off";
const agentRoute: AgentRouteView = {
  state: agentQuery as AgentRouteState,
  expires_in_ms: agentQuery === "running" ? 42 * 60_000 : null,
  account: "fake-agent",
};
const noteFor = (id: string) => views.find((note) => note.id === id)!;
const requestFor = (kind: string): AgentPendingView | null => {
  const base = { request_id: "fake-request-1", changes: [], expires_in_ms: 240_000 };
  switch (kind) {
    case "store":
      return { ...base, request: "store", kind: "api_key", note_id: null, title: "Fake agent RPC", visible: { service: "rpc.example.test", purpose: "Fake testing" }, needs_password: false };
    case "store_wallet":
      return { ...base, request: "store", kind: "private_key", note_id: null, title: "Fake agent EVM key", visible: { chain: "evm", address: "0x00000000000000000000000000000000000000a1" }, needs_password: true };
    case "edit":
    case "edit_wallet":
    case "replace":
    case "delete": {
      const note = noteFor(agentNote);
      const request = kind === "edit_wallet" ? "edit" : kind;
      const changes =
        kind === "edit"
          ? [
              { field: "service", from: "Example Test Service", to: "Fake Other Service" },
              { field: "url", from: "", to: "https://manage.example.test" },
            ]
          : kind === "edit_wallet"
            ? [{ field: "address", from: "", to: "0x00000000000000000000000000000000000000b2" }]
            : [];
      return { ...base, request, kind: note.kind, note_id: note.id, title: note.title, visible: note.visible, changes, needs_password: kind !== "edit" && kind !== "replace" ? true : note.kind === "seed_phrase" };
    }
    default:
      return null;
  }
};
let pending = agentRequest ? requestFor(agentRequest) : null;
const mark = (id: string, on: boolean) => {
  for (const note of [...views, ...summaries]) if (note.id === id) note.added_by_agent = on;
};

mockIPC(
  (command, args) => {
    switch (command) {
      case "app_status":
        return status;
      case "list_notes": {
        const { filter, query: text } = args as { filter: Filter; query: string };
        return summaries
          .filter((n) => (filter.type === "favorites" ? n.favorite : filter.type === "kind" ? n.kind === filter.kind : filter.type !== "trash"))
          .filter((n) => n.title.toLowerCase().includes(text.toLowerCase()));
      }
      case "get_note":
        return views.find((note) => note.id === (args as { id: string }).id);
      case "list_safety_copies":
        return [];
      case "vault_location":
        return "/fictional/Vault.noindex";
      case "companion_status":
      case "companion_start":
      case "companion_stop":
      case "companion_setup":
        return companion;
      case "set_settings":
        return status.settings;
      case "agent_pending":
        return pending;
      case "agent_complete_store":
        // Saved as the fake API key, marked.
        pending = null;
        mark("n2", true);
        return "n2";
      case "agent_confirm_edit":
      case "agent_complete_replace":
      case "agent_confirm_delete":
      case "agent_cancel_request":
        pending = null;
        return undefined;
      case "remove_agent_mark":
        mark((args as { id: string }).id, false);
        return undefined;
      case "agent_route_status":
      case "set_agent_account":
        return agentRoute;
      case "agent_route_start":
        agentRoute.state = "running";
        agentRoute.expires_in_ms = 60 * 60_000;
        return agentRoute;
      case "agent_route_stop":
        agentRoute.state = "off";
        agentRoute.expires_in_ms = null;
        return agentRoute;
      case "seed_wordlist":
        return [];
      default:
        return undefined;
    }
  },
  { shouldMockEvents: true },
);
