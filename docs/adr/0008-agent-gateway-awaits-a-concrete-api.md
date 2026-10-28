# Agents use a separate local companion for bounded OpenAI text generation

> **Amended by [ADR-0009](0009-optional-agent-companion-helper.md) (2026-10-26):** the companion can now also be bundled into encrypted-note as a separately sandboxed helper, pinned by commit and policy hash, with an absolute one-hour grant enforced at the broker. Its standalone launcher remains. The real-key gate below is unchanged.

Status: Service and operation selected by the owner, 2026-10-21; companion source published privately and development/preparation behavior verified with 30 fictional tests. Actual OS isolation, protected execution, provider controls, a real-key pilot and optional ChatGPT cloud connection remain pending. Refs [issue #28](https://github.com/nniiovoo/encrypted-note/issues/28).

The owner selected **OpenAI API text generation using one fixed model**, with a new separate restricted **Agent Key** and no owner-Vault access ([ADR-0007](0007-agents-use-separate-restricted-keys.md)). This replaces the earlier read-only-operation proposal. The selected implementation is a separate local companion whose primary interface is MCP over stdio.

The local encrypted-note app remains independent of this workflow. It needs no OpenAI account, hosted server, application login, tunnel or service credential to create, store or open a Vault. It receives no network code, dependency, agent interface, credential handling or file-format change ([ADR-0001](0001-local-only-vault-moved-by-hand.md), [ADR-0003](0003-tauri-with-secrets-kept-in-rust.md)). The companion protocol implements no app IPC, Vault, Note, metadata, Backup, Safety Copy, Master Password, Recovery Key, Wallet Kind or Emergency Reader route. Proving that the deployed runtime cannot read owner files or inspect owner processes requires the actual OS boundary checks that remain pending.

## Selected operation

The companion exposes only generate_text with exactly one text argument:

| Boundary | Fixed development contract |
| --- | --- |
| Input | Nonempty, well-formed plain text; at most 4096 UTF-8 bytes; no additional properties. |
| Provider operation | POST https://api.openai.com/v1/responses, constructed in trusted companion code. |
| Model | gpt-6-luna, reasoning none, service tier default; caller cannot select a model. |
| Output | At most 512 output tokens and 64 KiB returned text. Only completed assistant text from the fixed model returns; no provider headers, error bodies or response identifiers. |
| State and tools | store, stream and background false; tools empty; tool_choice none. No files, images, history, previous responses, prompt templates, custom destinations or tool definitions. |
| Local admission | One in-flight request; six per rolling minute, 60 per rolling hour and 100 per process. Failed admitted requests consume allowance. Restarting resets these local counters. |
| Failure | No redirect following or automatic retries; a timed-out or cancelled request may still be billed. |
| Destinations | Supplied text goes to OpenAI; returned text goes to the caller and may enter that agent's cloud conversation. Vault content and metadata are excluded throughout. |

These are implementation controls, not proof of provider permissions or protected key custody. A model instruction, tool annotation or domain allowlist cannot replace enforcement.

The official [model documentation](https://developers.openai.com/api/docs/models/gpt-6-luna) supports reasoning none and describes GPT-6 Luna as an efficient focused-task model. Current [Standard pricing](https://developers.openai.com/api/docs/pricing) is $0.10 per million short-context input tokens and $0.50 per million output tokens; cache writes may also be billed. Sources checked 2026-10-21. Fixing a model ID does not cryptographically pin its weights.

## Implementation and connection

The companion source, exact-pinned official MCP SDK dependencies and fictional tests are published in the private [encrypted-note-agent-companion repository](https://github.com/nniiovoo/encrypted-note-agent-companion), including merged [PR #4](https://github.com/nniiovoo/encrypted-note-agent-companion/pull/4), [PR #5](https://github.com/nniiovoo/encrypted-note-agent-companion/pull/5) and [PR #6](https://github.com/nniiovoo/encrypted-note-agent-companion/pull/6). They are not app dependencies or part of encrypted-note's release bundle. Operational logs remain private; source publication does not establish a protected live deployment. The available OpenAI Platform connector provisions encrypted expiring credentials; its exposed tools do not provide this bounded inference operation or establish protected runtime custody.

Local agents can use stdio without hosting or a public listener. An optional Streamable HTTP transport binds only loopback when explicitly started and requires separate authorization with strict Host, Origin, path and body checks. Neither entry point runs automatically with encrypted-note.

ChatGPT cloud cannot reach this computer's localhost or stdio directly. An optional [Secure MCP Tunnel](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels) can connect a private stdio or HTTP server using independently approved runtime permissions and the correct organization/workspace associations. It is not required for local-agent use or the Vault. No tunnel, hosted endpoint, account system or live ChatGPT authentication has been configured. A live connection is complete only after an observed end-to-end tool call ([official connection requirements](https://developers.openai.com/plugins/deploy/connect-chatgpt)).

## Real-key gate

A real-key pilot still requires independent owner control and verification of all of these together:

- A new dedicated Agent Key, with only api.responses.write on both the key and its effective least-privilege role; no broader inherited grants. The provider project model allowlist contains only gpt-6-luna. An empty key scopes array means all permissions, not no permissions ([service accounts](https://developers.openai.com/api/docs/guides/terraform/service-accounts), [model controls](https://developers.openai.com/api/docs/guides/terraform/project-controls)).
- Fixed provider-enforced expiry, owner-controlled revocation and denied requests after expiry/revocation. The agent cannot create credentials, extend expiry, change permissions or raise limits ([key lifetime guidance](https://developers.openai.com/api/docs/guides/production-best-practices#api-keys)).
- An enforced project hard spend limit and appropriate provider rate limits. A spend alert is insufficient. Current OpenAI hard limits can slightly overshoot while enforcement propagates; local admission counters are not durable provider caps ([spend limits](https://developers.openai.com/api/docs/guides/spend-limits)).
- Provider file-search, web-search, image-generation, MCP and Code Interpreter permissions disabled and verified. Project denial can require an organization policy permitting selected projects. These five switches do not document disabling hosted shell; the companion forbids tools, but a stolen raw key can retain broader Responses authority ([tool controls](https://developers.openai.com/api/docs/guides/terraform/project-controls#configure-hosted-tools)).
- Protected credential custody and readonly deployment outside agent control. A same-account unrestricted agent can inspect or alter the companion's process, environment or files. Local stdio alone does not supply that OS boundary; existing owner keys must never be reused ([credential separation](https://developers.openai.com/api/docs/guides/agents-api/environments/security#separate-credentials)).
- Explicit output/destination and retention acceptance. store:false does not establish Zero Data Retention; abuse monitoring and prompt-cache retention may still apply ([data handling](https://developers.openai.com/api/docs/guides/your-data#v1responses)).

The owner approved a one-hour key expiry, a $1 monthly hard project limit and preparation of a separate standard macOS account with a protected launcher. These account/provider settings have not been applied or verified. The actual agent tools must run under the separate identity; this owner-account development session does not establish isolation.

## Current outcome

The separate companion passed 30 automated fictional tests: 21 provider/protocol tests, including official MCP clients over subprocess stdio and loopback HTTP, plus 9 isolation-preparation tests using fictional fixtures. Independent review produced fixes for HTTP cancellation, stdin EOF and abrupt input close; regression tests cover all three. The preparation tests do not establish an actual OS execution identity, deployed permissions or provider controls. The exact scope and remaining checks are recorded in [Agent Key validation](../agent-key-validation.md).

No real credential, provider account, live inference, provider limit, actual OS isolation boundary or ChatGPT connection has been exercised. Issue #28 remains open for those operational checks; development verification does not make a real-key recommendation.
