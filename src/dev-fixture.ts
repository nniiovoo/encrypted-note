// Dev only, loaded by /fixture.html, which `vite build` never bundles. Answers IPC with fictional
// data so a browser can draw the screens: `npm run dev`, then
// /fixture.html?phase=unlocked&companion=running (add &guard for the Sharing Guard).
// phase: no_vault, confirm_recovery_kit, locked, needs_new_password, unlocked.
// companion: unsupported, unavailable, stopped, starting, running, stopping, expired, error.
// It never reaches Rust, so it proves nothing about native panels, locks or the real helper.
import { mockIPC } from "@tauri-apps/api/mocks";
import type { AppStatus, CompanionState, CompanionView, Filter, NoteSummary, NoteView, Phase } from "./api";

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
  quick_unlock: { available: false, enabled: false, can_unlock: false, cleanup_failed: false, reason: null },
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
  },
  {
    id: "n2",
    kind: "api_key",
    title: "Fake API key",
    visible: { service: "Example Test Service" },
    hidden: [["key", true]],
    favorite: false,
    created_at: now,
    updated_at: now,
    deleted_at: null,
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
  },
];

const summaries: NoteSummary[] = views.map(({ id, kind, title, favorite, updated_at, deleted_at }) => ({
  id,
  kind,
  title,
  subtitle: null,
  favorite,
  updated_at,
  deleted_at,
  expires: null,
  has_hidden: true,
}));

const companionState = (query.get("companion") ?? "stopped") as CompanionState;
const companion: CompanionView = {
  platform_supported: companionState !== "unsupported",
  helper_bundled: companionState !== "unsupported" && companionState !== "unavailable",
  state: companionState,
  error: companionState === "error" ? "internal" : null,
  expires_at: now + 3600,
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
      case "seed_wordlist":
        return [];
      default:
        return undefined;
    }
  },
  { shouldMockEvents: true },
);
