# encrypted-note

A personal desktop app (Mac and Windows) for keeping secrets encrypted and stored only on the owner's own computers. Nothing is sent to any server.

## Language

### Storage

**Vault**:
The single encrypted file that holds all of the owner's **Notes**; the same file opens on Mac and Windows.
_Avoid_: database, store, safe, wallet

**Backup**:
A complete copy of the **Vault** file the owner saved outside the app (e.g. on a USB stick) with "Back up now"; it is itself a **Vault** and opens with the same **Master Password** or **Recovery Key**.
_Avoid_: export, snapshot, sync

**Safety Copy**:
An earlier version of this computer's **Vault** that the app keeps automatically (the 10 most recent saves, the first save of each day for 30 days, and whenever opening a **Backup** replaced it), so a damaged or mistakenly replaced **Vault** can be restored.
_Avoid_: old vault, previous version, auto-backup, snapshot

**Emergency Reader**:
A small separate terminal tool that opens a **Vault** or **Backup** with the **Master Password** or **Recovery Key** and prints its **Notes** to the screen, for when the app itself no longer works.
_Avoid_: decrypt script, export tool, recovery tool

### Access

**Master Password**:
The password the owner chooses and types to open the **Vault** day to day.
_Avoid_: passcode, PIN, login password, vault password

**Recovery Key**:
A long random code generated once at setup that can also open the **Vault** and set a new **Master Password**.
_Avoid_: backup code, secret key, reset code

**Key Rotation**:
Replacing the **Vault**'s internal keys and its **Recovery Key** (so the owner writes a new **Recovery Kit**), used when the old **Master Password** or **Recovery Kit** may be known to someone else.
_Avoid_: reset, re-key, re-encrypt

**Locked** / **Unlocked**:
The two states of an open app: **Unlocked** means the **Vault** can be read; **Locked** means the app has dropped its open Vault and decrypted Notes. Native **Master Password** or **Recovery Key** entry opens it again; previously enabled **Quick Unlock** may also open it under its separate local rules.
_Avoid_: logged in / logged out, open / closed

**Auto-lock**:
The owner-chosen rule for when an **Unlocked** app becomes **Locked** by itself (default: 5 minutes idle, and instantly on sleep, screen lock or lid close). Idle means no use of the app's own window. On a Mac the owner can choose to count keyboard and mouse use anywhere on the computer as well; the app then still becomes **Locked** no later than 60 minutes after its own window was last used.
_Avoid_: timeout, session expiry

**Quick Unlock**:
Opening a **Locked** app with Touch ID on a supported Mac after explicit enrollment. The local source implementation uses a device-bound Secure Enclave key and an encrypted private file; no account, server or Apple Developer profile is part of this route. Native **Master Password** verification is still required after every app restart and every 7 days; Touch ID never extends that deadline or opens **Wallet Kinds**. Physical Touch ID acceptance remains pending. Windows Hello remains later.
_Avoid_: biometric login, fingerprint login

**Recovery Kit**:
The sheet (printed or handwritten) the owner keeps somewhere physical, carrying the **Recovery Key**.
_Avoid_: emergency kit, backup sheet

### Working with AI agents

**Agent Key**:
A separate credential the owner creates at a service only for AI agent work, with narrow provider-enforced permissions, expiry, revocation and hard spending/usage caps. It is never a copy of an owner's existing key and never enters the **Vault**. This boundary is approved; no provider or pilot is selected ([ADR-0007](docs/adr/0007-agents-use-separate-restricted-keys.md)).
_Avoid_: sharing an owner key, agent access to the Vault

### Screen privacy

**Capture Hiding**:
The operating-system switch that asks for the app's window to be left out of screenshots, recordings and screen shares. Much more reliable on Windows than on Mac, but a guarantee on neither.
_Avoid_: content protection, invisibility, stealth mode

**Sharing Guard**:
The app's own layer that covers its whole window with a shield and blocks Show while a known screen-sharing or recording app is running; the owner may dismiss it.
_Avoid_: privacy mode, screen shield, capture detection

### What's inside

**Note**:
One stored thing in the **Vault**, whatever its **Kind**.
_Avoid_: secret, item, entry, record

**Kind**:
The shape of a **Note**, which decides its form and how it's displayed: Seed Phrase, Private Key, Login, API Key, or Text.
_Avoid_: type, category, template

**Hidden Field**:
A part of a **Note** shown as dots or blur until the owner clicks Show; it hides again after 30 seconds, or another 30 each time the owner asks for more time while it is shown, or when another **Note** is opened (**Wallet Kinds** are stricter: see there).
_Avoid_: secret field, masked field, protected field

**Seed Phrase**:
A **Kind** of **Note** holding a 12- or 24-word wallet recovery phrase, shown as a numbered word grid. It can also carry the wallet's name, one of its public addresses and the derivation path that address is found at, which all stay visible.
_Avoid_: mnemonic, recovery phrase, secret phrase

**Passphrase**:
The optional extra word ("25th word") that some wallets add to a **Seed Phrase**.
_Avoid_: using "passphrase" for the **Master Password**

**Private Key**:
A **Kind** of **Note** holding one wallet/account private key, with the chain it belongs to and its public address.

**Wallet Kinds**:
**Seed Phrase** and **Private Key**, the **Kinds** that can move funds directly. Their **Hidden Fields** stay encrypted even while the app is **Unlocked**. Showing, adding or editing them needs the **Master Password** again in a native panel; stored Wallet readouts stay native too. Shown values stay visible while the Show button or Space key is held, or for 20 seconds when the timed reveal option is chosen; those 20 seconds can be given again three times, after which showing the value takes the **Master Password** once more.
_Avoid_: sensitive kinds, crypto notes

**Login**:
A **Kind** of **Note** holding a website, a username and a password, and optionally a line saying what it is for (its purpose).
_Avoid_: account, credential, password entry

**API Key**:
A **Kind** of **Note** holding a service name and its key or token string. It can also carry the rest of what a service hands out with a key: the key ID (the half that isn't hidden), the secret (a second **Hidden Field**, for services that issue two), its purpose, where it is managed, the day it expires and the variable name the owner uses for it.
_Avoid_: token, credential

**Text**:
The free-form **Kind** of **Note** for anything that doesn't fit the others.
_Avoid_: plain note, memo (never call this Kind just "Note")

**Trash**:
Where a deleted **Note** waits for 30 days, restorable, before it is removed from the **Vault** for good. It goes at an unlock after the 30 days, and only once two unlocks at least a day apart have both found it overdue, so a wrong date on the computer for one session removes nothing.
_Avoid_: bin, recycle bin, archive

**Favorite**:
A **Note** the owner has starred so it's pinned at the top of the list.
_Avoid_: pinned, bookmarked, starred item

## Relationships

- The owner has one **Vault**; it moves between computers only when the owner copies the file by hand.
- AI agents have no access to the **Vault**, its **Notes**, their metadata, **Backups**, **Safety Copies**, or the **Emergency Reader**. The **Master Password**, **Recovery Key** and **Wallet Kinds** never take part in agent work.
- Any future **Agent Key** integration is separate from this offline app. Hiding the key still lets an agent perform every operation its provider scope permits, within its enforced limits; only the owner can independently approve scope or limit changes ([ADR-0007](docs/adr/0007-agents-use-separate-restricted-keys.md)).
- Results from an external agent's allowed service actions may leave this computer, including for a cloud model provider; no Vault content or metadata may enter that route.
- Moving a **Vault** to the other computer means opening a **Backup** there. There is no separate "export" or "sync" concept.
- The app nudges the owner when changes have gone 7+ days without a **Backup**, and warns if a **Backup** is saved into a cloud-synced folder.
- Copying a **Vault** replaces the copy on the other computer; the two are never merged.
- Opening a **Backup** replaces this computer's **Vault**, which becomes a **Safety Copy**; if the **Backup** is older, or the two were changed separately, the owner is warned first, in plain words. When the app is **Locked** and can't compare them, it says so instead.
- A **Vault** holds zero or more **Notes**; each **Note** has exactly one **Kind**.
- Every **Note** has a title, which is never a **Hidden Field**.
- **Hidden Fields** by **Kind**: the words and **Passphrase** of a **Seed Phrase** (wallet name, address and derivation path stay visible); the key of a **Private Key** (chain and address stay visible); the password of a **Login** (website, username and purpose stay visible); the key and the secret of an **API Key** (service name, key ID, purpose, where it is managed, expiry date and variable name stay visible); the whole body of a **Text**.
- A **Text** can have text added to its end without showing what is already there.
- While the **Sharing Guard** is up, no **Hidden Field** can be shown, and the window shows only the shield (no titles either).
- A copied value is cleared from the clipboard after 30 seconds, or 60 if the owner chooses that; a **Wallet Kind** value always after 30. The visible parts of a **Note** can be copied too, and that never needs the **Master Password**.
- **Capture Hiding** is always on; the **Sharing Guard** exists because **Capture Hiding** alone can't be trusted on Mac.
- Search matches only the visible parts of a **Note** (title, website, username, purpose, wallet name, chain, address, derivation path, service name, key ID, where it is managed, expiry date, variable name), never its **Hidden Fields**. Several words can be typed: a **Note** is found when each word is somewhere in those parts, in any order, and a **Private Key** is also found by the name of its chain (such as "ethereum").
- The **Vault** file has only **Master Password** and **Recovery Key** credential slots. Enabled **Quick Unlock** opens this computer's current Vault with a separately protected **Vault Key**, without adding a credential slot or releasing the **Wallet Key**.
- A **Vault** has exactly one **Recovery Key**, shown to the owner once, at setup, to write onto their **Recovery Kit**.
- **Safety Copies** stay on this computer; a **Backup** is the owner's copy somewhere else. Both are complete **Vaults**.
- The **Emergency Reader** never writes **Notes** to disk; it only shows them.
- A **Vault** that uses any of the newer optional parts of a **Note** (the address or derivation path of a **Seed Phrase**; the purpose of a **Login**; the key ID, secret, purpose, where it is managed, expiry date or variable name of an **API Key**), even on a **Note** in **Trash**, opens only in a current app and a current **Emergency Reader**. Older ones refuse the whole file, and some of them call it damaged, so the app on the other computer and any saved **Emergency Reader** must be updated first, before such a **Vault** is moved there.
- A **Note** in **Trash** is still inside the **Vault** (still encrypted); "Delete forever" removes it from this copy only; older copies still contain it.
- Removing a **Wallet Kind** needs the **Master Password**, like every other change to one: moving it to **Trash**, "Delete forever", and emptying a **Trash** that holds one.
- Changing the **Master Password** changes it only in the copy of the **Vault** where it was changed; older copies still open with the old **Master Password** and old **Recovery Key**.
- Changing the **Master Password** keeps the same **Recovery Key**; only **Key Rotation** replaces it.

## Example dialogue

> **Dev:** "If I add a **Note** on the Mac, does it show up on the Windows PC?"
> **Owner:** "Only after I copy the **Vault** file over. Whatever copy I open last is the truth; nothing syncs on its own."
> **Dev:** "And the free-form one, is that just a 'note'?"
> **Owner:** "Everything is a **Note**. The free-form one is a **Text** **Note**."

## Flagged ambiguities

- "note" was used both for any stored thing and for free-form text. Resolved: every stored thing is a **Note**; the free-form **Kind** is **Text**.
- "wallet" never means the **Vault**; it only appears in **Wallet Kinds** and in wallet names inside a **Seed Phrase**.
- "passphrase" could mean the **Master Password** or a wallet's 25th word. Resolved: **Passphrase** is only ever the **Seed Phrase** extra word.
- "invisible to screen sharing" was used as a guarantee. Resolved: no platform guarantees it. Windows **Capture Hiding** is strong, Mac's is best-effort, and both are layered with **Hidden Fields** + **Sharing Guard** (ADR-0002).
- "secret" is used loosely for sensitive values. It is not a domain term; say **Note**, or name the field (the password of a **Login**). The one field with that name is the secret of an **API Key**, its second **Hidden Field**.
