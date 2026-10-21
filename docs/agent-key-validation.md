# Isolated Agent Key workflow validation

Status: ready-for-human — research only; no workflow has run. Refs [#27](https://github.com/nniiovoo/encrypted-note/issues/27).

The boundary is separate restricted Agent Keys and absolutely no agent access to the owner's Vault, Notes, Master Password, Recovery Key, Backups or Safety Copies ([#25](https://github.com/nniiovoo/encrypted-note/issues/25)). This record concerns an external sandbox only. It adds no interface, network code or dependency to encrypted-note ([ADR-0001](adr/0001-local-only-vault-moved-by-hand.md)). Account/execution isolation must be verified first ([#26](https://github.com/nniiovoo/encrypted-note/issues/26)).

## Official-source preflight, 2026-10-21

The research target is **local Docker Sandboxes, sbx 0.49.1**, rather than a cloud sandbox. No installed binary, daemon, agent image, kit, setting or network behavior has been checked. Recheck the official documentation and exact installed command help before a scoped run; current web documentation is not proof of a particular binary's behavior. The older ignored draft is not installation or test evidence.

| Topic | Documentation finding and source | Still to verify locally |
|---|---|---|
| Exact release | Docker lists stable 0.49.1, released 2026-10-20 ([release notes](https://docs.docker.com/ai/sandboxes/release-notes/)) | Exact binary version/source, integrity, daemon and agent/kit versions |
| Supported Mac | Docker lists macOS 14+ on Apple silicon and Docker sign-in ([installation requirements](https://docs.docker.com/ai/sandboxes/install/)) | Test hardware support; any tool-account sign-in is a separate owner action |
| Credential scope | Service values are held on the host; the proxy substitutes authentication headers. Service secrets are global by default; sandbox scope must be chosen explicitly ([credentials](https://docs.docker.com/ai/sandboxes/configuration/credentials/), [secret-set reference](https://docs.docker.com/reference/cli/sbx/secret/set/)) | Fake marker never readable in guest; only intended sandbox receives injection; no global fallback |
| Transport and mounts | Forward-proxy and transparent-proxy routes enforce policy, but only the forward proxy injects credentials. Direct mounts expose host files, and sandbox processes can write host clipboard text ([isolation layers](https://docs.docker.com/ai/sandboxes/security/isolation/)) | Exact route and destination policy, TLS handling, mount inventory and host bridges for this run |
| CLI telemetry | Docker documents command/result/duration analytics, including Docker username when signed in; `SBX_NO_TELEMETRY=1` opts out. Docker authentication and registry traffic are separate ([FAQ](https://docs.docker.com/ai/sandboxes/faq/)) | Effective CLI/daemon environment and observed outbound destinations; opt-out does not prove zero network traffic |
| Other host controls | Settings expose SSH-agent forwarding, host-browser opening, clipboard image access and automatic diagnostic-upload consent. Declining automatic diagnostics does not prohibit an explicit upload ([settings](https://docs.docker.com/ai/sandboxes/configuration/settings/)) | Effective values and restart requirements; SSH forwarding/browser bridges off, image reads off, automatic uploads declined; no explicit diagnostic upload |

Docker, its host proxy/daemon, the host account, hypervisor, selected agent/kit and allowed destination become part of the trusted setup. An agent in the same host account is outside the guest boundary. A domain allowlist limits destinations, not allowed actions at those destinations. These are reasons to test the selected setup, not claims that it is safe for real keys.

## Scope before execution

Keep an owner-approved operational record privately identifying the exact tool/version, account boundary, sandbox names, fake fixtures, permitted destinations, tools, expected changes and cleanup. This document authorizes no installation, account change, sign-in, provider-account modification, custom proxy, credential import, or real-key pilot. If no fake-only supported route is available, leave the relevant check pending.

Use one obviously fake marker (for example `FAKE-AGENT-KEY-VALIDATION-ONLY`) through the tool's interactive host entry. Do not copy from encrypted-note or use real values in arguments, environment dumps, shell history, chat or logs. No OAuth credential, dynamic host-secret resolver, SSH agent, MCP gateway, real service login, broad host mount or global secret belongs to this test. Keep shared skills and any required fixture mounts within the explicitly approved fake-only scope.

Before starting an agent, inspect the effective policy and all kit-added grants. Allow only the approved exact HTTPS test destination in the selected sandbox; inspect TLS trust and proxy bypass behavior. Test a second sandbox with no credential grant as a negative control. Do not widen policy to make a failing test succeed. A positive injection control needs an already-approved test endpoint that validates the fake header without returning it; do not deploy a new proxy or send the marker to an unreviewed echo service.

## Fake-only checks

| Check | Procedure and required observation | Result |
|---|---|---|
| Credential entry | Confirm fake entry is interactive and host-side; inspect only fake test history/logs for accidental echoes | pending |
| Positive injection control | An approved fixture accepts the fake injected header from the intended sandbox without revealing its value | pending |
| Guest non-disclosure | Search only the test guest's environment, files, process arguments and output for the exact fake marker; guest sees only the tool's placeholder | pending |
| Sandbox scope | The ungranted second sandbox cannot authenticate to that fixture; no global secret supplies a fallback | pending |
| Destination restriction | A designated unallowed HTTPS destination and a redirect to it are blocked; no marker is sent there | pending |
| Host boundary | Only approved fake fixtures are mounted; guest cannot read designated fake host-only files or use SSH, host-browser, clipboard-image or MCP bridges | pending |
| Errors and logs | Force a fake authentication error and inspect guest output plus approved host/proxy/daemon logs; no marker, header value or credential-bearing URL appears | pending |
| Response echo | With an approved fake fixture only, test whether headers or credential-derived values can return in a response; any marker reaching the guest fails non-disclosure | pending |
| Remove/revoke | After a successful fake positive control, remove its scoped host entry and repeat in the existing sandbox; authentication must stop without a restart loophole | pending |
| Expiry | If the fixture models expiry, prove an accepted fake value is rejected after expiry; label this a fixture result, not provider enforcement | pending |
| Policy and telemetry | Record effective scope, destination/TLS settings, host-bridge settings, telemetry opt-out, diagnostic consent and observed network destinations; unexplained traffic remains unresolved | pending |
| Cleanup | Remove only the scoped fake credentials, fake sandboxes and approved fixtures; verify other sandboxes and host settings are unaffected | pending |

An invalid fake key returning 401 before and after removal proves neither correct injection nor revocation. Without a successful positive control, leave those results pending. A fake fixture cannot establish a real provider's scopes, hard spending limits, expiry or revocation delay. Those require current official provider evidence and a separately owner-scoped later pilot; do not create or change a provider key for this record.

## Evidence and decision

Publish only dates, exact tool/OS/agent/kit versions, fake fixture labels, effective control summaries, sanitized outcomes and issue links. Keep account names, private paths, raw settings inventories, authentication transcripts, real identifiers/values, headers and raw logs out of the repository and GitHub. Record expected and observed results separately.

```text
Run date / private operational scope approval date:
Exact sbx, daemon, OS, agent image and kit versions:
Credential scope / approved fixture label / policy summary (no private paths):
Check / expected result / actual result / pass, fail or pending:
Positive and negative control results:
Provider controls: researched only, not validated (official source links):
Residual permissions and limits / follow-up issue:
Fake fixture and credential cleanup: complete or pending:
```

Every runtime result is pending. Any marker returned to the agent, unscoped injection or unexplained route blocks a real-key recommendation. Even successful isolation leaves an agent able to misuse every allowed operation and send data through an allowed service, up to provider-enforced limits. A later real-key pilot needs its own scope and provider-enforced restrictions; this checklist neither grants that permission nor makes any owner Vault accessible.
