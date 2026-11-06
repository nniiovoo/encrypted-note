# Product

## Register

product

## Users

An individual owner keeping personal secrets on their own Mac or Windows computer. The owner needs to find a Note, deliberately reveal a Hidden Field, save changes, make a Backup, or recover access without a server account. Use the established names in [CONTEXT.md](CONTEXT.md) and explain actions in plain language.

The optional Agent Companion serves a separate workflow: an owner gives tools in a distinct standard Mac account one restricted OpenAI text operation using a separately created Agent Key. An agent has no app interface to the Vault. The first private Agent Key entry flow is Mac only.

Planned, accepted, not built: agents that store and manage keys ([ADR-0012](docs/adr/0012-agents-store-and-manage-keys.md)). From Claude or Codex, the owner asks for a key to be stored; the app opens its own native window, and the owner types the value there. The agent gets back only "saved", can list and edit the details of only the keys it added, and never sees or uses a value. Replacing a value or deleting a key happens in the app's own window too.

## Product Purpose

Keep Notes encrypted locally and make access, locking, saving and recovery understandable. The Vault and its WebView remain offline. The optional companion's fixed network operation belongs to its separate sandbox and never receives Vault content or metadata.

Success means the owner can tell what an action will do before choosing it, what actually happened afterward, and how to cancel or recover. A local readiness check, an encrypted save, a running grant, provider restrictions and a working ChatGPT connection are different facts; the interface must identify each accurately.

## Brand Personality

Calm, direct, careful. Preserve the existing desktop utility's familiar system controls, concise labels and restrained Settings hierarchy. Use functional explanations and specific limitations instead of promotional language or absolute security claims. The existing app is the visual reference; no external brand reference has been supplied.

## Anti-references

- Marketing heroes, decorative photographs, repeated feature-card grids and oversized display typography inside the app.
- Decorative glass, gradients, pulsing requests, animated urgency and celebratory effects around sensitive actions.
- Cryptic technical errors, unexplained disabled controls, vague success messages and claims of protection that have not been observed.
- Custom controls that make ordinary keyboard, focus or cancellation behavior unfamiliar.

## Design Principles

1. **Keep the owner in control.** A request may produce a fixed notice. Opening private entry, saving or replacing a key, and starting a companion grant each require their own appropriate local action. Do not steal focus or start work automatically after saving.
2. **Make state truthful.** Show waiting, progress, cancellation, failure and completion in words. Distinguish local storage from provider validity and connection readiness. Report unverified behavior as unverified.
3. **Keep sensitive entry native.** An Agent Key belongs in the dedicated native secure control, never in chat or a WebView form. Status, errors, help and accessibility descriptions must not disclose its value or value-derived metadata. Vault credentials and content remain outside the companion workflow.
4. **Preserve familiar interaction.** Reuse the existing system font, semantic colors, grouped Settings rhythm and native controls. Keep active status and Stop visible, preserve focus across updates and keep the action hierarchy compact.
5. **Make interruption safe and understandable.** Cancel and Escape are ordinary outcomes. Focus loss and lifecycle changes invalidate private work. A late callback cannot revive a cancelled action, replace another request or restart a grant.

## Accessibility & Inclusion

Support keyboard-only use, visible focus, descriptive control labels and meaningful screen-reader status. Keep private input's native secure-control semantics without creating a plaintext mirror. Announce state changes without repeating unchanged polling results or moving focus.

Target at least 4.5:1 contrast for normal text, including placeholders and button labels, and 3:1 for focus and required non-text control indicators. Check actual supported light, dark, accent and Increase Contrast settings; system colors alone are not evidence of a passed check. Use words alongside semantic color. Respect reduced motion and keep actions reachable at the minimum supported window size and larger text settings.

These are implementation and acceptance requirements, not a claim of completed accessibility certification. Native authentication, secure accessibility, capture behavior and focus transitions need fictional-data physical checks in addition to source and Web fixture checks.

## Context and Current Scope

This document records existing product direction from the README, CONTEXT, native credential design, committed styles and the owner's approved private Agent Key design. It does not change the security policy in AGENTS.md or the ADRs.

The approved private entry flow is still being implemented. Existing plaintext companion setup must not be described as encrypted custody. New encrypted readiness and ChatGPT completion copy may ship only with the corresponding implementation and evidence. Source-based UI guidance and acceptance details are recorded locally in `.scratch/private-agent-key-entry-20261104/UI-AUDIT.md`.
