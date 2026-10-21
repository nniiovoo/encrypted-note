# Restricted OpenAI Agent Key workflow validation

Status: ready-for-human — separate companion source published privately; 30 fictional tests passed: 21 provider/protocol tests and 9 isolation-preparation tests. Actual OS isolation, protected custody, provider configuration and a real-key pilot remain unverified. Refs [#27](https://github.com/nniiovoo/encrypted-note/issues/27) and [#28](https://github.com/nniiovoo/encrypted-note/issues/28).

The owner selected OpenAI API text generation with one fixed model and a new separate restricted Agent Key. Agents receive no owner Vault, Notes, metadata, Master Password, Recovery Key, Wallet Kind, Backup, Safety Copy or Emergency Reader access ([ADR-0007](adr/0007-agents-use-separate-restricted-keys.md)). The operation and outstanding gate are recorded in [ADR-0008](adr/0008-agent-gateway-awaits-a-concrete-api.md).

This workflow is an optional companion outside encrypted-note. The app adds no network code, dependency, listener, account login or agent interface ([ADR-0001](adr/0001-local-only-vault-moved-by-hand.md)). The local Vault needs no OpenAI account or server. Local agents use MCP over stdio; no hosted service or public listener is required.

## Reviewed development contract, 2026-10-21

The companion source is published in the private [encrypted-note-agent-companion repository](https://github.com/nniiovoo/encrypted-note-agent-companion), including merged [PR #4](https://github.com/nniiovoo/encrypted-note-agent-companion/pull/4), [PR #5](https://github.com/nniiovoo/encrypted-note-agent-companion/pull/5) and [PR #6](https://github.com/nniiovoo/encrypted-note-agent-companion/pull/6). It uses the official MCP SDK, exact-pinned to @modelcontextprotocol/sdk 1.32.1 with zod 4.6.7 for schemas. These dependencies are outside the offline app. A reproducible install with lifecycle scripts disabled reported zero known package vulnerabilities at validation time; this is not a security guarantee.

| Boundary | Implemented and checked with fictional data |
| --- | --- |
| Tool and parameters | Only generate_text; exactly one nonempty, well-formed text argument, at most 4096 UTF-8 bytes. Additional model, URL, header, file, history, state and policy arguments are rejected before provider work. |
| Destination | Trusted code constructs POST https://api.openai.com/v1/responses. No runtime provider-URL override, redirect following or automatic retry. |
| Model and output | Fixed gpt-6-luna, reasoning none, service tier default; max_output_tokens 512. Only completed assistant text from that model returns, bounded to 64 KiB. |
| Provider state | store, stream and background false; tools empty and tool_choice none. No files, images, prompt templates, previous responses or conversations. |
| Admission | One concurrent request; six per rolling minute, 60 per rolling hour and 100 per process. Invalid input consumes none; admitted failures consume allowance. Restart resets local counters. |
| Bounds and errors | MCP messages bounded to 64 KiB and provider responses to 256 KiB. Thrown provider errors, headers, response IDs and error bodies are not forwarded. An exact provider-key echo is rejected. |
| Lifecycle | Timeout and cancellation abort work without retry. Independent review found HTTP cancellation, stdin EOF and abrupt input-close gaps; all were fixed and regression-tested. A cancelled request may still be billed. |
| Local MCP | Official client initialized, listed exactly one tool and called it successfully through a real stdio subprocess with a fictional provider. |
| Optional HTTP | Official Streamable HTTP client succeeded on an explicitly started loopback fixture. Missing/wrong authorization, unsafe Host/Origin, query paths, batching and oversized bodies were rejected. No production listener was started. |
| Configuration | Only a dedicated Agent Key variable is accepted; owner OPENAI_API_KEY is not reused. Optional HTTP needs a distinct bearer token. No actual credential file was created or inspected. |

All 30 automated fictional tests passed: 21 exercise provider/protocol behavior, and 9 exercise isolation preparation with fictional fixtures. Independent review found no remaining request-override or direct credential-disclosure defect in the inspected implementation. These results establish the tested source and preparation behavior; they do not establish an actual separate OS identity, deployed permissions, provider control, secure erasure, real billing behavior or resistance to an unrestricted same-account agent.

Published source remains separate from this app and its bundle. Operational logs remain private; repository publication is not evidence of a completed protected deployment or live pilot. Publish only sanitized findings and source versions; do not copy private paths, raw settings, account names, real credentials, headers or account transcripts into GitHub.

## Current official provider evidence

Sources below were checked on 2026-10-21. No account settings or provider permissions were queried.

| Requirement | Current documentation | Runtime result |
| --- | --- | --- |
| Fixed model | GPT-6 Luna supports reasoning none; the companion fixes its model ID ([model](https://developers.openai.com/api/docs/models/gpt-6-luna)). A fixed ID does not cryptographically pin weights. | Fictional request construction passed; actual account availability pending. |
| Narrow credential | A least-privilege service account and key can grant only api.responses.write; role grants are additive, and key scopes cannot exceed effective role access ([service accounts](https://developers.openai.com/api/docs/guides/terraform/service-accounts)). An empty scopes array means all permissions. | Separate role, key and denied permissions pending. |
| Model restriction | A provider project allowlist can permit only gpt-6-luna ([project controls](https://developers.openai.com/api/docs/guides/terraform/project-controls)). | Allowlist and denied-model checks pending. |
| Hosted tools | Five project policies cover file search, web search, image generation, MCP and Code Interpreter; project denial can require organization selected-project policy ([tool controls](https://developers.openai.com/api/docs/guides/terraform/project-controls#configure-hosted-tools)). Hosted-shell denial is not established by those five switches. | Provider tool denial pending; companion tool exclusion passed. |
| Expiry | Fixed project-key expiration and administrator maximum lifetimes are supported ([key lifetime](https://developers.openai.com/api/docs/guides/production-best-practices#api-keys)). | No key created; expiry and denied-after-expiry check pending. |
| Revocation | The owner must revoke/delete the provider credential, using the applicable service-account procedure where needed; stopping the local tool alone is insufficient. | No key created; revocation and denied-after-revocation check pending. |
| Hard spend cap | Enforced organization and project hard monthly limits return 429 after tracked spend reaches a cap; alerts do not stop traffic, and propagation permits a small overshoot ([spend limits](https://developers.openai.com/api/docs/guides/spend-limits)). | No limit applied or observed; pending. |
| Usage caps | Independent provider rate limits supplement the companion's admission controls; a monthly spend limit is not an exact request-count cap. | Local admission tests passed; provider limits pending. |
| Retention | store:false does not establish Zero Data Retention; abuse-monitoring and prompt-cache retention may still apply ([data handling](https://developers.openai.com/api/docs/guides/your-data#v1responses)). | Documentation only; actual project policy pending. |

The owner approved a one-hour credential expiry and a $1 monthly provider hard project limit for the first trial. These controls have not been applied or verified; approved values are not active account settings or evidence of a successful real-key pilot.

## Remaining owner-controlled operational checks

A real pilot requires an independent owner-controlled session that the agent cannot administer. Fix the concrete execution identity, protected code ownership, secret custody, actual provider project, operation limits, permitted output destinations and cleanup before importing a new Agent Key. No existing owner key may be reused or copied from the Vault.

The owner approved preparation of the separate standard macOS account/protected launcher and a first trial with a one-hour key expiry and $1 provider hard project cap. Preparation does not establish that the account, launcher or provider controls are active; their observed checks remain pending below.

A companion process under the same unrestricted account as the agent does not protect its key or policy from that agent. An environment variable kept out of chat is not an OS boundary. [Separate-account validation](agent-account-validation.md) remains pending; another independently reviewed execution boundary may be selected instead. No actual OS account, permission, home-folder or service change has been made for this workflow. The 9 isolation-preparation tests use fictional fixtures and do not replace the real OS boundary checks below.

| Check | Required observation before a real-key recommendation | Result |
| --- | --- | --- |
| Custody and code | Agent tools cannot read the provider key, inspect the keyholder process or modify protected code/policy; only approved fictional host fixtures are used to test the boundary first. | pending |
| Provider authority | Only the approved model and operation work; file/tool/admin/model expansion and credential management are denied as scoped. | pending |
| Expiry and revoke | A previously successful dedicated credential is rejected after expiry and owner revocation, with propagation behavior recorded. | pending |
| Hard limits | Applied provider hard-spend and rate limits are verified independently; limit errors do not trigger a retry or automatic limit increase. | pending |
| Live generation | One explicitly approved fictional text request succeeds through the protected runtime, with sanitized output and actual provider usage recorded. | pending |
| Disclosure and logs | No actual key, secret-bearing header, raw provider error or owner data reaches model-visible results, process arguments, history or logs. | pending |
| Cleanup | Scoped test credentials and optional runtime resources are removed without affecting owner Vaults or other services. | pending |

A fake invalid key returning an error cannot establish successful injection, provider revocation or expiry. The tested fake transport bypasses the live provider deliberately; it proves none of those account properties. Even a successful pilot permits misuse of every allowed operation up to independently enforced limits.

## Optional ChatGPT cloud connection

Local stdio is available to a local MCP client. ChatGPT cloud cannot directly reach localhost or a local stdio process. An owner-selected [Secure MCP Tunnel](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels) can reach a private stdio/HTTP server without exposing a public listener, with separate tunnel-runtime permissions and the appropriate organization/workspace associations.

Tunnel creation, client installation, authentication, workspace changes and cloud connectivity have not been performed. They remain optional and have no effect on the offline Vault. The optional loopback bearer transport does not itself implement OAuth or establish a ChatGPT connection.

Only an observed authenticated end-to-end tool call can complete that connection check ([official connection tests](https://developers.openai.com/plugins/deploy/connect-chatgpt)). The approved output destination must explicitly include any ChatGPT/model service that receives supplied or generated text.

## Evidence outcome

Completed: private source publication in the separate companion repository, independent code review, 30 fictional tests (21 provider/protocol plus 9 isolation preparation), official local MCP client compatibility and reproducible dependency installation.

Pending: actual OS isolation, protected execution and credential custody; provider account, permissions, model/tool/rate/spend policies; actual expiry/revocation; live API generation; optional ChatGPT authentication and connection. No real credential, owner Vault or wallet material was used.
