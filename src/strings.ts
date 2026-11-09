// WebView wording lives here; native panel text is in Rust.
// Honesty rules: say "best-effort" for screen-capture hiding on Mac and on Windows, never
// "invisible" or "guaranteed"; never blame the owner.

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
const KIT_COMPARE = "Before you finish, compare all 7 groups with your paper.";
// Starting a password change, a Key Rotation or a restore turns Touch ID off, even if it then fails.
const TOUCH_ID_OFF = "If you used Touch ID, it is now off. You can turn it on again in Settings.";
const NOT_SHARING = "I'm not sharing right now";
const ROTATE_CHOICE = "Someone may know it, or I lost my paper";
const HELPER_KEEPS_KEY =
  "helper keeps the Agent Key in a plain-text file that other non-admin Mac users can't open: credential.env, in the helper's own folder. Software running in your own account may be able to read it. That is not encrypted Vault storage. While the helper is on, the key is also in Node.js memory, which can't be reliably erased. Stop doesn't delete the file or cancel the key. Use a key with a spending cap and a short expiry, and revoke it at OpenAI if in doubt.";
type CompanionPart = "runtime" | "plan" | "credential" | "handoff" | "agent" | "code";
// Who asked, in every line about an agent's request (ADR-0012). The app can't tell which agent
// it was (Claude Code, Codex, the Claude app or any other program in the agent's Mac user), so
// it never names one. To name one, change it here only.
const AGENT = "An agent";
const HELPER_STOPPED_UNEXPECTEDLY =
  "The helper was stopped because something unexpected happened. It won't restart by itself. You can press Start once more. If it happens again, don't keep trying: install the app again from a copy you trust, and revoke the Agent Key at OpenAI.";

export const strings = {
  appName: "encrypted-note",
  // The line at the bottom of Home, in parts: the middle one can be a link to Settings.
  statusEncrypted: "Encrypted",
  statusSaved: "Saved on this computer",
  statusOffline: "The Vault never connects to the internet",
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
  save: "Save",
  saving: "Saving…",
  checking: "Checking…",
  masterPassword: "Master Password",
  recoveryKey: "Recovery Key",
  nativeCredentialHint: "A password box opens in this window.",
  nativeSetupHint: `Next, you'll choose your Master Password and write your Recovery Key on paper. ${KIT_COMPARE}`,
  nativeKitHint: "Finish writing down your Recovery Key. Nothing is saved until you confirm it.",
  // What the keyboard calls the key that presses a focused button, for the two lock hints below.
  returnKey: (mac: boolean) => (mac ? "Return" : "Enter"),
  lockHintPassword: (key: string) => `Press ${key}, then type in the box that opens in this window.`,
  lockHintTouchId: (key: string) => `Press ${key}, then touch the sensor when the box opens in this window.`,
  useRecoveryKeyTitle: "Use your Recovery Key?",
  useRecoveryKeyText: "This turns off Touch ID, even if you cancel. You can turn it on again in Settings.",
  useRecoveryKeyConfirm: "Use Recovery Key",
  unlockWithPassword: "Unlock with Master Password",
  unlockWithTouchId: "Unlock with Touch ID",
  quickUnlockTitle: "Touch ID",
  quickUnlockText:
    "Unlock with your finger on this Mac instead of typing your Master Password. You still type your Master Password after restarting the app, and at least every 7 days. Touch ID keeps one encrypted file, quick-unlock.sealed, in your Vault folder.",
  quickUnlockWalletLimit: "To see, copy, change or delete a Seed Phrase or Private Key, you need your Master Password every time.",
  // Physical acceptance is still open (docs/quick-unlock-design.md): take out "hasn't yet been
  // checked" once it is recorded.
  quickUnlockTurnsOff:
    "Touch ID turns off when you start changing your Master Password or use your Recovery Key, even if you cancel, and when you replace this Vault with a Backup or Safety Copy. A missed finger, a lockout, a closed lid or a disconnected Touch ID keyboard should leave it on, but this hasn't yet been checked by hand on a real Mac.",
  touchIdIsOn: "Touch ID is on.",
  touchIdIsOff: "Touch ID is off.",
  enableTouchId: "Turn on Touch ID",
  disableTouchId: "Turn off Touch ID",
  quickUnlockCleanupFailed: "Touch ID is off, but the app couldn't finish removing its saved Touch ID file from this computer. Click Try again.",
  quickUnlockFileChanged:
    "Touch ID was turned off, because the Vault file on this computer changed while the app was closed: it was replaced by another Vault, or by a copy from before a change of Master Password or Recovery Key. Unlock with your Master Password. If you didn't replace it yourself, check your Notes before you turn Touch ID on again in Settings.",
  retryTouchIdCleanup: "Try again",
  title: "Title",

  // Welcome
  welcomeLead: "A calm place for your Seed Phrases, Private Keys, Logins, API Keys and private Text.",
  welcomeCards: [
    [
      "Stays on this computer",
      "Your Vault is encrypted and saved in a folder on this computer, and it never connects to the internet. Settings explains where each key is kept. On a Mac, an optional AI helper (Agent Companion), off unless you start it, can use the internet but can't open your Vault.",
    ],
    [
      "You hold the keys",
      "Your Master Password opens your Vault. Your Recovery Key, written on paper, opens it if you forget the password. Neither one is saved on this computer, and nobody can reset them. Keep the paper safe: with it, someone could open your Vault. On a Mac you can also turn on Touch ID later.",
    ],
    [
      "Hidden until you choose",
      "Passwords, keys and Seed Phrases stay as dots until you click Show. Titles stay visible. The app also asks your computer to keep this window out of screenshots and screen shares. This is best-effort, not a guarantee. It is weaker on Mac: some screen-sharing and recording tools can still see the window.",
    ],
    ["What it can't do", "It can't stop malware or AI tools running in your account on this computer, or someone watching your screen. Settings lists the main limits."],
  ] as const,
  welcomeNext: "Next you choose a Master Password, then write down your Recovery Key.",
  createMyVault: "Create my Vault",
  haveVault: "I already have a Vault → Open a Backup",

  // Master Password
  createTitle: "Create a new Vault",
  createIntro:
    "You'll type your Master Password to open your Vault. Use at least 15 characters: 5 or 6 random words are strong and easy to remember. If you forget it, only your Recovery Key can set a new one. If you write it down, use paper and keep it away from this computer.",
  createVault: "Choose a Master Password",
  setupWhereStored:
    "Your encrypted Vault will be saved in a folder on this computer. Your Master Password and Recovery Key are not saved in any file, so only you can keep them safe.",
  settingUp: "Setting up your Vault… this takes a few seconds.",
  setNewPasswordTitle: "Set a new Master Password",
  setNewPasswordIntro: "You opened your Vault with your Recovery Key. Choose a new Master Password to finish. Your Recovery Key keeps working.",

  // Recovery Key on paper (the Recovery Kit, in the code)
  kitTitle: "Write down your Recovery Key",
  lockAndCancel: "Lock and cancel this step",
  kitCancelled:
    "Setup did not finish, so nothing was saved. The Recovery Key you just wrote down will not work: cross it out. If you have an older Recovery Key for a Backup or Safety Copy, keep it. You will get a new key when you create your Vault again. This can happen if you cancel, switch to another app, the screen turns off, the app locks, screen sharing is detected, or after 10 minutes.",

  // Lock screen
  unlocking: "Unlocking…",
  // Read out by a screen reader when the app locks by itself. Fixed text, no Note data.
  vaultLockedAnnounce: "Your Vault is Locked.",
  capsLockOn: "Caps Lock is on.",
  tryAgainIn: (s: number) => `You can try again in ${s} s.`,
  forgotUseKit: "Forgot it? Use your Recovery Key",
  restoreVault: "Restore a saved copy (Backup or Safety Copy)",
  restoreVaultText:
    "A Backup is an encrypted copy of your Vault that you saved yourself, for example on a USB stick. A Safety Copy is an earlier version the app keeps by itself on this computer. Choose one, then open it with its Master Password or Recovery Key. This computer's current Vault will be kept as a Safety Copy for at least 30 days. If you use Touch ID, restoring turns it off.",

  // Sharing Guard
  shieldTitle: "This window is covered",
  shieldApps: (apps: string) => `A screen sharing or recording app is running: ${apps}.`,
  shieldText: `While this cover is on, nothing can be shown or copied. If you press "${NOT_SHARING}", Show and Copy work again, even while that app is still running. Stop any screen share first.`,
  shieldClosed: "If a Master Password or Recovery Key box was open, it is now closed. You will need to type it again.",
  notSharing: NOT_SHARING,
  shareTip: "On calls, share one window, not your whole screen. Lock first with Cmd/Ctrl+L.",

  // Home
  nav: { all: "All", favorites: "Favorites", trash: "Trash", settings: "Settings" },
  trashPurgeFailed:
    "Some Notes couldn't be removed from Trash, because the Vault couldn't be saved. They are still in Trash. Check that your disk isn't full, then lock and unlock the app to try again.",
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
  noMatches: "Nothing matches that search. Search never looks inside hidden values, so try a title or name.",
  nothingHere: "Nothing here yet.",
  emptyKind: (kinds: string) => `No ${kinds} here.`,
  noExpiringKeys: "No API Keys expire soon.",
  noFavorites: "No Favorites yet. Open a Note and click its star.",
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
  showAllApiKeys: "Show all API Keys",
  captureOn: "Screen-capture protection: on (best-effort)",
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
  noteAction: (action: string, title: string) => `${action}: “${title}”`,
  // One line of a shown value of several lines. `line` counts from 1; never what the line holds.
  copyLine: (line: number, field: string) => `Copy line ${line} of ${field}`,
  // An API Key with a variable name: the name is a visible part of the Note, the key is not shown.
  copyAsEnv: (name: string) => `Copy as ${name}=value`,
  // Under that button. The guide section's title, as it is in docs/guide.md.
  copyAsEnvNotForAgents: "This is your own key. For an AI agent, make a separate key at the service instead: see When an agent needs a key in docs/guide.md.",
  edit: "Edit",
  // The mark on a Note an agent asked to store (ADR-0012): a word, never color alone.
  addedByAgent: "Added by an agent",
  stopAgentManaging: "Stop agent managing this",
  stopAgentManagingText:
    "The agent will no longer see this key in its list, or be able to change, replace or delete it. The key itself stays here as it is. There is no way to give it back to the agent.",
  duplicate: "Duplicate",
  duplicateTitle: (title: string) => `${title} copy`,
  moveToTrash: "Move to Trash",
  walletTrashText: "Moving a Seed Phrase or Private Key to Trash needs your Master Password, like every other change to one.",
  walletDeleteText: "Deleting a Seed Phrase or Private Key for good needs your Master Password.",
  walletEmptyTrashText: (titles: string[]) =>
    `Enter your Master Password to delete these Notes forever: ${listed(titles.map((title) => `“${title}”`), "and")}.`,
  // Empty Trash when it holds a Wallet Kind: the native panel can't list the titles.
  emptyTrashWalletText: (titles: string[]) =>
    `This removes these Notes from this copy of your Vault: ${listed(titles.map((title) => `“${title}”`), "and")}. Older Backups and Safety Copies still contain them.`,
  crashTitle: "Something went wrong",
  crashText: "encrypted-note couldn't draw this screen, so it locked your Vault. Nothing was changed. Unlock to carry on.",
  crashContinue: "Continue",
  movedToTrash: "Moved to Trash. It's kept there for 30 days.",
  empty: "Empty",
  hidesIn: (s: number) => `Hides in ${s} s`,
  // More time starts the 30 seconds afresh; it doesn't add 30 to what is left.
  resetTimer: "Reset to 30 s",
  resetTimerFor: (field: string) => `Reset to 30 s for ${field}`,
  showWalletTitle: (field: string) => `Show ${field}`,
  holdHint:
    "Choose how to show this value. Type your Master Password in the box that opens. Then use its Hold to show button (keyboard: hold Space) or its 20-second display. The value stays on that screen.",
  // On this sheet these only go on to the password box. The hold happens on the screen after it.
  holdToShow: "Continue: hold to show",
  showFor20: "Continue: show for 20 seconds",
  continueShowFor20AndCopy: "Continue: show for 20 seconds and copy",
  showFor20AndCopy: "Show for 20 seconds and copy",
  copyTitle: (field: string) => `Copy ${field}`,
  copyText: (s: number) => `Enter your Master Password to copy. The clipboard clears after ${s} seconds.`,
  copySeedText: "Seed Phrases are safest typed by hand. Copy anyway?",
  copyAnyway: "Copy anyway",
  goBack: "Go back",
  wordN: (n: number) => `Word ${n}`,
  enterToFinish: "Press Space or Tab to go to the next word. Or type the first letters and press Enter: if only one word fits, it is filled in.",
  moreWordsFit: "More than one word still fits. Type another letter.",

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
      service: "For example OpenAI or GitHub.",
      key: "Paste or type the key here. It stays hidden and is saved encrypted.",
      key_id: "Only if the service also gives you a public ID. This box is not hidden.",
      purpose: "What this key is for. One line.",
      url: "The web page where you manage this key. The app never opens it.",
      expires: "Like 2027-01-31. The list marks this key when the day is near.",
      env_var: "For programmers: the name your code uses for this key, like MY_API_KEY. English letters, numbers and _ only. It can't start with a number.",
      secret: "Only if the service gives you a second hidden value.",
    },
    text: {},
  } as Record<Kind, Record<string, string>>,
  expiryPlaceholder: "YYYY-MM-DD",
  // Under the Title box of a new Note, where a blank Title takes another field's value.
  titleHint: { login: "Leave blank to use the Website.", api_key: "Leave blank to use the Service name." } as Partial<Record<Kind, string>>,
  addTitleOr: (field: string) => `Add a Title or a ${field} to save.`,
  // The summary of the optional fields' fold. `names`: their labels, joined.
  moreDetails: (names: string) => `More details (optional): ${names}`,
  // Under the fields of the Editor. `hidden`: the Kind's Hidden Fields, by label.
  notHidden: (hidden: string[]) =>
    `When you save, this Note is encrypted into your Vault on this computer. Hidden until you click Show: ${listed(hidden, "and")}. Everything else here is visible and can be found by search.`,
  // `fields`: the optional parts that older versions of the app can't read, by label.
  needsCurrentApp: (fields: string[]) =>
    `Once ${listed(fields, "or")} is filled in, older versions of this app and of the Emergency Reader (the small tool that reads a Vault without the app) can't open this Vault. They may say it is damaged; it is not. Before you move the Vault to another computer, update the app there. Also replace any Emergency Reader you saved with a current one.`,
  // Editing a Text that has text in it: two ways to save what is typed.
  textMode: "What to do with the text you type",
  replaceTheText: "Replace the text",
  addToTheEnd: "Add to the end",
  typeToAdd: "Type what to add. The text already there stays hidden.",
  // ADR-0006: saved text is never loaded back into the Editor, so a word can't be fixed in place.
  addsText: "Saving adds this after the current text, on a new line. To fix a word in the current text, choose Replace the text and type all of it again.",
  unknownWords: (positions: number[]) =>
    `${positions.length === 1 ? "Word" : "Words"} ${positions.join(", ")} ${positions.length === 1 ? "isn't" : "aren't"} in the standard word list. You can still save.`,
  checksumFailed: "These words don't pass the standard check, so one may be mistyped. You can still save.",
  addTitle: "Add a title to save.",
  fillWords: (n: number, editing: boolean) =>
    editing ? `Fill all ${n} words, or leave them all blank to keep the current words.` : `Fill all ${n} words to save.`,
  discardTitle: "Discard unsaved changes?",
  discardText: "What you typed in this Note hasn't been saved yet.",
  discardAgentText: "What you typed hasn't been saved, and the agent's request will be cancelled.",
  discard: "Discard",
  savePasswordText: "Seed Phrases and Private Keys need your Master Password to save.",
  explainerTitle: "A quick word about Seed Phrases",
  explainerText:
    "Wallet makers recommend keeping your Seed Phrase on paper or metal, offline. Think of this as a convenient extra copy, not your only one.",
  saveAnyway: "Save anyway",
  // What the Editor can say about a typed Hidden Field without showing any of it.
  typedCount: (n: number, edgeSpace: boolean, multiline: boolean) =>
    `${counted(n, "character", "characters")}${edgeSpace ? `. It starts or ends with a space${multiline ? " or a line break" : ""}.` : ""}`,
  multilineKey: (lines: number, characters: number) =>
    `Multi-line key pasted: ${counted(lines, "line", "lines")}, ${counted(characters, "character", "characters")}`,
  // Under a key or secret shown in the Editor, where Rust trims the value on save.
  editorEdgeSpace: (n: number) => `${counted(n, "character", "characters")}. The space at the start or end is removed when you save.`,
  clear: "Clear",
  clearField: (field: string) => `Remove the current ${field}`,
  replacesText: "Saving replaces the current text.",
  mainBlank: (field: string) => `You haven't added the ${field} yet. You can save now and add it later.`,
  // Something that belongs in a Hidden Field was typed into a visible one. Never repeats it.
  looksLikeKey: (field: string) => `This looks like a key, and the ${field} is not hidden.`,
  looksLikeSeed: (field: string) => `This looks like a Seed Phrase, and the ${field} is not hidden.`,
  looksLikePrivateKey: (field: string) => `This looks like a Private Key, and the ${field} is not hidden.`,
  // The same where no Move button can be offered.
  looksLikeKeyManual: (field: string, target: string) => `This looks like a key, and the ${field} is not hidden. Delete it here, then enter it in the ${target} box.`,
  looksLikeSeedManual: (field: string) => `This looks like a Seed Phrase, and the ${field} is not hidden. Delete it here and save it as a Seed Phrase Note.`,
  moveToBox: (field: string) => `Move it to the ${field} box`,
  moveToWords: "Move it to the word boxes",
  movedToBox: (field: string, needsTitle: boolean) => `Moved to the ${field} box.${needsTitle ? " Add a title to save." : ""}`,
  movedToWords: (needsTitle: boolean) => `Moved to the word boxes.${needsTitle ? " Add a title to save." : ""}`,
  // Visible fields with a rule, checked before Save. Fixed text, never the value.
  expiryProblem: "Write it like 2027-01-31, using a day that exists.",
  envVarProblem: "Use only English letters, numbers and _, don't start with a number, and keep it to 128 characters.",
  purposeProblem: "Keep this to 200 characters.",
  clearAllWords: "Clear all words",
  chainName: "Chain name",
  chainNameHint: "For a chain that isn't listed. Leave blank to save it as Other.",
  saveAndAddAnother: "Save and add another",
  savedAnother: (kind: string) => `Saved. This is a new ${kind}.`,
  // Seed Phrases and Private Keys: what the Sharing Guard shield doesn't bring back. Field names only.
  draftLossWallet: (fields: string[]) => `If a screen sharing or recording app makes the app cover this window, what you typed here is kept, except: ${fields.join(", ")}.`,
  enterAgain: (fields: string[]) => `Type these again: ${fields.join(", ")}. They are not kept while the window is covered.`,

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
  autoLockReauthLimit:
    "Making Auto-lock less strict needs your Master Password again, in a box that opens in this window. This helps prevent changes made by accident or while you are away. A hacked app or operating system is still a risk.",
  autoLockAfter: "Lock after this long without use",
  minutes: (m: number) => `${m} min`,
  lockOnSwitch: "Lock when I switch to another app",
  lockOnSwitchHint:
    "Unsaved typing is lost when it locks. Something you copied just before switching stays on the clipboard for its normal time (30 or 60 seconds), so you can still paste it.",
  // Offered only where the computer can tell when it was last used (the Mac, for now).
  // `locksOnSwitch`: the switch above is on, which this one doesn't change.
  countActivityAnywhere: "Count my activity anywhere on this computer",
  countActivityAnywhereHint: (locksOnSwitch: boolean) =>
    `Typing or using the mouse in any app keeps this app Unlocked, even if someone else is using the computer. It still locks after 60 minutes without use of its own window, and at once on sleep, screen lock or user switch.${
      locksOnSwitch ? " It also still locks when you switch to another app, as chosen above." : ""
    }`,
  lockEventNames: {
    sleep: "sleep",
    screen_lock: "screen lock",
    user_switch: "user switch",
    display_off: "the display turning off",
    lid_close: "lid close",
  },
  lockEventsUnavailable: (events: string) =>
    `This computer can't tell the app about: ${events}. The app won't lock by itself at those moments, so lock it yourself first. It still locks when it isn't used for the time you chose in Settings.`,
  lockEventsDegraded: "The app can't lock by itself at some moments; see the warning above. It still locks when it isn't used for the time you chose.",
  alwaysLocks: (mac: boolean) =>
    mac
      ? "The app also locks when the computer sleeps, the screen turns off or locks, or you switch users. Closing the lid usually turns the screen off. If an external screen is attached, lock the app yourself first."
      : "The app also locks when the computer sleeps, the screen locks, the lid closes or you switch users.",
  // `anywhere`: this computer can offer the Count my activity anywhere switch.
  autoLockMoreTime: (anywhere: boolean) =>
    `Need more time when adding keys? You can choose a longer time above${anywhere ? ", or turn on Count my activity anywhere" : ""}. The app then also stays open longer if you walk away, so someone at this computer has more time to use it.`,
  clipboardClearAfter: "Clear a copy from the clipboard after",
  clipboardSeconds: (n: number) => `${n} seconds`,
  walletCopiesClearAfter30: "Copies of Seed Phrases and Private Keys always clear after 30 seconds.",
  backupsTitle: "Backups",
  backupsIntro: "A Backup is a full encrypted copy of your Vault. Save one on a USB stick. It opens with the Master Password or Recovery Key you had when you saved it.",
  lastBackupWhere:
    "The app doesn't remember where you saved it. Unless you changed the name, look for a file named encrypted-note-backup-YYYY-MM-DD-HHMM.enote. The date and time in the name use UTC (world time), so they can differ from your clock.",
  backingUp: "Backing up…",
  backupDone: (n: number) => `Backup saved and checked: ${plural(n, "Note", "Notes")}.`,
  exfatTip: "Using the USB stick on both Mac and Windows? Format it as exFAT first. Formatting erases the stick.",
  // Desktop/Documents only *may* be synced (iCloud setting), so it gets "may" wording.
  cloudTitle: (cloud: CloudProvider) =>
    cloud === "mac_desktop_or_documents" ? "This folder may go to the internet" : "This folder goes to the internet",
  cloudWarning: (cloud: CloudProvider) =>
    cloud === "mac_desktop_or_documents"
      ? "Your Desktop or Documents folder may be synced to iCloud, so a Backup there could leave this computer. A USB stick is safer."
      : `${cloudNames[cloud]} is synced online, so a Backup there leaves this computer. A USB stick is safer.`,
  chooseAnother: "Choose another place",
  openBackupTitle: "Open a Backup",
  openBackupText: "Bring your Vault from a USB stick or your other computer. This computer's Vault is kept as a Safety Copy for at least 30 days.",
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
    }. After you open it, ${n === 1 ? "that Note" : "those Notes"} will remain only in a Safety Copy on this computer, kept for at least 30 days.`,
  olderWarning: (safetyCopy: boolean) =>
    `This ${safetyCopy ? "Safety Copy" : "Backup"} doesn't have every change that is in the Vault on this computer: it is older, or the two were changed separately. Opening it replaces those changes (they're kept as a Safety Copy for at least 30 days).`,
  cannotCompareWarning:
    "This computer already has a Vault, and encrypted-note couldn't compare it with this copy because it is locked. If this copy is older, opening it replaces newer changes. Not sure? Cancel, unlock first, then open it from Settings.",
  backupEjectHint: "If this is a USB stick or other removable drive, eject it before you unplug it.",
  vaultMissingTitle: "Your Vault's files are missing",
  vaultMissingText:
    "This app's settings show that this computer had a Vault, but its Vault file isn't in its folder now. If you have a Backup, open it to get your Notes back. A new Vault starts empty, and your Recovery Key can't bring Notes back without a Vault file.",
  vaultWasHereTitle: "A Vault was on this computer before",
  vaultWasHereText:
    "Its Vault file is missing, but Safety Copies of it are still here. A Safety Copy is an earlier version of your Vault that the app keeps by itself. You can restore one with the Master Password or Recovery Key it had when it was saved.",
  keyRotationCompleted: `Your Master Password and Recovery Key are changed. Copies made before now still open with the old ones: Backups, the Safety Copies on this computer, and full-computer backups (such as Time Machine on Mac). The Safety Copies are in the safety folder inside your Vault folder: Settings → Where your keys are stored shows where. Backups are wherever you saved them, named encrypted-note-backup-YYYY-MM-DD-HHMM.enote unless you renamed them. ${TOUCH_ID_OFF}`,
  keyRotationCancelled: `Your password was not changed, because the Recovery Key step didn't finish. Your old Master Password and old Recovery Key still work. Keep your old paper and throw away only the new key you just wrote down. If someone may know your old password, or you lost your paper, do the change again now. ${TOUCH_ID_OFF}`,
  settingsUnreadable:
    "Your lock settings couldn't be read, so the strictest ones are in use: lock after 1 minute, and when you switch apps. Choose them again in Settings.",
  olderCopyOpened:
    "This Vault is older than one this app saved or opened on this computer before. Notes you changed or deleted since then may be back, and newer ones may be missing. If you restored it yourself, for example from Time Machine, you can keep working. If you didn't, look in Settings → Safety Copies, or open your newest Backup.",
  keptAsSafetyCopy: "This computer's current Vault will be kept as a Safety Copy for at least 30 days.",
  // Beside keptAsSafetyCopy. Installing a copy writes its credential slots too (ADR-0005).
  restoreCredentialsWarning:
    "After this, your Vault opens only with the Master Password and Recovery Key this copy uses. If those differ from the ones you use now, the ones you use now stop working, so keep the paper with the Recovery Key that matches this copy. If you use Touch ID, this turns it off.",
  replaceVault: "Replace this computer's Vault",
  useThisVault: "Use this Vault",
  safetyCopiesTitle: "Safety Copies",
  // Numbers from crates/store/src/lib.rs: change them together.
  safetyCopiesText:
    "Earlier versions of your Vault the app keeps automatically, in case something goes wrong. They stay on this computer, so they don't replace a Backup. The app keeps copies from your 10 most recent saves, and the first save of each day for 30 days. A Vault replaced by a Backup or a restored copy is kept for at least 30 days; once it's older than that, it's removed when a newer one exists.",
  safetyKinds: { automatic: "Automatic", replaced: "Before opening a Backup" } satisfies Record<"automatic" | "replaced", string>,
  noSafetyCopies: "No Safety Copies yet.",
  lookingForSafetyCopies: "Looking for Safety Copies…",
  safetyCopiesUnreadable: "The Safety Copies couldn't be read, so this doesn't show whether there are any.",
  safetyCopiesStale: "The Safety Copies couldn't be read again, so this list may be out of date.",
  safetyCopiesUncheckedTitle: "Safety Copies couldn't be checked",
  safetyCopiesUncheckedText:
    "If this computer had a Vault before, Safety Copies of it (earlier versions the app keeps by itself) may still be here. Try again before you create a new Vault.",
  tryAgain: "Try again",
  seedPasteTooMany: "That paste contains more than 24 words. Clear the field and paste a 12- or 24-word phrase.",
  changePasswordTitle: "Change Master Password",
  changePasswordText: `Your Recovery Key keeps working, unless you choose "${ROTATE_CHOICE}". If you use Touch ID, starting this turns it off, even if you cancel. You can turn it on again in Settings.`,
  whyChange: "Why are you changing it?",
  justNew: "Just want a new one",
  someoneKnows: ROTATE_CHOICE,
  rotateExplained: `We'll also replace your Recovery Key, so you'll write down a new one next. ${KIT_COMPARE}`,
  justNewExplained: `Anyone who has an old copy of your Vault and your old password could also open newer copies. If that worries you, choose "${ROTATE_CHOICE}".`,
  changeAndRotate: "Change and make a new Recovery Key",
  saveDurabilityWarning: "Your change is saved, but the computer couldn't confirm it is safely on the disk. Make a Backup before you close the app or restart.",
  // `anywhere`: activity anywhere on this computer counts. `canSaveNow`: not a Wallet Kind.
  draftLossHint: (minutes: number, anywhere: boolean, onSwitch: boolean, canSaveNow: boolean) =>
    `Not saved yet. If this ${anywhere ? "computer" : "window"} isn't used for ${plural(minutes, "minute", "minutes")}${anywhere ? ", or this window for 60 minutes," : ","} the app locks and what you typed here is lost. Sleep or a locked screen locks it at once.${
      onSwitch ? " Switching to another app also locks it." : ""
    }${canSaveNow ? " You can press Save now and add the rest later." : ""}`,
  // The banner while a half-typed Note is about to be lost, and the notice after it was.
  lockSoonDraft: "Not saved. The app will lock soon and what you typed will be lost.",
  keepWorking: "Keep working",
  lockedDraftLost: "The app locked, so the Note you were typing wasn't saved.",
  dismiss: "Dismiss",
  passwordChangeCompleted: `Your Master Password changed. Copies made before now still open with the old password: Backups, the Safety Copies on this computer, and full-computer backups (such as Time Machine on Mac). ${TOUCH_ID_OFF}`,
  backupRestoreCompleted: `The Backup was restored successfully. ${TOUCH_ID_OFF}`,
  safetyCopyRestoreCompleted: `The Safety Copy was restored successfully. ${TOUCH_ID_OFF}`,
  restoreFailed: "The restore did not finish, so your Vault was not replaced. If you use Touch ID, it may now be off. You can turn it on again in Settings.",
  privacyTitle: "Screen privacy tips",
  privacyTips: [
    "On calls, share one window, not your whole screen. Lock first with Cmd/Ctrl+L.",
    // Take out "hasn't been tested" once the Windows results are recorded in docs/security-model.md.
    "The app asks your computer to keep its window out of screenshots, recordings and screen shares. This is best-effort, not a guarantee. Windows' protection is stronger than Mac's, but this app hasn't been tested on a real Windows PC yet. On Mac, some screen-sharing and recording tools can still see the window.",
    `When a known sharing or recording app is running, the app covers itself and blocks Show and Copy. If you press "${NOT_SHARING}", they work again until the app locks or another sharing app starts, so stop any screen share first.`,
    "Meetings running inside a web browser can't be detected, so lock before you share.",
  ],
  limitsTitle: "What this app can't protect you from",
  limits: [
    "Malware already running on your computer.",
    "Someone pointing a phone camera at your screen.",
    "A hacked operating system.",
    "Being forced to unlock the app.",
    "AI tools running in your own account on this computer. They work as you, so they may copy your Vault files and Backups, read what you copy and, if you allow it, see or control your screen. See AI tools on this computer in docs/guide.md.",
    "Old copies: Backups, Safety Copies and full-computer backups (such as Time Machine on Mac) keep what they held, even after you delete or change something.",
  ],
  limitsAdvice: [
    "Keep your operating system updated: many protections live there, not in this app.",
    "Review which apps may record your screen (Mac: System Settings → Privacy & Security → Screen & System Audio Recording).",
    "The Vault's own code has no network code, so the app has no built-in way to send your Vault anywhere. The app is also built with code from other projects. If someone secretly changed that code, this might not hold, and your operating system doesn't block network use. The optional Agent Companion is a separate helper that can't open the Vault.",
    "This is a personal v1 project, not independently audited. Keep a paper or metal copy of any Seed Phrase or Private Key that holds real funds.",
  ],

  // ---------- Settings: Where your keys are stored ----------
  // Each row is a label and its text. The screen picks the row's word from `status`, so the state
  // is always a word, never color alone.
  storage: {
    title: "Where your keys are stored",
    intro: "Your Vault is saved on this computer, in one folder. Here is where each thing is kept, and whether it is encrypted.",
    moreTitle: "More: older versions, Backups, settings, copies, and what leaves this computer",
    status: {
      encrypted: "Encrypted",
      notSaved: "Not saved",
      off: "Off",
      notEncrypted: "Not encrypted",
      inMemory: "In memory",
      optional: "Optional",
    },
    notes: (mac: boolean) => ({
      label: "Your Notes",
      text: `In vault.enote, in your Vault folder. This includes your Logins, API Keys, Private Keys and Seed Phrases. The keys that encrypt your Notes are saved inside vault.enote, locked by your Master Password and by your Recovery Key. It opens only with your Master Password or Recovery Key${mac ? ", or with Touch ID if you turn it on" : ""}. Seed Phrases and Private Keys are encrypted a second time.`,
    }),
    master: {
      label: "Master Password",
      text: "The app doesn't save it in any file and can't show it to you. If you forget it, your Recovery Key can open your Vault and let you choose a new one.",
    },
    recovery: {
      label: "Recovery Key",
      text: `The app doesn't save it in any file. It shows it once. The only copy is the one you wrote on paper. Lost the paper? If you still know your Master Password, you can make a new Recovery Key: further down this page, in Change Master Password, choose "${ROTATE_CHOICE}". You also choose a new Master Password there. Then choose Back up now, because your older Backups still open only with the old password and the old key.`,
    },
    // Physical acceptance is still open (docs/quick-unlock-design.md): take out "hasn't yet been
    // checked" once it is recorded. An old copied file stays usable, so don't soften that part.
    touchOn: {
      label: "Touch ID",
      text: "In quick-unlock.sealed, in your Vault folder. It holds a protected copy of the key that opens your Notes, locked by this Mac's security chip. It is made to work only on this Mac, with a fingerprint saved on this Mac, but this hasn't yet been checked by hand on a real Mac. Another app running under your Mac account could copy this file and ask for Touch ID, so approve only Touch ID requests you started. It can't open the hidden parts of Seed Phrases or Private Keys. Turning off Touch ID deletes this file, but old system backups may still hold a copy, and an old copy of the file can still work on this Mac.",
    },
    // Start-up cleanup removes any stray file from a Vault that never used Touch ID, and a failed
    // removal shows other text. A failed settings write can leave the file, so don't say it's gone.
    touchOff: {
      label: "Touch ID",
      text: "If you never tried to turn on Touch ID, the app never made a Touch ID file. If you used it before, the app deletes its file when Touch ID turns off, but old system backups may still hold a copy, and an old copy can still work on this Mac.",
    },
    agentOn: {
      label: "Agent Key",
      text: "In credential.env, in the Agent Companion helper's own folder, not in your Vault. It is plain text. Other non-admin Mac users can't open it, but programs running in your account might read it. While the helper is on, the key is also in the helper's memory. Stop doesn't delete the file. When you are done, revoke the key at OpenAI, then delete this file and the key file you chose during setup. While the Companion is on, a short-lived agent token is also kept in /Users/Shared/encrypted-note-companion. It is removed when the Companion stops normally.",
    },
    // The flag can be false while the file is there, so never say that no file exists.
    agentUnknown: {
      label: "Agent Key",
      text: "Only if you set up the Agent Companion, which lets an AI agent use OpenAI. If you never did, you can ignore this. If you did, the app can't check the key file right now. It may still be there, and it is plain text, not encrypted.",
    },
    keptHere: "Kept here:",
    keptHereIfSetUp: "If you set it up, it is kept here:",
    // A fixed string, never helper output (ADR-0009). Right for the real app: a test build made
    // with --identifier has a different folder name, which agentPathNote says.
    agentPath: "~/Library/Containers/com.encryptednote.app.companion/Data/Library/Application Support/encrypted-note-companion/credential.env",
    agentPathNote: "You don't need to open this file. A text editor can keep its own copy. If you use a test copy of the app, one name in this path is different.",
    // The page OpenAI's documentation names for API keys (docs/agent-companion.md). Shown as text
    // to copy, like the paths above: the WebView never opens an address. The address ends the line.
    agentRevoke: "To revoke the key, use OpenAI's API keys page and look under the Agent Key's own project. Click the address to select it, then copy it with Cmd+C:",
    agentRevokePage: "https://platform.openai.com/api-keys",
    folderTitle: "Your Vault folder",
    // Before the path where it is shown outside Settings. The path ends the line: a full stop
    // after it could be read as part of it.
    folderHere: "Your Vault folder on this computer:",
    folderMac: "macOS hides this folder and may ask before another app opens it. Programs you allow, and malware on this computer, could still copy these files. That is why your Vault is encrypted.",
    folderWindows: "This folder is inside your hidden AppData folder. Other programs you run could copy these files. That is why your Vault is encrypted.",
    folderCopy: "Click the path to select it, then copy it with Cmd/Ctrl+C.",
    folderLibrary: "The Library folder is hidden in Finder. To open it, press Shift-Cmd-G in Finder and paste this path.",
    folderLeave: "Don't move, rename or delete these files. To make a copy, use Back up now.",
    revealFinder: "Show in Finder",
    revealExplorer: "Show in File Explorer",
    revealLocks: (mac: boolean) => `${mac ? "Finder" : "File Explorer"} opens in front of this app, so the app locks, as you chose in Auto-lock.`,
    // Numbers from crates/store/src/lib.rs: change them together.
    safety: {
      label: "Older versions (Safety Copies)",
      text: "In the safety folder, inside your Vault folder. Each one is a complete Vault. It opens with the Master Password and Recovery Key it had when it was saved. The app keeps copies from your 10 most recent saves, and the first save of each day for 30 days. A key you deleted or replaced stays inside these copies until the app removes them.",
    },
    backups: {
      label: "Backups",
      text: "Wherever you saved them, in files named encrypted-note-backup-YYYY-MM-DD-HHMM.enote unless you changed the name. The date and time in the name use UTC (world time). The app doesn't remember where. Each one opens with the Master Password and Recovery Key it had when it was saved. A key you deleted or replaced stays inside any Backup made before that, until you delete that Backup.",
    },
    settingsFile: (mac: boolean) => ({
      label: "Settings",
      text: `In settings.json, in your Vault folder. It holds your choices and a few facts: Auto-lock rules, Backup and Trash dates, the count of wrong passwords, whether you have seen the Seed Phrase notice, your Vault's id and a count of its changes (to warn you if an older copy opens)${mac ? ", and a check value for Touch ID" : ""}. No Notes, titles, keys or passwords. Other programs could change it.`,
    }),
    copied: (n: number) => ({
      label: "Copied values",
      text: `On the clipboard for ${n} seconds, then the app clears it. Seed Phrases and Private Keys always clear after 30 seconds. Other apps could read it during that time, and a clipboard history app might keep a copy. The app writes copies to no file.`,
    }),
    // Nothing locks this memory in place (hardening.rs only turns off core dumps), so the computer
    // could still write it to the disk.
    openNotes: {
      label: "Open Notes",
      text: "While the app is Unlocked, your Notes and the key that opens them are in its memory. What you are typing but haven't saved, and a value you clicked Show on, are also in the memory of this window. The app doesn't write these to any other file. It clears them when it locks or quits, as far as the computer allows, and it can't promise the computer never moves this memory to the disk. The hidden parts of Seed Phrases and Private Keys stay encrypted until you enter your Master Password.",
    },
    backUpTitle: "What to back up",
    backUpText:
      "1. Your Recovery Key on paper: keep it somewhere safe. 2. A Backup: choose Back up now and keep the file on a USB stick. Make a new one after you change something. That is all. With a Backup and the Master Password or Recovery Key it was saved with, you can open your Vault on another computer. You don't need to copy anything else from the Vault folder.",
    leavesTitle: "What leaves this computer",
    leavesText:
      "The Vault's own code has no network code, so the app has no built-in way to send your Notes, Master Password or Recovery Key anywhere. Files leave only when they are copied: a Backup you save elsewhere, or a full-computer backup such as Time Machine on a Mac. In those copies your Vault stays encrypted, but settings.json and the Agent Key file are not encrypted. Malware on this computer is a separate risk: see What this app can't protect you from. The optional Agent Companion is separate. When you start it, it uses your Agent Key to send the text an agent gives it to OpenAI. It can't open your Vault. If you let an agent store keys, the names and visible details of the keys it adds reach that agent's AI company as part of the chat. Their values never do.",
    firstNoteWhere: "Your Notes are saved, encrypted, in:",
  },

  // ---------- Agent Companion (ADR-0009) ----------
  companionTitle: "Agent Companion",
  companionIntro:
    "Optional, and off unless you start it. It lets any program in the agent's Mac user use one OpenAI text tool, with a separate Agent Key you made only for this. It runs apart from your Vault and can't open it. The app never passes anything from your Vault to it.",
  // Take this out once docs/agent-companion.md says a full run is possible.
  companionCantFinish:
    "You can't finish this setup yet. How you log in to the agent's Mac user is still undecided, and nobody has done a full run. See docs/agent-companion.md.",
  companionUnsupported: "Only available on Mac for now.",
  companionNotBundled: "This build of the app doesn't include the Agent Companion helper. Build it with scripts/package-companion.sh --app --owner-build.",
  companionSummary: (state: string) => `Agent Companion (optional, for AI agents) · ${state}`,
  companionChecking: "Checking the Agent Companion…",
  companionStates: {
    unsupported: "Not available",
    unavailable: "Not included in this build",
    stopped: "Off",
    starting: "Starting…",
    running: "On",
    stopping: "Stopping…",
    expired: "Off: the one hour ended",
    error: "Off: needs attention",
  } satisfies Record<string, string>,
  // Not a helper state: said for "stopped" when the app stopped it because the Vault locked.
  companionEndedByLock: "Off: ended because the Vault locked",
  companionRunningUntil: (time: string) => `On until ${time} at the latest. It stops sooner if the Vault locks, the computer sleeps or the app quits.`,
  companionLocalTitle: "Ready on this Mac",
  companionLocal: {
    runtime: "Helper program found, and its rules match this app",
    plan: "A correct plan file is added (Set up…)",
    credential: "An Agent Key file is in place, not yet tried (Set up…)",
    handoff: "Private agent folder /Users/Shared/encrypted-note-companion is ready",
    agent: "The agent's Mac user is Standard, not an admin",
    code: "The helper is protected from the agent's Mac user",
  } satisfies Record<CompanionPart, string>,
  // Under an item marked ✗. Fixed text: never an account name or path from the plan file.
  companionLocalFix: {
    runtime: "The helper program doesn't match this app. Rebuild the app with scripts/package-companion.sh --app --owner-build.",
    plan: "Use Set up… to choose your filled-in plan file, with ownerUid from id -u. The items below can't be checked until this one passes.",
    credential: "Use Set up… and choose the key file you made with the command in docs/agent-companion.md.",
    handoff: "See step 3 under Before you start in docs/agent-companion.md: your own folder, mode 700, and one allow entry for the plan's agentShortName.",
    agent: "The plan's agentShortName must be an existing Mac user that is Standard, not an admin.",
    code: "Checked after the agent's Mac user passes. Only you or an admin may be able to change the app's folders: move the app to /Applications.",
  } satisfies Record<CompanionPart, string>,
  companionReady: "Ready to start. The app doesn't try the key or check its limits at OpenAI: those are yours to set.",
  companionNotReady: "Not ready: finish the items marked ✗. See Setup steps below.",
  companionChatgptTitle: "Private ChatGPT connection",
  companionChatgpt:
    "Not checked by this app. It is set up from the agent account and counts as working only after a test call you make yourself. See docs/agent-companion.md in the project folder.",
  companionOperationTitle: "What any program in the agent's Mac user can do",
  companionOperation: (o: { name: string; model: string; destination: string; max_input_bytes: number; max_output_tokens: number; per_minute: number; per_hour: number; grant_minutes: number }) => [
    `One tool: ${o.name}, with the fixed model ${o.model}. No files, browsing, other tools or other models.`,
    `Sends the text an agent gives it (at most ${o.max_input_bytes.toLocaleString("en-US")} bytes) to ${o.destination}, and returns at most ${o.max_output_tokens} tokens of text.`,
    `One request at a time; at most ${o.per_minute} a minute and ${o.per_hour} an hour. Each start lasts at most ${o.grant_minutes} minutes.`,
  ],
  companionCustody: `The ${HELPER_KEEPS_KEY}`,
  // For an owner with no confirmed key. Never says that no file exists: the check can say no while
  // the file is there.
  companionCustodyIfSetUp: `If you set this up, the ${HELPER_KEEPS_KEY}`,
  companionSetup: "Set up…",
  companionSetupDone:
    "The helper copied the files you chose. Start checks the files, not the key: the key is first tried on an agent's first call. Once that call has returned text, delete your own Agent Key file and empty the Trash. The helper keeps its own copy. If you run Set up… again, you will need a key file again. Deleting it doesn't cancel the key; only OpenAI can do that.",
  companionSetupTitle: "Setup steps",
  // Numbered by the list that draws them.
  companionSetupSteps: [
    "Make a new Standard user on this Mac for the agent (System Settings > Users & Groups). Run the agent only in that user. Never put your own passwords, Apple Account or keys there.",
    "In your own Mac user, run scripts/companion-setup.sh as docs/agent-companion.md shows. It makes the private agent folder /Users/Shared/encrypted-note-companion so that other users are kept out, a fake key file and the plan. Never put your key file in that folder.",
    "Start with a fake key, not a real one. Click Set up… and choose the plan first, then the key file.",
    "Finish the checks in docs/agent-companion.md with the fake key. Only then make a new key at OpenAI, following The real Agent Key in that guide: one permission, one model, hosted tools off, a hard spending limit and a short expiry. Never reuse a key from your Vault.",
    "Once an agent's first call with the real key has returned text, delete your own key file and empty the Trash. The helper keeps its own copy.",
  ],
  companionStart: "Start for up to one hour",
  // Beside Start while it is off (ADR-0009: any lock ends the grant).
  companionStartLocks: "The hour ends early if the Vault locks, so keeping the tool running means keeping your Vault Unlocked.",
  companionStop: "Stop",
  companionErrors: {
    plan_missing: "No plan file is added. Use Set up… to choose it.",
    plan_invalid: "The plan file isn't a correct version-2 plan. Prepare a new one.",
    wrong_identity: "The plan file names a different macOS owner.",
    code_unprotected: "Another Mac user could change the helper, so it wasn't started.",
    agent_unverified: "The agent's Mac user couldn't be confirmed as Standard, not an admin.",
    credential_missing: "No Agent Key file is added. Use Set up… to choose it.",
    credential_invalid: "The Agent Key file must hold one line: OPENAI_AGENT_API_KEY= followed by the key. Make it again with the command in docs/agent-companion.md, then use Set up… again.",
    handoff_unavailable: "The private agent folder isn't set up correctly. See Setup steps below.",
    port_unavailable: "Another program is using the Agent Companion's local port, 8789. Quit that program, then press Start again.",
    grant_inactive: "The one hour couldn't start. Press Start again.",
    control_invalid: HELPER_STOPPED_UNEXPECTEDLY,
    runtime_missing: "A part of the helper is missing from this app. Install the app again from a copy you trust.",
    policy_mismatch: "The helper's rules don't match this app, so it wasn't started. This copy may be damaged or changed. Install the app again from a copy you trust.",
    launch_failed: "The helper couldn't start. Press Start again.",
    start_timeout: "The helper took too long to start, so it was stopped. Press Start again.",
    unexpected_exit: "The helper stopped unexpectedly. It won't restart by itself. Press Start if you still need it.",
    protocol_invalid: HELPER_STOPPED_UNEXPECTEDLY,
    busy: "Stop the Agent Companion before changing its setup.",
    install_invalid: "Those files couldn't be added. Choose the plan file and the Agent Key file again.",
    helper_restarted: "The helper stopped unexpectedly. It won't restart by itself. Press Start if you still need it.",
    helper_timeout: "The helper didn't answer in time. Wait a moment, then try again. If this keeps happening, quit the app and open it again.",
    helper_unreachable: "The app couldn't reach the helper. Quit the app and open it again.",
    invalid_reply: "The app didn't understand the helper's reply. Quit the app and open it again. If this happens again, install the app again from a copy you trust.",
    internal: "Something went wrong in the helper. Quit the app and open it again.",
  } satisfies Record<string, string>,

  // ---------- An agent's request (ADR-0012) ----------
  // Titles and details here were written by the agent: they are shown as plain text, in quotes.
  agentAskedStore: `${AGENT} asked to store this key. Type the value and choose Save.`,
  agentAskedReplace: `${AGENT} asked to replace this value. Type the new value and choose Save.`,
  // Under that line: what the agent will and won't get.
  // Store: the name and details the owner types here are what the agent's list_keys returns.
  agentNeverSeesValue: "The agent never sees the value you type. It does see the name and details saved here, and they reach its AI company. It hears only that the key was saved, or that you cancelled.",
  // Replace: only the value is shown, so all of it stays out of the agent's reach.
  agentNeverSees: "The agent never sees what you type. It hears only that the key was saved, or that you cancelled.",
  agentReplaceTitle: (title: string) => `Replace a value of “${title}”`,
  agentTypeValue: "Type the new value to save.",
  agentRequestEnded: "The agent's request has ended, so this can't be saved for it any more. Nothing was changed.",
  agentWaiting: {
    store: (title: string, kind: string) => `${AGENT} is asking to store “${title}” (${kind}).`,
    edit: (title: string) => `${AGENT} is asking to change “${title}”.`,
    replace: (title: string) => `${AGENT} is asking to replace a value of “${title}”.`,
    delete: (title: string) => `${AGENT} is asking to delete “${title}”.`,
  },
  agentOpen: "Open",
  agentDecline: "Cancel the request",
  agentEditTitle: "Allow this change?",
  agentEditIntro: (title: string) => `${AGENT} asked to change “${title}”:`,
  // One line per field. Empty values are added or cleared, not changed.
  agentChange: (field: string, from: string, to: string) =>
    !from ? `${field}: add “${to}”` : !to ? `${field}: clear “${from}”` : `${field}: from “${from}” to “${to}”`,
  agentEditWhy: "Check each one. A wrong website, username, address or chain could send you, or your money, somewhere else.",
  agentEditPassword: "Seed Phrases and Private Keys need your Master Password for this, next.",
  agentAllow: "Allow",
  agentEditPasswordText: "Enter your Master Password to make the change the agent asked for.",
  agentDeleteTitle: (title: string) => `Delete “${title}”?`,
  agentDeleteText: (title: string) => `${AGENT} asked to delete “${title}”. It will go to Trash for 30 days. Your Master Password is needed next.`,
  agentDeletePasswordText: "Enter your Master Password to move this key to Trash, as the agent asked. It stays there for 30 days.",
  agentRequestGone: "The agent's request ended before you answered, so nothing was changed.",

  // ---------- Agents storing keys (ADR-0012) ----------
  agentKeysTitle: "Agents storing keys",
  agentKeysSummary: (state: string) => `Agents storing keys (optional) · ${state}`,
  agentKeysChecking: "Checking…",
  agentKeysIntro:
    "Optional, and off unless you start it. While it is on, an agent in a separate Mac user can ask this app to store a key, list and change the details of the keys it added, and ask you to replace or delete one. You type every value here, and the agent never sees or uses one. It can't see your own Notes. This is separate from the Agent Companion.",
  agentKeysLeaves: "The names and visible details of the keys an agent adds reach that agent's AI company as part of the chat. Name them with that in mind.",
  agentKeysStates: {
    off: "Off",
    running: "On",
    ended_by_lock: "Off: ended because the Vault locked",
    expired: "Off: the one hour ended",
  } satisfies Record<string, string>,
  agentKeysTimeLeft: (minutes: number) =>
    `${plural(minutes, "minute", "minutes")} left at most. It stops sooner if the Vault locks, the computer sleeps or the app quits.`,
  agentAccount: "The agent's Mac user",
  agentAccountHint: "Its Account name from System Settings → Users & Groups, such as enote-agent. A Standard user, never yours or an admin. Changing it stops the agent's access.",
  agentAccountSave: "Save",
  agentAccountSaved: "Saved.",
  agentKeysSetup:
    "Before you start: make that Standard user, run scripts/agent-keys-setup.sh with its name from your own Mac user, and add the tool it prints to the agent there.",
  agentKeysStart: "Start for up to one hour",
  agentKeysStartLocks: "Start asks you again in its own window. The hour ends early if the Vault locks.",
  agentKeysStop: "Stop",
};
