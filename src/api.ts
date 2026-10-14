// The ONLY doorway from the screens into the Rust core (ADR-0003). Screens must call these
// functions and never `invoke` directly. This file is the contract the Rust commands implement:
// command names, argument names (camelCase here = snake_case in Rust) and response shapes
// (snake_case, as serde produces them).
//
// Trust rules the Rust side enforces:
// * Listing returns visible fields only. A Hidden Field value is returned only by showField /
//   showWalletField / showAndCopyWalletField, one at a time, and the screens must drop it when
//   told to hide.
// * Copy never returns the value: Rust writes the clipboard itself. That goes for one line of a
//   value and for NAME=value too. Text added to a Text Note is joined to its body in Rust.
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
  /** The expiry date of an API Key (YYYY-MM-DD). null for one without a date and for every
   *  other Kind. Rust always sends it. */
  expires?: string | null;
  /** Whether the Kind's first Hidden Field has a value, so a row knows whether there is
   *  anything to copy. Rust always sends it. */
  has_hidden?: boolean;
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
  /** How long a copy stays on the clipboard: 30 (default) or 60 seconds. A copied Wallet Kind
   *  value is cleared after 30 either way. Rust always sends it. */
  clipboard_clear_seconds?: number;
  /** The owner's choice (off by default) to count keyboard and mouse use anywhere on this
   *  computer as activity for Auto-lock. Even then the app locks no later than 60 minutes after
   *  the last activity inside its own window; lock_in_ms is the real countdown. Rust always
   *  sends it. */
  count_activity_anywhere?: boolean;
  /** Whether this computer can do that (the Mac only, for now). Where it is false the setting
   *  stays off whatever is sent. Rust always sends it. */
  count_activity_anywhere_supported?: boolean;
}

/** What setSettings takes. The lock rule is always sent. Each of the two newer settings may be
 *  left out, which keeps the value it has. */
export interface SettingsInput {
  idle_minutes: number;
  lock_on_app_switch: boolean;
  /** 30 or 60; anything else becomes 30. */
  clipboard_clear_seconds?: number;
  count_activity_anywhere?: boolean;
}

export interface AppStatus {
  /** Monotonic server sequence; late polls and events cannot roll the screen back. */
  revision: number;
  phase: Phase;
  platform: "mac" | "windows" | "other";
  /** Result of the Capture Hiding self-check (ADR-0002). null = can't tell on this platform. */
  capture_hiding_active: boolean | null;
  sharing_guard: { up: boolean; apps: string[] };
  /** Milliseconds until Auto-lock, null when not unlocked. */
  lock_in_ms: number | null;
  /** Milliseconds until the clipboard is cleared, null if nothing pending. */
  clipboard_clear_in_ms: number | null;
  /** How many copies the app has put on the clipboard since it started: a count, never what
   *  was copied. It goes up by one for every copy that worked, so a change means a new copy;
   *  the time left can't tell (a Wallet Kind copy may have less than the one before it, and a
   *  clear that is being tried again has more). A lock doesn't reset it. Rust always sends
   *  it. */
  copies?: number;
  /** The currently shown Hidden Field, if any (screens hide anything else). */
  reveal: {
    note_id: string;
    field: string;
    expires_in_ms: number | null;
    /** Whether extendReveal would give this value more time right now. Never for a value
     *  shown while held. Rust always sends it. */
    can_extend?: boolean;
    /** For a Wallet Kind value shown for 20 s: how many more times it can be given more time
     *  (3 at first). null where there is no such count: an ordinary Kind (no limit) or a held
     *  value (never). Rust always sends it. */
    extensions_left?: number | null;
  } | null;
  failed_attempts: number;
  /** Milliseconds before another unlock attempt is allowed, null if allowed now. */
  retry_in_ms: number | null;
  show_recovery_hint: boolean;
  backup: { last_backup_at: number | null; reminder_due: boolean; unbacked_changes: boolean };
  purge_trash_failed: boolean;
  save_durability_uncertain: boolean;
  /** The settings file couldn't be read, so the strictest lock rules are in force. */
  settings_unreadable: boolean;
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
  /** false: written and checked, but the drive should be ejected properly before unplugging. */
  durable: boolean;
}

export interface VaultPreview {
  note_count: number;
  changed_at: number;
  /** true: the Vault on this computer may hold changes this copy lacks. null: nothing to compare
   *  with, or (when replaces_vault) the Vault here is Locked and couldn't be compared. */
  older_than_current: boolean | null;
  /** Installing this copy replaces a Vault that exists on this computer. */
  replaces_vault: boolean;
  // What this computer's Vault holds, to say what installing the copy would replace. All three
  // have a value only when the Vault here is Unlocked and the copy is a copy of that same
  // Vault. Otherwise all three are null: there is nothing to compare, or the Vault here is
  // Locked. Rust always sends them.
  /** Notes in this computer's Vault that are not in Trash. */
  current_note_count?: number | null;
  /** When this computer's Vault was last changed (Unix seconds, like changed_at). */
  current_changed_at?: number | null;
  /** How many Notes here (not in Trash) the copy doesn't hold at all, its Trash included.
   *  A number only: no titles are sent. */
  only_here?: number | null;
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
    | "too_many" // Show refused: more than 8 different Hidden Fields within a minute
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
/** Removing a Wallet Kind needs the Master Password, like every other change to one: trashing
 *  it, deleting it for good, and emptying a Trash that holds one. */
export const trashNote = (id: string, masterPassword?: string) =>
  invoke<void>("trash_note", { id, masterPassword: masterPassword ?? null });
export const restoreNote = (id: string) => invoke<void>("restore_note", { id });
export const deleteForever = (id: string, masterPassword?: string) =>
  invoke<void>("delete_forever", { id, masterPassword: masterPassword ?? null });
export const emptyTrash = (masterPassword?: string) =>
  invoke<number>("empty_trash", { masterPassword: masterPassword ?? null });
export const setFavorite = (id: string, favorite: boolean) => invoke<void>("set_favorite", { id, favorite });
/** Adds text to the end of a Text Note's body, on a new line (none is added when the body is
 *  empty or already ends with a line break). Only the new text is sent: Rust joins it to the
 *  body and saves, and nothing comes back, so the Note need not be shown first. Text Notes
 *  only; an empty text is refused. */
export const appendText = (id: string, text: string) => invoke<void>("append_text", { id, text });

// ---------- showing & copying ----------

/** Non-wallet Kinds. Shown for 30 s (status.reveal says when it expires). */
export const showField = (id: string, field: string) => invoke<string>("show_field", { id, field });
/** Wallet Kinds: Master Password again; mode "held" (until hideField) or "accessible" (20 s). */
export const showWalletField = (id: string, field: string, masterPassword: string, mode: "held" | "accessible") =>
  invoke<string>("show_wallet_field", { id, field, masterPassword, mode });
/** Wallet Kinds: one Master Password entry shows the value for 20 s (never held) and copies it.
 *  Returns the value to show, like showWalletField in "accessible" mode. If the reveal is
 *  refused (code "too_many", "sharing_guard_up") nothing is copied either. The copy is cleared
 *  after 30 s. */
export const showAndCopyWalletField = (id: string, field: string, masterPassword: string) =>
  invoke<string>("show_and_copy_wallet_field", { id, field, masterPassword });
export const hideField = () => invoke<void>("hide_field");
/** More time for the value shown now, counted from this call: another 30 s for a non-wallet
 *  Kind, as often as asked; another 20 s for a Wallet Kind value, three times at most
 *  (status.reveal.extensions_left), then it must be shown again with the Master Password.
 *  Rejects with "not_found" when nothing is shown any more (it is never brought back) and with
 *  "invalid_input" for a held value or one that has had its three. status.reveal.can_extend
 *  says beforehand whether this will work. */
export const extendReveal = () => invoke<void>("extend_reveal");
/** Rust writes the clipboard privately and clears it after the time chosen in Settings (30 or
 *  60 s; a Wallet Kind value always after 30 s). Wallet Kinds need the password. */
export const copyField = (id: string, field: string, masterPassword?: string) =>
  invoke<void>("copy_field", { id, field, masterPassword: masterPassword ?? null });
/** Copies one visible field (username, website, service, public address, ...) the same private,
 *  timed way. No Master Password, for Wallet Kinds either. `field` must be a visible field of
 *  the Note's Kind other than word_count; an empty one is refused like an empty Hidden Field. */
export const copyVisibleField = (id: string, field: string) => invoke<void>("copy_visible_field", { id, field });
/** Copies one line of a Hidden Field without the value coming to the screens. `line` counts
 *  from 0 over every line of the value, exactly as `value.split("\n")` numbers them; a trailing
 *  "\r" is left off. An empty line and a line past the end are refused ("not_found").
 *  Non-wallet Kinds only. */
export const copyFieldLine = (id: string, field: string, line: number) =>
  invoke<void>("copy_field_line", { id, field, line });
/** For an API Key with a variable name (env_var) and a key: copies NAME=value, the value bare
 *  or in single quotes as a shell needs. A key with a line break or a single quote in it is
 *  refused ("invalid_input"); copyField still copies that key by itself. */
export const copyFieldEnv = (id: string) => invoke<void>("copy_field_env", { id });

// ---------- checks ----------

export const seedWordlist = () => invoke<string[]>("seed_wordlist");
export const checkSeed = (phrase: string) => invoke<SeedCheck>("check_seed", { phrase });
/** `chain` is the Note's chain field as it is, a Chain or any name the owner typed: a name the
 *  app doesn't know is checked like "other" (no check, null). */
export const checkPrivateKey = (chain: string, key: string) => invoke<string | null>("check_private_key", { chain, key });

// ---------- Backups & Safety Copies ----------

/** Opens the OS save dialog (Rust side). Rejects with code "cancelled" if the owner cancels. */
export const pickBackupDestination = () => invoke<PickedFile>("pick_backup_destination");
/** Writes, re-reads and verifies the Backup at the picked destination. The pick is used up:
 *  every Backup needs a fresh pickBackupDestination. */
export const writeBackup = () => invoke<BackupWritten>("write_backup");
/** The owner backed out after picking (e.g. at the cloud-folder warning). */
export const forgetBackupDestination = () => invoke<void>("forget_backup_destination");
/** Opens the OS open dialog (Rust side) for a Backup file. */
export const pickBackupToOpen = () => invoke<PickedFile>("pick_backup_to_open");
/** Decrypt the picked Backup in memory with its credential and describe it. */
export const inspectBackup = (credential: Credential) => invoke<VaultPreview>("inspect_backup", { credential });
/** Install the inspected Backup; the current Vault becomes a Safety Copy. Phase -> unlocked
 *  (or needs_new_password if a Recovery Key was used). */
export const replaceWithBackup = () => invoke<void>("replace_with_backup");
/** The preview was closed without installing: Rust forgets the decrypted copy and the picked file. */
export const discardInspection = () => invoke<void>("discard_inspection");
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
export const setSettings = (settings: SettingsInput) => invoke<void>("set_settings", { settings });
export const markSeedExplainerSeen = () => invoke<void>("mark_seed_explainer_seen");
/** The folder this computer's Vault file and Safety Copies are in, as text to show (built like
 *  PickedFile.display_path). Not a secret; works in every phase. */
export const vaultLocation = () => invoke<string>("vault_location");
/** "I'm not sharing right now". */
export const dismissSharingGuard = () => invoke<void>("dismiss_sharing_guard");
