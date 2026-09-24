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
An earlier version of this computer's **Vault** that the app keeps automatically (recent saves, and whenever opening a **Backup** replaced it), so a damaged or mistakenly replaced **Vault** can be restored.
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
The two states of an open app: **Unlocked** means the **Vault** can be read; **Locked** means the app has forgotten how to read it until the **Master Password** or **Recovery Key** is entered again.
_Avoid_: logged in / logged out, open / closed

**Auto-lock**:
The owner-chosen rule for when an **Unlocked** app becomes **Locked** by itself (default: 5 minutes idle, and instantly on sleep, screen lock or lid close).
_Avoid_: timeout, session expiry

**Quick Unlock**:
Opening a **Locked** app with Touch ID (Mac) or Windows Hello instead of typing the **Master Password**; the **Master Password** is still required after every restart and every 7 days. (Planned for v1.1.)
_Avoid_: biometric login, fingerprint login

**Recovery Kit**:
The sheet (printed or handwritten) the owner keeps somewhere physical, carrying the **Recovery Key**.
_Avoid_: emergency kit, backup sheet

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
A part of a **Note** shown as dots or blur until the owner clicks Show; it hides again after 30 seconds or when another **Note** is opened (**Wallet Kinds** are stricter: see there).
_Avoid_: secret field, masked field, protected field

**Seed Phrase**:
A **Kind** of **Note** holding a 12- or 24-word wallet recovery phrase, shown as a numbered word grid.
_Avoid_: mnemonic, recovery phrase, secret phrase

**Passphrase**:
The optional extra word ("25th word") that some wallets add to a **Seed Phrase**.
_Avoid_: using "passphrase" for the **Master Password**

**Private Key**:
A **Kind** of **Note** holding one wallet/account private key, with the chain it belongs to and its public address.

**Wallet Kinds**:
**Seed Phrase** and **Private Key**, the **Kinds** that can move funds directly. Their **Hidden Fields** stay encrypted even while the app is **Unlocked**. Showing, adding or editing them needs the **Master Password** again, and shown values stay visible only while the Show button is held.
_Avoid_: sensitive kinds, crypto notes

**Login**:
A **Kind** of **Note** holding a website, a username and a password.
_Avoid_: account, credential, password entry

**API Key**:
A **Kind** of **Note** holding a service name and its key or token string.
_Avoid_: token, credential

**Text**:
The free-form **Kind** of **Note** for anything that doesn't fit the others.
_Avoid_: plain note, memo (never call this Kind just "Note")

**Trash**:
Where a deleted **Note** waits for 30 days, restorable, before it is removed from the **Vault** for good.
_Avoid_: bin, recycle bin, archive

**Favorite**:
A **Note** the owner has starred so it's pinned at the top of the list.
_Avoid_: pinned, bookmarked, starred item

## Relationships

- The owner has one **Vault**; it moves between computers only when the owner copies the file by hand.
- Moving a **Vault** to the other computer means opening a **Backup** there. There is no separate "export" or "sync" concept.
- The app nudges the owner when changes have gone 7+ days without a **Backup**, and warns if a **Backup** is saved into a cloud-synced folder.
- Copying a **Vault** replaces the copy on the other computer; the two are never merged.
- Opening a **Backup** replaces this computer's **Vault**, which becomes a **Safety Copy**; if the **Backup** is older, the owner is warned first, in plain words.
- A **Vault** holds zero or more **Notes**; each **Note** has exactly one **Kind**.
- Every **Note** has a title, which is never a **Hidden Field**.
- **Hidden Fields** by **Kind**: the words and **Passphrase** of a **Seed Phrase**; the key of a **Private Key** (chain and address stay visible); the password of a **Login** (website and username stay visible); the key of an **API Key** (service name stays visible); the whole body of a **Text**.
- While the **Sharing Guard** is up, no **Hidden Field** can be shown, and the window shows only the shield (no titles either).
- **Capture Hiding** is always on; the **Sharing Guard** exists because **Capture Hiding** alone can't be trusted on Mac.
- Search matches only the visible parts of a **Note** (title, website, username, wallet name, chain, address, service name), never its **Hidden Fields**.
- A **Vault** opens with either its **Master Password** or its **Recovery Key**; nothing else can open it.
- A **Vault** has exactly one **Recovery Key**, shown to the owner once, at setup, to write onto their **Recovery Kit**.
- **Safety Copies** stay on this computer; a **Backup** is the owner's copy somewhere else. Both are complete **Vaults**.
- The **Emergency Reader** never writes **Notes** to disk; it only shows them.
- A **Note** in **Trash** is still inside the **Vault** (still encrypted); "Delete forever" removes it from this copy only; older copies still contain it.
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
- "secret" is used loosely for sensitive values. It is not a domain term; say **Note**, or name the field (the password of a **Login**).
