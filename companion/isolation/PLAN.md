# Isolated local-agent setup: review package

Status: PREPARED ONLY. No account, OS setting, ACL, credential or running broker has been created or changed by this package. Agent tools running in the owner's account remain unisolated. Review these artifacts before execution.

The selected first trial is one NEW separate OpenAI Agent Key, provider-enforced one-hour expiry, and a $1 USD enforced monthly project hard cap. These are approved trial values; they are not yet applied account settings. The operation remains only generate_text with gpt-6-luna, 4096 UTF-8 input bytes and 512 output tokens.

## Required identities and paths

| Component | Execution identity and access |
| --- | --- |
| encrypted-note, owner Vault, provider administration | Existing owner account; independent owner-controlled session. Never an agent input or mounted workspace. |
| Actual agent and every local tool executor | Fresh Standard macOS account, proposed short name enote-agent; different nonadmin UID. No owner-session executor, remote owner host, owner GUI bridge or owner credentials. |
| Broker, code, policy, Node runtime and dependencies | Owner-controlled paths, with no agent write or replacement authority through POSIX permissions, ACLs or controlling ancestors. |
| Provider credential file | Secure OpenAI Platform picker plus confirmed destination and trusted helper only. New dedicated file containing OPENAI_AGENT_API_KEY; owner UID, mode 0600, no symlink/hardlink, protected ancestors. No plaintext key enters chat, argv, environment, plist, logs or repository. |
| Delegated local token | Different random token authorizing only the fixed broker operation. A fresh owner-owned file, 0600, with only the selected agent user granted read and its fresh parent granted search. This token is intentionally available to the agent; the provider key is not. |
| Agent bridge | Agent UID, agent-owned workspace. It knows only the delegated token and fixed 127.0.0.1:8789 endpoint; it can never choose a provider destination or widen broker policy. |

Separate macOS UIDs do not restrict an authenticated cloud account or a connected owner-host tool. The agent's actual tool inventory must exclude owner-session execution, cloud connectors that can reach owner material, cross-thread owner control, host-browser/clipboard/capture bridges and privilege escalation. A same-account process or a process merely launched through a one-off sudo command is not proof that the real agent is isolated.

The local Vault requires no new server login, OpenAI account, Apple Developer login or cloud connection. The new Mac account is for agent execution only.

## Owner-reviewed execution sequence

1. Review the source, pinned dependencies and fixed policy. Record hashes and versions privately. Establish trust in the launcher and interpreter before executing them: self-checking code cannot bootstrap trust in its own potentially modified source. Inspect the exact code/interpreter/dependency paths and every controlling ancestor using trusted macOS tools, including ACLs. Do not recursively change existing owner home, Vault, volume or tool permissions.
2. In an independent owner-controlled Mac session, create a NEW Standard user through System Settings → Users & Groups. This requires local administrator authorization. Do not put that password in an agent session or command. Do not reuse/convert an existing account without a new review. Leave administrator status off and do not add owner Apple Accounts, developer identities, credentials or remote-sharing access.
3. Independently review admin/sudo authorization, Remote Login, Screen Sharing, File Sharing and remote management access for that identity. The scripted sudo -n -l check detects current noninteractive authorization only; failure does not prove that password-authenticated sudoers grants are absent.
4. Copy the nonsecret plan.example.json (version 2) into the private operational record and replace null values with the actual distinct UIDs and the exact delegated-token path. For the standalone owner launcher, set grantExpiresAt to a canonical UTC instant at most one hour ahead; for encrypted-note's integrated helper leave it null, because each owner Start issues the grant. Verify the short name maps to the chosen UID. A placeholder plan deliberately cannot run, and version-1 plans are refused.
5. Prepare only new fictional fixtures with fixtures.mjs in an already protected owner directory. Run the actual agent in the new account's separate login session, with only its approved workspace and tools. Execute agent-probe.mjs through that actual tool executor, then verify unchanged fixtures in the owner session. Repeat on each proposed filesystem. The probes never target a real Vault or existing owner file.
6. Complete process, clipboard, capture/control and connected-host checks below. Fix failures only through a separately reviewed change; do not grant privacy/debugging permissions to make a denial check pass. If ownership is ignored on a volume or broad ACL changes seem necessary, stop and select a different deployment location.
7. Independently apply provider permission, model/tool denial, rate limits, one-hour expiry, $1 HARD monthly project limit and owner revocation controls. Spend alerts are insufficient. Hard-limit enforcement can slightly overshoot during propagation. The known provider hosted-shell scope gap remains explicit; raw-key custody must hold.
8. Only after boundary review, use the secure Platform picker, destination confirmation and trusted helper for the NEW Agent Key. Read no existing owner key. The selected file is parsed without eval, sourcing or environment export. The one key-paste route is write-agent-key.mjs, which refuses any value that isn't shaped like an OpenAI API key.
9. In the owner session, explicitly launch owner-launcher.mjs with the reviewed plan path, confirmed credential-file path and delegated-token-file path. These arguments are paths only. The launcher binds loopback, holds the key in owner-process memory, and stops when the plan's grant ends (at most one hour). JavaScript does not guarantee memory erasure. Alternatively, encrypted-note's integrated helper runs isolation/integrated-broker.mjs: the owner installs the plan and confirmed credential through Settings → Agent Companion → Set up…, and each Start issues a one-hour grant and a fresh delegated token in the handoff folder (encrypted-note docs/agent-companion.md).
10. In the actual agent session, configure the MCP client to launch agent-bridge.mjs with only the nonsecret plan path. No key or token value belongs in MCP configuration. Make one owner-approved fictional text request. Verify expiry, revocation and provider denial before recommending real use. Stopping the broker is not provider revocation.

No launchd plist, login item, automatic restart, sudoers rule, public listener, custom user-login service or account-creation script is installed. The broker is an explicitly started local process and can be stopped with Control-C in the owner terminal.

## Script contract

Use a trusted absolute Node executable; replace paths locally without putting actual values in repository files.

- fixtures.mjs prepare takes the reviewed plan path and an existing protected parent; it creates only a fresh enote-isolation-* directory with 0700 directories and 0600 fictional members. Its output is a nonsecret private fixture manifest, never a GitHub artifact.
- agent-probe.mjs takes the reviewed plan, copied nonsecret fixture manifest and a new private agent workspace. It checks actual UID/name, privileged groups and noninteractive sudo before touching fixtures. It then checks its own workspace positively and requires permission denial for owner read/list/write/create/rename/unlink, fake code/policy/dependency writes and an agent-created symlink into the owner fixture.
- fixtures.mjs verify-owner takes the original owner-record.json and requires unchanged fixture ownership, modes and hashes. Preserve evidence on failure. Remove only these explicitly created disposable members after owner verification; no broad recursive cleanup or permission command is provided.
- owner-launcher.mjs takes reviewed plan, confirmed credential path and delegated-token path. It requires the owner UID and the token path to match the plan. Application/SDK imports are delayed until protection checks finish, but an independent trusted preflight is still mandatory.
- write-agent-key.mjs takes one output path and reads the NEW Agent Key on stdin (`pbpaste | node write-agent-key.mjs ~/enote-agent-key.env`), never as an argument. It writes only a value shaped like an OpenAI API key (sk-, then 37 to 509 letters, digits, - or _; no spaces or line breaks). It refuses Admin keys (sk-admin-), Anthropic (sk-ant-) and OpenRouter (sk-or-) keys, and input over 4096 bytes however the reads split it; an OpenAI key copied from the Vault would still pass. It writes owner-only 0600 without following a symlink or writing through a hard link, refuses a file with an ACL allow entry (set on it or inherited from its folder, checked before and after writing), then asks Time Machine to leave the file out (tmutil addexclusion), which may fail without stopping it. No message includes the value.
- agent-bridge.mjs requires the actual selected Standard UID. It reads only the plan and the separate delegated token and exposes stdio to the local MCP client. It forwards only the one fixed bounded operation. It lists the tool even while the broker is off and reads the token again on each call, so a new Start needs no restart; with no token or no listener, a call returns one fixed error.

The delegated token may need a fresh directory under /Users/Shared if the owner home is not traversable. This exception is only for the limited local token, never the provider key. Require root-owned sticky /Users/Shared, a new owner-owned 0700 child, exactly one agent-user search ACL on that child and exactly one agent-user read ACL on its 0600 token file. Other allow ACLs fail closed. Never weaken an existing owner parent to make token sharing work. An owner-reviewed targeted token-creation/ACL step remains pending; this package executes none.

## Additional fictional checks and stop rules

| Check | Required evidence |
| --- | --- |
| Actual identity | Every actual agent tool executes as the new UID. No callable owner host, owner GUI, other account's tool daemon or remote-owner route remains. |
| Code/interpreter integrity | Agent cannot open protected deployed code/dependencies for writing or replace them through any ancestor. The fixture canaries cover only tested paths; actual deployment ACLs must also be reviewed. |
| Fake owner process | Pin PID, UID and start time of a purpose-created fictional process. From the actual agent, kill(pid, 0) must return EPERM; ESRCH or a stale PID is inconclusive. A separately reviewed task_for_pid check must fail to obtain its task port. Never inspect a real process's memory, argv or environment. |
| Clipboard | Owner first replaces its clipboard with an agreed fictional marker. Compare only that marker without printing contents; test both session directions. Do not read or restore an earlier real clipboard. |
| Capture and input | Owner prepares an entirely fictional test desktop/window. Actual agent tools cannot see or control the owner window/session through capture, Accessibility, Automation or a connected host. Capturing its own agent session is not a failure. |
| Privilege | Independent owner sudo/admin review plus actual tool denial. Do not enter owner credentials or grant Full Disk Access, Accessibility, Screen Recording, Automation or debugging rights. |
| Provider | Denied other model/tool/file/admin requests, actual one-hour expiry, $1 hard limit, rates and owner revocation. Fixture success does not establish provider behavior. |

Any unexpected success, missing fixture, stale process, unverified control route, changed owner hash, unexplained ACL or unusable ownership enforcement stops the real-key pilot. Keep the owner Vault untouched. Account/fixture setup is reversible through owner-reviewed cleanup; do not delete an account or home directory automatically.

## Evidence and remaining work

Preparation tests exercise parsers, fake filesystem outcomes, role/path denial and the bridge with fictional values. They do not prove a cross-UID boundary because the new account does not yet exist. Actual account creation, privileged authorization review, filesystem/process/clipboard/capture checks, token ACL provisioning, provider settings, key creation and live generation remain pending.

Apple sources checked 2026-10-21: [Standard account creation](https://support.apple.com/en-ie/guide/mac-help/mchl3e281fc9/27/mac/27), [filesystem permissions and ACL interaction](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGuide/FileSystemDetails/FileSystemDetails.html). OpenAI sources: [least privilege](https://developers.openai.com/api/docs/guides/terraform/service-accounts), [expiry](https://developers.openai.com/api/docs/guides/production-best-practices#api-keys), [hard limits](https://developers.openai.com/api/docs/guides/spend-limits).
