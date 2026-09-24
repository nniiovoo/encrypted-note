// The ONLY doorway from the screens into the Rust core (ADR-0003). Screens must call these
// functions and never `invoke` directly. This file is the contract the Rust commands implement:
// command names, argument names (camelCase here = snake_case in Rust) and response shapes
// (snake_case, as serde produces them).
//
// Trust rules the Rust side enforces:
// * Listing returns visible fields only. A Hidden Field value is returned only by showField /
//   showWalletField, one at a time, and the screens must drop it when told to hide.
// * Copy never returns the value: Rust writes the clipboard itself.
// * Passwords and Recovery Keys are sent once and the calling screen clears its input afterwards.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// ---------- shared types ----------

export type Kind = "seed_phrase" | "private_key" | "login" | "api_key" | "text";
export const KINDS: readonly Kind[] = ["seed_phrase", "private_key", "login", "api_key", "text"];
export const isWalletKind = (kind: Kind): boolean => kind === "seed_phrase" || kind === "private_key";

export type Filter =
  | { type: "all" }
  | { type: "favorites" }
  | { type: "kind"; kind: Kind }
  | { type: "trash" };

export type Chain = "evm" | "solana" | "bitcoin" | "other";

export interface NoteSummary {
  id: string;
  kind: Kind;
  title: string;
  subtitle: string | null;
  favorite: boolean;
  updated_at: number;
  deleted_at: number | null;
}

export interface NoteView {
  id: string;
  kind: Kind;
  title: string;
  /** Visible fields by name (see notes::field). */
  visible: Record<string, string>;
  /** Every Hidden Field of the Kind in order, with whether it has a value. */
  hidden: [string, boolean][];
  favorite: boolean;
  created_at: number;
  updated_at: number;
  deleted_at: number | null;
}

export type Phase =
  | "no_vault" // first run: Welcome
  | "confirm_recovery_kit" // a Vault was just created (or keys rotated): show Recovery Key, confirm
  | "locked"
  | "needs_new_password" // unlocked with the Recovery Key: must set a new Master Password
  | "unlocked";

export interface Settings {
  idle_minutes: number;
  lock_on_app_switch: boolean;
  seed_explainer_seen: boolean;
}

export interface AppStatus {
  phase: Phase;
  platform: "mac" | "windows" | "other";
  /** Result of the Capture Hiding self-check (ADR-0002). null = can't tell on this platform. */
  capture_hiding_active: boolean | null;
  sharing_guard: { up: boolean; apps: string[] };
  /** Milliseconds until Auto-lock, null when not unlocked. */
  lock_in_ms: number | null;
  /** Milliseconds until the clipboard is cleared, null if nothing pending. */
  clipboard_clear_in_ms: number | null;
  /** The currently shown Hidden Field, if any (screens hide anything else). */
  reveal: { note_id: string; field: string; expires_in_ms: number | null } | null;
  failed_attempts: number;
  /** Milliseconds before another unlock attempt is allowed, null if allowed now. */
  retry_in_ms: number | null;
  show_recovery_hint: boolean;
  backup: { last_backup_at: number | null; reminder_due: boolean; unbacked_changes: boolean };
  settings: Settings;
}

export interface Assessment {
  acceptable: boolean;
  score: number;
  feedback: string[];
}

export interface SeedCheck {
  unknown_words: number[];
  checksum_ok: boolean;
}

export type CloudProvider = "i_cloud_drive" | "mac_desktop_or_documents" | "one_drive" | "dropbox" | "google_drive";

export interface PickedFile {
  /** For display only; Rust keeps the real path. */
  display_path: string;
  cloud: CloudProvider | null;
}

export interface BackupWritten {
  display_path: string;
  note_count: number;
}

export interface VaultPreview {
  note_count: number;
  changed_at: number;
  /** null when this computer has no Vault yet. */
  older_than_current: boolean | null;
}

export interface SafetyCopy {
  id: string;
  kind: "automatic" | "replaced";
  created_at: number;
}

export type Credential = { master_password: string } | { recovery_key: string };

/** Errors come back as { code, message }; message is already plain, owner-facing English. */
export interface ApiError {
  code:
    | "wrong_credential"
    | "damaged"
    | "unsupported"
    | "retry_later"
    | "weak_password"
    | "locked"
    | "sharing_guard_up"
    | "not_found"
    | "invalid_input"
    | "already_open"
    | "cancelled"
    | "io"
    | "internal";
  message: string;
}

export const isApiError = (e: unknown): e is ApiError =>
  typeof e === "object" && e !== null && "code" in e && "message" in e;

// ---------- status & events ----------

export const appStatus = () => invoke<AppStatus>("app_status");
/** Report owner activity (throttle to about once per 5 s). */
export const activity = () => invoke<void>("activity");
/** Rust emits this whenever the status changes (lock, guard, reveal hidden, clipboard cleared). */
export const onStatusChanged = (handler: (s: AppStatus) => void): Promise<UnlistenFn> =>
  listen<AppStatus>("status-changed", (event) => handler(event.payload));

// ---------- setup, unlock, lock ----------

export const suggestPassphrase = () => invoke<string>("suggest_passphrase");
export const assessPassword = (password: string) => invoke<Assessment>("assess_password", { password });
/** Calibrates the KDF (~1 s), creates the Vault in memory, returns the Recovery Key to show once.
 *  Nothing is written to disk until confirmRecoveryKit succeeds. Phase -> confirm_recovery_kit. */
export const createVault = (masterPassword: string) =>
  invoke<{ recovery_key: string }>("create_vault", { masterPassword });
/** The owner retypes the last two groups of the Recovery Key. On success the Vault is saved
 *  (or the rotation is committed) and phase -> unlocked. */
export const confirmRecoveryKit = (lastGroups: string) => invoke<void>("confirm_recovery_kit", { lastGroups });
export const unlock = (credential: Credential) => invoke<void>("unlock", { credential });
/** After a Recovery Key unlock (phase needs_new_password). */
export const setNewPassword = (masterPassword: string) => invoke<void>("set_new_password", { masterPassword });
/** Works whenever the Vault is open, including needs_new_password and confirm_recovery_kit
 *  (an unconfirmed new Vault or Key Rotation is dropped: nothing was saved yet). */
export const lock = () => invoke<void>("lock");

// ---------- Notes ----------

export const listNotes = (filter: Filter, query: string) => invoke<NoteSummary[]>("list_notes", { filter, query });
/** Also tells the session a Note was opened (hides any shown value). */
export const getNote = (id: string) => invoke<NoteView>("get_note", { id });

export interface NoteInput {
  kind: Kind;
  title: string;
  visible: Record<string, string>;
  /** Only fields being set. For edits, omitted = keep, "" = clear. */
  hidden: Record<string, string>;
}
/** Wallet Kinds require the Master Password (ADR-0004). */
export const createNote = (input: NoteInput, masterPassword?: string) =>
  invoke<string>("create_note", { input, masterPassword: masterPassword ?? null });
export const updateNote = (id: string, input: Omit<NoteInput, "kind">, masterPassword?: string) =>
  invoke<void>("update_note", { id, input, masterPassword: masterPassword ?? null });
export const trashNote = (id: string) => invoke<void>("trash_note", { id });
export const restoreNote = (id: string) => invoke<void>("restore_note", { id });
export const deleteForever = (id: string) => invoke<void>("delete_forever", { id });
export const emptyTrash = () => invoke<number>("empty_trash");
export const setFavorite = (id: string, favorite: boolean) => invoke<void>("set_favorite", { id, favorite });

// ---------- showing & copying ----------

/** Non-wallet Kinds. Shown for 30 s (status.reveal says when it expires). */
export const showField = (id: string, field: string) => invoke<string>("show_field", { id, field });
/** Wallet Kinds: Master Password again; mode "held" (until hideField) or "accessible" (20 s). */
export const showWalletField = (id: string, field: string, masterPassword: string, mode: "held" | "accessible") =>
  invoke<string>("show_wallet_field", { id, field, masterPassword, mode });
export const hideField = () => invoke<void>("hide_field");
/** Rust writes the clipboard privately and clears it after 30 s. Wallet Kinds need the password. */
export const copyField = (id: string, field: string, masterPassword?: string) =>
  invoke<void>("copy_field", { id, field, masterPassword: masterPassword ?? null });

// ---------- checks ----------

export const seedWordlist = () => invoke<string[]>("seed_wordlist");
export const checkSeed = (phrase: string) => invoke<SeedCheck>("check_seed", { phrase });
export const checkPrivateKey = (chain: Chain, key: string) => invoke<string | null>("check_private_key", { chain, key });

// ---------- Backups & Safety Copies ----------

/** Opens the OS save dialog (Rust side). Rejects with code "cancelled" if the owner cancels. */
export const pickBackupDestination = () => invoke<PickedFile>("pick_backup_destination");
/** Writes, re-reads and verifies the Backup at the picked destination. */
export const writeBackup = () => invoke<BackupWritten>("write_backup");
/** Opens the OS open dialog (Rust side) for a Backup file. */
export const pickBackupToOpen = () => invoke<PickedFile>("pick_backup_to_open");
/** Decrypt the picked Backup in memory with its credential and describe it. */
export const inspectBackup = (credential: Credential) => invoke<VaultPreview>("inspect_backup", { credential });
/** Install the inspected Backup; the current Vault becomes a Safety Copy. Phase -> unlocked
 *  (or needs_new_password if a Recovery Key was used). */
export const replaceWithBackup = () => invoke<void>("replace_with_backup");
export const listSafetyCopies = () => invoke<SafetyCopy[]>("list_safety_copies");
export const inspectSafetyCopy = (id: string, credential: Credential) =>
  invoke<VaultPreview>("inspect_safety_copy", { id, credential });
/** Install the inspected Safety Copy (same rules as replaceWithBackup). */
export const restoreSafetyCopy = () => invoke<void>("restore_safety_copy");

// ---------- account care & settings ----------

/** rotate=false: new password only. rotate=true: Key Rotation, returns the new Recovery Key
 *  to show, and phase -> confirm_recovery_kit. */
export const changePassword = (currentPassword: string, newPassword: string, rotate: boolean) =>
  invoke<{ recovery_key: string | null }>("change_password", { currentPassword, newPassword, rotate });
export const setSettings = (settings: Omit<Settings, "seed_explainer_seen">) => invoke<void>("set_settings", { settings });
export const markSeedExplainerSeen = () => invoke<void>("mark_seed_explainer_seen");
/** "I'm not sharing right now". */
export const dismissSharingGuard = () => invoke<void>("dismiss_sharing_guard");
