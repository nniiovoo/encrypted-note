// Every piece of user-facing wording lives here (PRD: English only, one strings module).
// Honesty rules: say "best-effort" for Mac screen-capture hiding, never "invisible" or
// "guaranteed"; never blame the owner.

import type { Chain, CloudProvider, Kind } from "./api";

const plural = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;
const cloudNames: Record<Exclude<CloudProvider, "mac_desktop_or_documents">, string> = {
  i_cloud_drive: "iCloud Drive",
  one_drive: "OneDrive",
  dropbox: "Dropbox",
  google_drive: "Google Drive",
};

export const strings = {
  appName: "encrypted-note",
  statusLine: "Encrypted · Stored only on this computer · Never connects to the internet",
  locksIn: (time: string) => `Locks in ${time}`,
  lastBackup: (date: string) => `Last backup ${date}`,
  neverBackedUp: "No Backup yet",
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
  recoveryKeyHint: "Type the Recovery Key from your Recovery Kit. Dashes and spaces don't matter.",

  // Sharing Guard
  shieldTitle: "Your Notes are covered",
  shieldApps: (apps: string) => `A screen sharing or recording app is running: ${apps}.`,
  shieldText: "While it runs, nothing in this app can be shown or copied.",
  notSharing: "I'm not sharing right now",
  shareTip: "On calls, share one window, not your whole screen. Lock first with Cmd/Ctrl+L.",

  // Home
  nav: { all: "All", favorites: "Favorites", trash: "Trash", settings: "Settings" },
  navKinds: "Kinds",
  noteCount: (n: number) => plural(n, "Note", "Notes"),
  lockShortcut: (mac: boolean) => (mac ? "⌘L" : "Ctrl+L"),
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
  backupReminder: "Your latest changes haven't been backed up for over a week.",
  backUpNow: "Back up now",
  goToBackups: "Go to Backups",
  captureOn: (mac: boolean) => (mac ? "Screen-capture protection: on (best-effort on Mac)" : "Screen-capture protection: on"),
  captureOff: "Screen-capture protection is off. Avoid sharing your screen while this app is open.",
  captureUnknown: "Screen-capture protection: can't check on this computer",
  copied: (s: number) => `Copied. The clipboard clears in ${s} s.`,
  copiedAnnounce: "Copied. The clipboard clears in 30 seconds.",

  // Note detail
  updated: (date: string) => `Updated ${date}`,
  favorite: "Favorite",
  show: "Show",
  hide: "Hide",
  copy: "Copy",
  edit: "Edit",
  moveToTrash: "Move to Trash",
  movedToTrash: "Moved to Trash. It's kept there for 30 days.",
  empty: "Empty",
  hidesIn: (s: number) => `Hides in ${s} s`,
  showWalletTitle: (field: string) => `Show ${field}`,
  holdHint:
    "Type your Master Password, then press Enter to show it for 20 seconds, or press and hold the button to show it only while you hold (keyboard: Tab to the button, then hold Space).",
  holdToShow: "Hold to show",
  showFor20: "Show for 20 seconds",
  copyTitle: (field: string) => `Copy ${field}`,
  copyText: "Enter your Master Password to copy. The clipboard clears after 30 seconds.",
  copySeedTitle: "Copy this Seed Phrase?",
  copySeedText: "Seed phrases are safest typed by hand. Copy anyway?",
  copyAnyway: "Copy anyway",
  wordN: (n: number) => `Word ${n}`,
  suggestions: "Word suggestions",
  enterToFinish: "Press Enter to finish this word.",

  // Editor
  newTitle: (kind: string) => `New ${kind}`,
  editTitle: (kind: string) => `Edit ${kind}`,
  fields: {
    seed_phrase: { word_count: "Number of words", words: "Words", wallet_name: "Wallet name", passphrase: "Passphrase" },
    private_key: { chain: "Chain", address: "Public address", key: "Private Key" },
    login: { website: "Website", username: "Username", password: "Password" },
    api_key: { service: "Service", key: "Key" },
    text: { body: "Text" },
  } as Record<Kind, Record<string, string>>,
  chains: { evm: "Ethereum / EVM", solana: "Solana", bitcoin: "Bitcoin", other: "Other" } satisfies Record<Chain, string>,
  optional: "(optional)",
  words: (n: number) => `${n} words`,
  leaveBlankToKeep: "Leave blank to keep the current value",
  leaveBlankToKeepWords: "Leave every word blank to keep the current words.",
  passphraseTip: "Only if your wallet uses one. Write it on paper too: without it the words alone won't restore the wallet.",
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

  // Trash
  trashText:
    "Deleted Notes wait here for 30 days. Deleted Notes still exist in older Backups and Safety Copies, so if a password or key was exposed, change it, don't just delete it.",
  deletedOn: (date: string) => `Deleted ${date}`,
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
  alwaysLocks: "It always locks when your computer sleeps, the screen locks, the lid closes or you switch users.",
  backupsTitle: "Backups",
  backingUp: "Backing up…",
  backupDone: (n: number) => `Backup saved and checked: ${plural(n, "Note", "Notes")}.`,
  exfatTip: "Tip: format your USB stick as exFAT so both Mac and Windows can read it.",
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
  olderWarning: (safetyCopy: boolean) =>
    `This ${safetyCopy ? "Safety Copy" : "Backup"} is older than the Vault on this computer. Opening it replaces your newer changes (they're kept as a Safety Copy).`,
  keptAsSafetyCopy: "This computer's current Vault will be kept as a Safety Copy.",
  replaceVault: "Replace this computer's Vault",
  useThisVault: "Use this Vault",
  safetyCopiesTitle: "Safety Copies",
  safetyCopiesText: "Earlier versions of your Vault the app keeps automatically, in case something goes wrong.",
  safetyKinds: { automatic: "Automatic", replaced: "Before opening a Backup" } satisfies Record<"automatic" | "replaced", string>,
  noSafetyCopies: "No Safety Copies yet.",
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
