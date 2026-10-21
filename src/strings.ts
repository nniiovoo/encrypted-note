// Every piece of user-facing wording lives here (PRD: English only, one strings module).
// Honesty rules: say "best-effort" for Mac screen-capture hiding, never "invisible" or
// "guaranteed"; never blame the owner.

import type { Chain, CloudProvider, Kind } from "./api";

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
// The same with the thousands marked ("1,704 characters"), for lengths.
const counted = (n: number, one: string, many: string) => `${n.toLocaleString("en-US")} ${n === 1 ? one : many}`;
// "A, B and C", for a list of field names inside a sentence.
const listed = (items: string[], last: "and" | "or") =>
  items.length < 2 ? items.join("") : `${items.slice(0, -1).join(", ")} ${last} ${items[items.length - 1]}`;
const cloudNames: Record<Exclude<CloudProvider, "mac_desktop_or_documents">, string> = {
  i_cloud_drive: "iCloud Drive",
  one_drive: "OneDrive",
  dropbox: "Dropbox",
  google_drive: "Google Drive",
};

export const strings = {
  appName: "encrypted-note",
  statusLine: "Encrypted · Stored only on this computer · Never connects to the internet",
  // `anywhere`: the owner chose in Settings to count activity anywhere on this computer.
  locksIn: (time: string, anywhere: boolean) =>
    `Locks in ${time}${anywhere ? ", counting activity anywhere on this computer" : ""}`,
  lastBackup: (date: string) => `Last backup ${date}`,
  neverBackedUp: "No Backup yet",
  lastBackupUnbacked: (date: string) => `Last backup ${date}, newer changes not backed up`,
  neverBackedUpUnbacked: "No Backup yet, changes not backed up",
  somethingWentWrong: "Something went wrong. Please try again.",

  // shared buttons and labels
  back: "Back",
  cancel: "Cancel",
  close: "Close",
  continue: "Continue",
  done: "Done",
  save: "Save",
  saving: "Saving…",
  checking: "Checking…",
  masterPassword: "Master Password",
  recoveryKey: "Recovery Key",
  title: "Title",

  // Welcome
  welcomeLead: "A calm place for your Seed Phrases, Private Keys, Logins, API Keys and private Text.",
  welcomeCards: [
    ["Stays on this computer", "Your Vault is one encrypted file on this computer. This app never connects to the internet."],
    [
      "Only you have the key",
      "Your Master Password and your Recovery Kit are the only ways in. Nobody can reset them for you, so keep your Recovery Kit safe.",
    ],
    [
      "Hidden until you choose",
      "The private parts of each Note stay as dots until you click Show. The app also asks your computer to keep its window out of screenshots and screen shares. On Mac this is best-effort, not a guarantee.",
    ],
    [
      "What it can't do",
      "It can't protect you from malware already on this computer, a phone camera pointed at your screen, someone forcing you to unlock it, or old copies of your Vault. Settings lists every limit.",
    ],
  ] as const,
  createMyVault: "Create my Vault",
  haveVault: "I already have a Vault → Open a Backup",

  // Master Password
  createTitle: "Create your Master Password",
  createIntro:
    "You'll type this to open your Vault. A few random words are strong and easy to remember. Nobody can reset it, so write it down somewhere safe.",
  suggestPassword: "Suggest a Master Password",
  newPassword: "New Master Password",
  typeItAgain: "Type it again",
  showPassword: "Show password",
  passwordsDontMatch: "The two passwords don't match yet.",
  strongEnough: "Strong enough.",
  notStrongEnough: "Not strong enough yet.",
  createVault: "Create my Vault",
  settingUp: "Setting up your Vault… this takes a few seconds.",
  setNewPasswordTitle: "Set a new Master Password",
  setNewPasswordIntro: "You opened your Vault with your Recovery Key. Choose a new Master Password to finish. Your Recovery Kit keeps working.",
  saveNewPassword: "Save new Master Password",

  // Recovery Kit
  kitTitle: "Your Recovery Kit",
  kitIntro:
    "This Recovery Key opens your Vault if you forget your Master Password. Write it on paper and keep it somewhere physical and safe. It's shown only now.",
  kitPdfWarning: "Don't save this as a PDF or photo, and never into a synced folder like iCloud Drive, OneDrive or Dropbox.",
  kitLost:
    "Your new Recovery Key can only be shown once, and this screen was reloaded. Nothing has been saved yet. Lock the app to start this step again.",
  lockAndCancel: "Lock and cancel this step",
  retypeLastGroups: "Type the last two groups of your Recovery Key",
  storedPhysically: "I've stored my Recovery Kit somewhere physical",
  oldCopiesAfterRotation:
    "Replace your Backups: old copies of your Vault still open with your old password and old Recovery Kit.",
  oldCopiesAfterChange: "Replace your Backups: old copies of your Vault still open with your old password.",

  // Lock screen
  unlock: "Unlock",
  unlocking: "Unlocking…",
  capsLockOn: "Caps Lock is on.",
  tryAgainIn: (s: number) => `You can try again in ${s} s.`,
  forgotUseKit: "Forgot it? Use your Recovery Kit",
  useMasterPassword: "Use my Master Password instead",
  restoreVault: "Restore a Backup or Safety Copy",
  restoreVaultText: "Choose a saved copy, then open it with its Master Password or Recovery Key. This computer's current Vault will become a Safety Copy.",
  recoveryKeyHint: "Type the Recovery Key from your Recovery Kit. Dashes and spaces don't matter.",

  // Sharing Guard
  shieldTitle: "Your Notes are covered",
  shieldApps: (apps: string) => `A screen sharing or recording app is running: ${apps}.`,
  shieldText: "While it runs, nothing in this app can be shown or copied.",
  notSharing: "I'm not sharing right now",
  shareTip: "On calls, share one window, not your whole screen. Lock first with Cmd/Ctrl+L.",

  // Home
  nav: { all: "All", favorites: "Favorites", trash: "Trash", settings: "Settings" },
  trashPurgeFailed: "Some Trash could not be removed because the Vault couldn't be saved. The Notes are still in Trash; try again after checking that the Vault folder is writable.",
  navKinds: "Kinds",
  noteCount: (n: number) => plural(n, "Note", "Notes"),
  shortcut: (mac: boolean, key: string) => (mac ? `⌘${key}` : `Ctrl+${key}`),
  shiftShortcut: (mac: boolean, key: string) => (mac ? `⇧⌘${key}` : `Ctrl+Shift+${key}`),
  kindNames: { seed_phrase: "Seed Phrase", private_key: "Private Key", login: "Login", api_key: "API Key", text: "Text" } satisfies Record<Kind, string>,
  kindPlurals: { seed_phrase: "Seed Phrases", private_key: "Private Keys", login: "Logins", api_key: "API Keys", text: "Text" } satisfies Record<Kind, string>,
  kindBlurbs: {
    seed_phrase: "A 12- or 24-word wallet recovery phrase",
    private_key: "One wallet's private key and address",
    login: "A website, username and password",
    api_key: "A service and its key or token",
    text: "Anything else",
  } satisfies Record<Kind, string>,
  lock: "Lock",
  search: "Search",
  searchLabel: "Search titles, websites, names, addresses",
  newNote: "New Note",
  pickKind: "What would you like to add?",
  firstNote: "Add your first Note",
  pickNote: "Pick a Note on the left.",
  noMatches: "Nothing matches that search.",
  nothingHere: "Nothing here yet.",
  sortLabel: "Sort",
  sortRecent: "Recent",
  sortAZ: "A to Z",
  copyFrom: (field: string, title: string) => `Copy ${field} of ${title}`,
  // What a screen reader says for the row the arrow keys are on while typing in the search box.
  markedRow: (title: string, kind: string) => `${title}, ${kind}`,
  backupReminder: "Your latest changes haven't been backed up for over a week.",
  backUpNow: "Back up now",
  goToBackups: "Go to Backups",
  // An API Key's expiry date, set against today's date on this computer.
  expired: "Expired",
  expiresIn: (days: number) => (days === 0 ? "Expires today" : days === 1 ? "Expires tomorrow" : `Expires in ${days} days`),
  // `past`: API Keys whose date has gone by; `soon`: those within `days` days of it.
  expiryBanner: (past: number, soon: number, days: number) =>
    past && soon
      ? `${plural(past, "API Key is past its expiry date", "API Keys are past their expiry date")}, and ${soon} more ${soon === 1 ? "expires" : "expire"} within ${days} days.`
      : past
        ? `${plural(past, "API Key is past its expiry date", "API Keys are past their expiry date")}.`
        : `${plural(soon, "API Key expires", "API Keys expire")} within ${days} days.`,
  showApiKeys: "Show API Keys",
  captureOn: (mac: boolean) => (mac ? "Screen-capture protection: on (best-effort on Mac)" : "Screen-capture protection: on"),
  captureOff: "Screen-capture protection is off. Avoid sharing your screen while this app is open.",
  captureUnknown: "Screen-capture protection: can't check on this computer",
  copiedFrom: (field: string, title: string) => `Copied ${field} of “${title}”.`,
  copied: (s: number, source = "Copied.") => `${source} The clipboard clears in ${s} s.`,
  // Said once per copy, with the time that copy has: 30 or 60 seconds as chosen in Settings, and
  // always 30 for a Seed Phrase or Private Key.
  copiedAnnounce: (s: number, source = "Copied.") => `${source} The clipboard clears in ${plural(s, "second", "seconds")}.`,

  // Note detail
  created: (date: string) => `Created ${date}`,
  updated: (date: string) => `Updated ${date}`,
  favorite: "Favorite",
  show: "Show",
  hide: "Hide",
  copy: "Copy",
  replace: "Replace",
  add: "Add",
  // What a screen reader says for a button repeated on every row, e.g. "Copy Password".
  fieldAction: (action: string, field: string) => `${action} ${field}`,
  // One line of a shown value of several lines. `line` counts from 1; never what the line holds.
  copyLine: (line: number, field: string) => `Copy line ${line} of ${field}`,
  // An API Key with a variable name: the name is a visible part of the Note, the key is not shown.
  copyAsEnv: (name: string) => `Copy as ${name}=value`,
  edit: "Edit",
  duplicate: "Duplicate",
  duplicateTitle: (title: string) => `${title} copy`,
  moveToTrash: "Move to Trash",
  walletTrashText: "Moving a Seed Phrase or Private Key to Trash needs your Master Password, like every other change to one.",
  walletDeleteText: "Deleting a Seed Phrase or Private Key for good needs your Master Password.",
  walletEmptyTrashText: "Trash holds a Seed Phrase or Private Key, so emptying it needs your Master Password.",
  crashTitle: "Something went wrong",
  crashText: "encrypted-note couldn't draw this screen, so it locked your Vault. Nothing was changed. Unlock to carry on.",
  crashContinue: "Continue",
  movedToTrash: "Moved to Trash. It's kept there for 30 days.",
  empty: "Empty",
  hidesIn: (s: number) => `Hides in ${s} s`,
  // More time starts the 30 seconds afresh; it doesn't add 30 to what is left.
  resetTimer: "Reset to 30 s",
  resetTimerFor: (field: string) => `Reset to 30 s for ${field}`,
  keepHolding: "Released before it appeared. Keep holding until it shows.",
  showWalletTitle: (field: string) => `Show ${field}`,
  holdHint:
    "Type your Master Password, then press Enter to show it for 20 seconds, or press and hold the button to show it only while you hold (keyboard: Tab to the button, then hold Space).",
  holdToShow: "Hold to show",
  showFor20: "Show for 20 seconds",
  showFor20AndCopy: "Show for 20 seconds and copy",
  // A Seed Phrase or Private Key shown for 20 seconds: more time starts the 20 seconds afresh,
  // like the 30 above, three times at most.
  resetTimer20: "Reset to 20 s",
  moreTimesLeft: (n: number) =>
    n > 0 ? `${plural(n, "time", "times")} left` : "No more time can be added. Enter your Master Password to show it again.",
  copyTitle: (field: string) => `Copy ${field}`,
  copyText: (s: number) => `Enter your Master Password to copy. The clipboard clears after ${s} seconds.`,
  copySeedTitle: "Copy this Seed Phrase?",
  copySeedText: "Seed phrases are safest typed by hand. Copy anyway?",
  copyAnyway: "Copy anyway",
  wordN: (n: number) => `Word ${n}`,
  enterToFinish: "Type the first letters of a word, then press Enter: once they fit only one word, it is finished for you.",

  // Editor
  newTitle: (kind: string) => `New ${kind}`,
  editTitle: (kind: string) => `Edit ${kind}`,
  fields: {
    seed_phrase: { word_count: "Number of words", words: "Words", wallet_name: "Wallet name", address: "Public address", derivation_path: "Derivation path", passphrase: "Passphrase" },
    private_key: { chain: "Chain", address: "Public address", key: "Private Key" },
    login: { website: "Website", username: "Username", purpose: "Purpose", password: "Password" },
    api_key: { service: "Service", key_id: "Key ID", purpose: "Purpose", url: "Management URL", expires: "Expiry date", env_var: "Variable name", key: "Key", secret: "Secret" },
    text: { body: "Text" },
  } as Record<Kind, Record<string, string>>,
  chains: { evm: "Ethereum / EVM", solana: "Solana", bitcoin: "Bitcoin", other: "Other" } satisfies Record<Chain, string>,
  optional: "(optional)",
  words: (n: number) => `${n} words`,
  leaveBlankToKeep: "Leave blank to keep the current value",
  leaveBlankToKeepWords: "Leave every word blank to keep the current words.",
  // What a field is for, under its box. Only where the label alone doesn't say it.
  hints: {
    seed_phrase: {
      address: "One of this wallet's public addresses, so you can tell which wallet this is.",
      derivation_path: "Where that address is found in the wallet, for example m/44'/60'/0'/0/0.",
      passphrase: "Only if your wallet uses one. Write it on paper too: without it the words alone won't restore the wallet.",
    },
    private_key: {},
    login: { purpose: "One line, up to 200 characters, saying what this Login is for." },
    api_key: {
      key_id: "The half that isn't hidden, such as an access key ID.",
      purpose: "One line, up to 200 characters, saying what this key is for.",
      url: "Where you manage this key. It is kept as text you can copy; the app never opens it.",
      expires: "Use YYYY-MM-DD, for example 2027-01-31. The list marks this key when the day is near.",
      env_var:
        "The name your programs read this key from. Use letters, digits and underscores, starting with a letter or underscore; at most 128 characters. With a name, the key can be copied as NAME=value.",
      secret: "Only if the service gives you a second hidden part. It is hidden like the key.",
    },
    text: {},
  } as Record<Kind, Record<string, string>>,
  expiryPlaceholder: "YYYY-MM-DD",
  // Under the fields of the Editor. `hidden`: the Kind's Hidden Fields, by label.
  notHidden: (hidden: string[]) =>
    `Hidden: ${listed(hidden, "and")}. Everything else in this Note is always visible: shown without Show, and found by search.`,
  // `fields`: the optional parts that older versions of the app can't read, by label.
  needsCurrentApp: (fields: string[]) =>
    `Once ${listed(fields, "or")} is filled in, this Vault needs a current encrypted-note on every computer that opens it, and a current Emergency Reader. Update those first, before you move the Vault there.`,
  // Editing a Text that has text in it: two ways to save what is typed.
  textMode: "With what you type",
  replaceTheText: "Replace the text",
  addToTheEnd: "Add to the end",
  typeToAdd: "Type what to add. The text already there stays hidden.",
  addsText: "Saving adds this after the current text, on a new line.",
  unknownWords: (positions: number[]) =>
    `${positions.length === 1 ? "Word" : "Words"} ${positions.join(", ")} ${positions.length === 1 ? "isn't" : "aren't"} in the standard word list. You can still save.`,
  checksumFailed: "These words don't pass the standard checksum, so one may be mistyped. You can still save.",
  addTitle: "Add a title to save.",
  fillWords: (n: number, editing: boolean) =>
    editing ? `Fill all ${n} words, or leave them all blank to keep the current words.` : `Fill all ${n} words to save.`,
  discardTitle: "Discard unsaved changes?",
  discardText: "What you typed in this Note hasn't been saved yet.",
  discard: "Discard",
  savePasswordTitle: "Confirm with your Master Password",
  savePasswordText: "Seed Phrases and Private Keys need your Master Password to save.",
  explainerTitle: "A quick word about Seed Phrases",
  explainerText:
    "Wallet makers recommend keeping your recovery phrase on paper or metal, offline. Think of this as a convenient extra copy, not your only one.",
  haveCopy: "I have a paper/metal copy",
  saveAnyway: "Save anyway",
  // What the Editor can say about a typed Hidden Field without showing any of it.
  typedCount: (n: number, edgeSpace: boolean, multiline: boolean) =>
    `${counted(n, "character", "characters")}${edgeSpace ? `. It starts or ends with a space${multiline ? " or a line break" : ""}.` : ""}`,
  multilineKey: (lines: number, characters: number) =>
    `Multi-line key pasted: ${counted(lines, "line", "lines")}, ${counted(characters, "character", "characters")}`,
  clear: "Clear",
  clearField: (field: string) => `Remove the current ${field}`,
  replacesText: "Saving replaces the current text.",
  mainBlank: (field: string) => `No ${field} entered yet. You can still save.`,
  // Something that belongs in a Hidden Field was typed into a visible one. Never repeats it.
  looksLikeKey: (field: string) => `This looks like a key. The ${field} is always visible and searchable.`,
  looksLikeSeed: (field: string) => `This looks like a Seed Phrase. The ${field} is always visible and searchable.`,
  looksLikePrivateKey: (field: string) => `This looks like a Private Key. The ${field} is always visible and searchable.`,
  clearAllWords: "Clear all words",
  chainName: "Chain name",
  chainNameHint: "For a chain that isn't listed. Leave blank to save it as Other.",
  saveAndAddAnother: "Save and add another",
  savedAnother: (kind: string) => `Saved. This is a new ${kind}.`,
  // Seed Phrases and Private Keys: what the Sharing Guard shield doesn't bring back. Field names only.
  draftLossWallet: (fields: string[]) => `If the Sharing Guard covers the window, everything typed here is kept except: ${fields.join(", ")}.`,
  enterAgain: (fields: string[]) =>
    `Enter again: ${fields.join(", ")}. A Seed Phrase's words, a Passphrase and a Private Key are never kept while the Sharing Guard covers the window.`,

  // Trash
  trashText:
    "Deleted Notes wait here for 30 days. Deleted Notes still exist in older Backups and Safety Copies, so if a password or key was exposed, change it, don't just delete it.",
  deletedOn: (date: string) => `Deleted ${date}`,
  backToTrash: "Back to Trash",
  restore: "Restore",
  deleteForever: "Delete forever",
  deleteForeverText: "This removes it from this copy of your Vault. Older Backups and Safety Copies still contain it.",
  emptyTrash: "Empty Trash",
  emptyTrashText: "This removes every Note in Trash from this copy of your Vault. Older Backups and Safety Copies still contain them.",

  // Settings
  autoLockTitle: "Auto-lock",
  autoLockAfter: "Lock after this long without use",
  minutes: (m: number) => `${m} min`,
  lockOnSwitch: "Lock when I switch to another app",
  lockOnSwitchHint: "A copy made just before you switch stays on the clipboard for its normal time, so you can still paste it.",
  // Offered only where the computer can tell when it was last used (the Mac, for now).
  // `locksOnSwitch`: the switch above is on, which this one doesn't change.
  countActivityAnywhere: "Count my activity anywhere on this computer",
  countActivityAnywhereHint: (locksOnSwitch: boolean) =>
    `With this on, using the keyboard or mouse in any app counts as use, so the app stays Unlocked while this computer is in use. It still locks after 60 minutes without activity in its own window, and at once when the computer sleeps, the screen locks or you switch users.${
      locksOnSwitch ? " It also still locks when you switch to another app, as chosen above." : ""
    }`,
  alwaysLocks: "It always locks when your computer sleeps, the screen locks, the lid closes or you switch users.",
  clipboardClearAfter: "Clear a copy from the clipboard after",
  clipboardSeconds: (n: number) => `${n} seconds`,
  walletCopiesClearAfter30: "Copies of Seed Phrases and Private Keys always clear after 30 seconds.",
  backupsTitle: "Backups",
  backingUp: "Backing up…",
  backupDone: (n: number) => `Backup saved and checked: ${plural(n, "Note", "Notes")}.`,
  exfatTip: "Tip: format your USB stick as exFAT so both Mac and Windows can read it.",
  // The folder's path ends the line: a full stop after it could be read as part of it.
  vaultFolder: (folder: string) => `Your Vault and its Safety Copies are kept on this computer in: ${folder}`,
  // Desktop/Documents only *may* be synced (iCloud setting), so it gets "may" wording.
  cloudTitle: (cloud: CloudProvider) =>
    cloud === "mac_desktop_or_documents" ? "This folder may go to the internet" : "This folder goes to the internet",
  cloudWarning: (cloud: CloudProvider) =>
    cloud === "mac_desktop_or_documents"
      ? "Your Desktop or Documents folder may be synced to iCloud, so a Backup there could leave this computer. A USB stick is safer."
      : `${cloudNames[cloud]} is synced online, so a Backup there leaves this computer. A USB stick is safer.`,
  chooseAnother: "Choose another place",
  openBackupTitle: "Open a Backup",
  openBackupText: "Bring your Vault from a USB stick or your other computer. This computer's Vault is kept as a Safety Copy.",
  openBackup: "Open a Backup",
  pickBackupText: "Choose the Backup file, then enter the Master Password or Recovery Key it opens with.",
  chooseFile: "Choose file",
  unlockWith: "Open it with",
  openIt: "Open",
  previewNotes: (n: number) => `It holds ${plural(n, "Note", "Notes")}.`,
  previewChanged: (date: string) => `Last changed ${date}.`,
  // The same two facts with this computer's Vault beside the copy, where the two can be compared.
  compareCopy: (safetyCopy: boolean) => (safetyCopy ? "This Safety Copy" : "This Backup"),
  compareHere: "This computer's Vault",
  compareNotes: "Notes",
  compareChanged: "Last changed",
  // `n`: Notes here, not in Trash, that the copy doesn't hold at all. A number only, never a title.
  onlyHereWarning: (n: number, safetyCopy: boolean) =>
    `${plural(n, "Note exists", "Notes exist")} only on this computer: this ${safetyCopy ? "Safety Copy" : "Backup"} doesn't hold ${
      n === 1 ? "it" : "them"
    }. After you open it, ${n === 1 ? "that Note" : "those Notes"} will remain only in a Safety Copy on this computer.`,
  olderWarning: (safetyCopy: boolean) =>
    `This ${safetyCopy ? "Safety Copy" : "Backup"} doesn't have every change that is in the Vault on this computer: it is older, or the two were changed separately. Opening it replaces those changes (they're kept as a Safety Copy).`,
  cannotCompareWarning:
    "This computer already has a Vault, and encrypted-note couldn't compare it with this copy because it is locked. If this copy is older, opening it replaces newer changes. Not sure? Cancel, unlock first, then open it from Settings.",
  backupEjectHint: "If this is a USB stick or other removable drive, eject it before you unplug it.",
  vaultWasHereTitle: "A Vault was on this computer before",
  vaultWasHereText:
    "Its Vault file is missing, but Safety Copies of it are still here. You can restore one with its Master Password or Recovery Key.",
  keyRotationCompleted:
    "Your Master Password and Recovery Kit are changed. Backups and Safety Copies made before now, including the Safety Copies on this computer, still open with the old ones.",
  keyRotationCancelled:
    "The change was cancelled before it was saved. Your old Master Password and old Recovery Kit still apply; destroy the new Recovery Key you wrote down.",
  settingsUnreadable:
    "Your lock settings couldn't be read, so the strictest ones are in use: lock after 1 minute, and when you switch apps. Choose them again in Settings.",
  keptAsSafetyCopy: "This computer's current Vault will be kept as a Safety Copy.",
  replaceVault: "Replace this computer's Vault",
  useThisVault: "Use this Vault",
  safetyCopiesTitle: "Safety Copies",
  safetyCopiesText: "Earlier versions of your Vault the app keeps automatically, in case something goes wrong.",
  safetyKinds: { automatic: "Automatic", replaced: "Before opening a Backup" } satisfies Record<"automatic" | "replaced", string>,
  noSafetyCopies: "No Safety Copies yet.",
  lookingForSafetyCopies: "Looking for Safety Copies…",
  safetyCopiesUnreadable: "The Safety Copies couldn't be read, so this doesn't show whether there are any.",
  safetyCopiesStale: "The Safety Copies couldn't be read again, so this list may be out of date.",
  safetyCopiesUncheckedTitle: "Safety Copies couldn't be checked",
  safetyCopiesUncheckedText:
    "If this computer had a Vault before, Safety Copies of it may still be here. Try again before you create a new Vault.",
  tryAgain: "Try again",
  seedPasteTooMany: "That paste contains more than 24 words. Clear the field and paste a 12- or 24-word phrase.",
  changePasswordTitle: "Change Master Password",
  changePasswordText: "Your Recovery Kit keeps working unless you choose that someone may know your old password.",
  whyChange: "Why are you changing it?",
  justNew: "Just want a new one",
  someoneKnows: "Someone may know my old password",
  rotateExplained: "We'll also replace your Recovery Key, so you'll write a new Recovery Kit next.",
  justNewExplained:
    'Anyone who has an old copy of your Vault and your old password could also open newer copies. If that worries you, choose "Someone may know my old password".',
  currentPassword: "Current Master Password",
  changeAndRotate: "Change and make a new Recovery Kit",
  passwordChanged: "Your Master Password is changed.",
  saveDurabilityWarning: "A change was installed, but the system could not confirm it is durable. Make a Backup before closing or restarting.",
  draftLossHint: "Locking the app discards unsaved changes.",
  dismiss: "Dismiss",
  passwordChangeCompleted: "Your Master Password changed. Existing Backups and Safety Copies still use their original credentials.",
  passwordChangeFailed: "The password change did not complete; your previous Master Password remains active.",
  backupRestoreCompleted: "The Backup was restored successfully.",
  safetyCopyRestoreCompleted: "The Safety Copy was restored successfully.",
  restoreFailed: "The restore did not complete; no replacement was made.",
  privacyTitle: "Screen privacy tips",
  privacyTips: [
    "On calls, share one window, not your whole screen. Lock first with Cmd/Ctrl+L.",
    "The app asks your computer to keep its window out of screenshots, recordings and screen shares. Windows does this well; on Mac it's best-effort.",
    "When a known sharing or recording app is running, the app covers itself and blocks Show and Copy.",
    "Meetings running inside a web browser can't be detected, so lock before you share.",
  ],
  limitsTitle: "What this app can't protect you from",
  limits: [
    "Malware already running on your computer.",
    "Someone pointing a phone camera at your screen.",
    "A hacked operating system.",
    "Being forced to unlock the app.",
    "Old copies: Backups and Safety Copies keep what they held, even after you delete or change something.",
  ],
  limitsAdvice: [
    "Keep your operating system updated: many protections live there, not in this app.",
    "Review which apps may record your screen (Mac: System Settings → Privacy & Security → Screen & System Audio Recording).",
    "This app contains no network code, so it can't send your Vault anywhere.",
  ],
};
