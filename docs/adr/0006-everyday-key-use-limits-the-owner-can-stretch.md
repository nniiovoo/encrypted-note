# Everyday key use: a few limits the owner can stretch, and the ones that stay fixed

The first version treated every time limit as a constant: 30 seconds on the clipboard, 30 seconds for a shown **Hidden Field**, 20 seconds for a **Wallet Kind**, **Auto-lock** counted from the last click inside the window. Using the app daily for **API Keys** and **Logins** showed where that cost more than it protected: a copy expired before it could be pasted behind a sign-in page, the app **Locked** while the owner was typing in a terminal next to it, reading out a **Seed Phrase** meant the **Master Password** every 20 seconds, and a half-typed **Note** vanished when a recording app merely started. The owner asked for all of these to be eased (October 2026). We eased them only where the change is opt-in or bounded, and kept the rules that ADR-0003 and ADR-0004 rest on.

What changed:

- **Clipboard time is 30 or 60 seconds**, the owner's choice in Settings, default 30. A **Wallet Kind** value is always cleared after 30. With "Lock when I switch to another app" on, that lock leaves the pending clear on its timer so a copy can be pasted; every other lock still clears at once.
- **A shown value can be given more time while it is still shown.** An ordinary **Hidden Field** gets another 30 seconds as often as asked. A **Wallet Kind** value gets another 20 seconds at most three times; after that it takes the **Master Password** again. A value that has been hidden is never brought back this way.
- **Auto-lock can count activity anywhere on the computer** (Mac only, off unless the owner turns it on). The app then stays **Unlocked** while the computer is in use, but still **Locks** no later than 60 minutes after the last activity inside its own window, and at once on sleep, screen lock or user switch. The 60-minute limit lives in the session rules, is a constant, and is not moved by activity elsewhere. A held-down key is not activity.
- **One Master Password entry can show and copy a Wallet Kind value.** It is still one freshly derived Wallet Key, for one field of one **Note**, dropped before the call returns (ADR-0004 is unchanged).
- **The visible parts of a Note can be copied**, including the address of a **Wallet Kind**, without the **Master Password**. They were already readable on screen while **Unlocked**.
- **An unsaved Note survives the Sharing Guard shield**, in memory only, and is dropped on any lock. The words of a **Seed Phrase**, a **Private Key**'s key and a **Passphrase** are never kept; they must be typed again.
- **The owner's place survives a lock**: which filter was selected and which **Note** was open, as an id. No title, no search text, no value.
- **A Text can be added to without showing it**: the new lines are joined to the old text inside the Rust core.
- **API Key, Login and Seed Phrase gained optional parts** (FORMAT.md, schema 2). The visible ones (purpose, key ID, where it is managed, expiry date, variable name, address, derivation path) are visible on purpose: shown without Show and found by search.

## Consequences

- A 60-second clipboard and an extended reveal are longer exposures. Both happen only after an explicit choice by the owner, and the defaults are the old values.
- "Count activity anywhere" means someone else using the computer keeps the app **Unlocked** for up to 60 minutes after the owner last touched it. The Settings text says so, and the status line shows when it is on. Like the idle time itself, it can be switched from the WebView without the **Master Password**; that is an accepted limit of the current design, not something this option fixes.
- While the shield is up, a typed but unsaved **Login** password, **API Key** or **Text** stays in WebView memory. Before this it was discarded the moment the shield appeared. Nothing is drawn or listening behind the shield.
- A **Vault** that uses any schema-2 part opens only in a current app and a current **Emergency Reader**. Builds from before 12 October 2026 call it damaged. A **Vault** that uses none of them is still written as schema 1.
- Anything typed into a visible part is not protected like a **Hidden Field**. The Editor warns when a visible box holds something shaped like a key or a **Seed Phrase**.
- The management address of an **API Key** is text to copy. The app never opens it (ADR-0001).

## Considered Options

- **Keep one Note's decrypted Wallet Kind fields for a short "visit" so several actions need one Master Password**: rejected. It is the state ADR-0004 exists to prevent; show-and-copy and the bounded extension cover most of the need.
- **Load the old text into the Editor to change a Text in place**: rejected. Editing would have to reveal a **Hidden Field** with no time limit. Adding to the end and replacing are offered instead.
- **Keep an unsaved Note across a lock**: rejected. **Locked** means the app has forgotten what it held.
- **Let a Sharing Guard dismissal survive a lock**: rejected. "I'm not sharing right now" is a statement about now. The dismiss button is focused after an unlock instead, so it costs one key press.
- **Keep the previous value when a key is replaced**: rejected. It would keep a possibly exposed key alive inside the **Vault**; **Safety Copies** already cover a wrong replace.
- **A box for pasting many keys at once, or importing a `.env` file**: rejected as an import feature. "Save and add another" covers entering several by hand.
- **A global hot key or a menu-bar window**: rejected. It needs a new dependency, and a second window would sit outside Capture Hiding (ADR-0002).
- **Quick Unlock** would remove the largest daily cost, typing the **Master Password** after every lock. It stays planned (ADR-0003 names the allowed designs) because it needs a signed build and hardware to test on.
