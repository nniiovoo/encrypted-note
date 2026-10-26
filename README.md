# Restricted OpenAI text companion

A separate, optional local MCP tool for short text generation. This directory is outside encrypted-note. It neither imports that app nor opens its Vault, Notes, metadata, backups, recovery material, wallet keys or IPC. It is a runnable development implementation with fictional-provider tests; no live key, account configuration, deployment or ChatGPT connection has been performed.

The default transport is stdio: importing the modules or running tests starts no production listener. Optional HTTP starts only with the explicit HTTP entry point and binds 127.0.0.1:8789. There is no hosted account system, public listener, telemetry, browser launcher or automatic tunnel setup.

## Use from your agent or code without a provider key

The selected deployment is: agent/code → stdio bridge → local owner broker → OpenAI → text result. The bridge receives a separate limited local token; only the owner broker reads the new Agent Key and adds provider authentication. Your code calls `generate_text({ text })` through MCP. It needs no OpenAI API key or secret-bearing client configuration.

[The keyless code example](docs/keyless-code.md) shows the client call and its prerequisites. This is one restricted operation, not a drop-in OpenAI SDK endpoint. The Mac identity boundary, provider controls and live connection remain unverified. The planned credential file format is plaintext with owner-only permissions, not encrypted Vault storage; no real credential file has been provisioned. OpenAI receives the key over HTTPS for authentication. Owner Vault and wallet keys are never part of this route.

## Fixed operation

Exactly one MCP tool is exposed:

- generate_text accepts exactly { "text": "..." }; no other properties.
- Nonempty, well-formed text: at most 4096 UTF-8 bytes.
- One fixed POST destination: https://api.openai.com/v1/responses.
- One fixed model: gpt-6-luna; reasoning none; service tier default.
- At most 512 output tokens, including any reasoning; response text is additionally bounded to 64 KiB.
- store, stream and background are false. tools is empty and tool_choice is none.
- No files, images, history, previous responses, prompt templates, URL, headers, model choice, plugins or tool definitions can be supplied.
- No automatic retry or redirect following. A timeout or cancellation may still incur a provider charge.
- One in-flight request, six admitted requests per rolling minute, 60 per rolling hour, and 100 per process. Failed admitted requests consume the local allowance.
- Provider response bodies are bounded to 256 KiB. Only completed assistant text from the fixed model returns to the caller; provider errors, headers and response identifiers do not.
- Local counters reset when this process restarts. They supplement independent provider caps; they are not durable provider spending controls.
- An owner grant ends at an absolute time at most one hour away, or after one hour of monotonic time, whichever is first. Restarting never extends it; an expired grant returns a fixed error with no provider work.
- Returned text is untrusted. The companion never executes it.

The versioned nonsecret contract is policy.json. Runtime input cannot change it. The trusted deployment must protect code and policy against agent modification.

GPT-6 Luna is currently documented as an efficient focused-task model, priced at $0.10/M short-context input tokens and $0.50/M output tokens under Standard pricing. Cache-write pricing may also apply. This fixes a model ID, not a cryptographic model-weights snapshot. Sources checked 2026-10-21: [model](https://developers.openai.com/api/docs/models/gpt-6-luna), [pricing](https://developers.openai.com/api/docs/pricing).

## Develop and verify without credentials

Use Node 22 or later:

    npm ci --ignore-scripts
    npm test

Tests inject a fictional provider in code and use obvious fake markers. They include the official MCP client over an actual stdio subprocess and optional loopback Streamable HTTP. No test calls the OpenAI API. There is no runtime provider-URL override or fake-provider switch in the production entry points.

Dependencies are exact-pinned: @modelcontextprotocol/sdk 1.32.1 supplies official MCP server/client protocol and transports; zod 4.6.7 supplies the SDK input/output schemas. package-lock.json pins their transitive resolution. No OpenAI SDK or separate HTTP framework is added directly; production inference uses Node HTTPS. The [official OpenAI MCP guide](https://developers.openai.com/plugins/build/mcp-server) recommends the official MCP SDKs.

## Development runtime entry points

A live runtime needs a NEW, separately restricted Agent Key. It must never use an owner key or a value stored in the owner Vault.

The generic development entry points require OPENAI_AGENT_API_KEY in the trusted companion process environment. They do not read OPENAI_API_KEY, dotenv files, the owner Keychain, home-directory credential stores or app data. This variable must not enter the agent environment, tool arguments, MCP client configuration, chat, shell history, logs or repository. No actual credential is included or created here. The selected protected deployment instead uses the [prepared owner launcher and agent bridge](isolation/PLAN.md); callers must not launch the generic entry point with a provider key.

After an independent owner-controlled runtime is configured, npm start runs stdio. A same-account unrestricted agent can inspect or tamper with a process, environment or code owned by that account. This implementation does not solve that OS boundary. Do not give it a real key until the owner has selected and verified a protected execution identity and readonly deployment outside agent control. The key is held in JavaScript process memory; secure erasure is not guaranteed.

Optional npm run start:http additionally requires a distinct AGENT_COMPANION_BEARER_TOKEN of 32–256 base64url characters. The bearer token authorizes only this bounded tool; it is not the provider key. HTTP checks authorization on every request, accepts only the /mcp path, rejects query strings and batching, bounds bodies to 64 KiB, and rejects unexpected Host and Origin values. GET and DELETE are unsupported. Only loopback is bound. It adds no user registration or login system.

No startup command containing a secret or secret-bearing MCP configuration is supplied. Secret provisioning and runtime control remain an independent owner operation.

## Required provider controls before a live pilot

None of these controls has been applied or verified by this project. The owner selected a one-hour expiry and a $1 monthly project hard spend limit for the first trial; those settings remain unapplied and unverified.

| Control | Required owner verification |
| --- | --- |
| Separate project and credential | New dedicated Agent Key; no owner credential or owner Vault data. |
| Permission | Only api.responses.write on the key and its effective least-privilege project role. Avoid broader inherited owner/member grants. All other key permissions denied; an empty scopes array means ALL permissions, not no permissions. |
| Model | Provider project allowlist contains only gpt-6-luna. |
| Tools | File search, web search, image generation, MCP and Code Interpreter disabled in provider policy. Disabling a project tool may require changing an organization policy from allow-all to selected-project access. |
| Remaining tool gap | These five provider switches do not document disabling hosted shell. The companion forbids all tool requests, but a stolen raw key could retain broader Responses capability. Protected custody remains necessary. |
| Expiry | Fixed provider-enforced expiration; no agent authority to extend it. |
| Spending | An enforced project HARD limit, not a spend alert. Enforcement can slightly overshoot during propagation. |
| Usage | Owner-approved model/project rate limits; verify provider-side rejection. Local rate controls alone do not prove provider enforcement. |
| Administration | Agent cannot create credentials, change roles, raise caps, extend expiry or edit protected companion code. |
| Revocation | Owner-controlled provider revoke/delete operation, followed by an observed denied request. Service-account-owned keys require the applicable service-account revocation procedure; deleting an individual project key via the generic API rejects service-account-owned keys. |
| Data destination | Supplied text goes to OpenAI; generated text returns to the calling agent and may enter that agent's cloud conversation. No owner-Vault content or metadata is permitted. |
| Retention | store:false does not establish Zero Data Retention. Abuse monitoring and prompt-cache retention may still apply. |
| Negative controls | Denied models, endpoints, tools, admin permissions; expiry and revocation; cap rejection; output/error/log non-disclosure; isolated custody and code ownership. |

Sources: [least-privilege service accounts](https://developers.openai.com/api/docs/guides/terraform/service-accounts), [model/tool controls](https://developers.openai.com/api/docs/guides/terraform/project-controls), [key expiry](https://developers.openai.com/api/docs/guides/production-best-practices#api-keys), [hard spend limits](https://developers.openai.com/api/docs/guides/spend-limits), [data handling](https://developers.openai.com/api/docs/guides/your-data#v1responses). These are documentation findings, not evidence of this account's actual permissions.

## Optional ChatGPT connection

Local stdio is the primary integration and needs no public server. The official SDK client tests prove protocol behavior, not a live ChatGPT connection.

ChatGPT can optionally reach a private MCP server through owner-selected Secure MCP Tunnel. That path requires a separately approved tunnel identity, narrow runtime tunnel permissions, the correct Platform organization/workspace associations, and a trusted local tunnel-client. It can launch this stdio server without a public listener. Do not share the inference key with the tunnel's control-plane identity or grant tunnel administration to the inference key.

Tunnel creation, client installation, sign-in, credential provisioning, workspace changes, authentication configuration and cloud connectivity have not been performed. They remain optional owner decisions. The optional HTTP bearer transport does not implement OAuth or claim to complete ChatGPT authentication by itself.

[Official private-tunnel requirements](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels), [ChatGPT connection and end-to-end test](https://developers.openai.com/plugins/deploy/connect-chatgpt). Step-by-step preparation and the checks that make a connection count as verified: [docs/chatgpt-connection.md](docs/chatgpt-connection.md).

## Current limits

This implementation is ready for code review and fictional-data development. It is not approved for real keys or production use. Provider controls, OS isolation, credential custody, successful live generation, ChatGPT authentication and end-to-end connection remain unverified. Neither source review nor a successful fake-provider test can guarantee that a key cannot be stolen or misused.

## Integrated with encrypted-note

encrypted-note can bundle this companion as its optional **Agent Companion** helper ([encrypted-note ADR-0009](https://github.com/nniiovoo/encrypted-note/blob/main/docs/adr/0009-optional-agent-companion-helper.md)). The app pins one commit of this repository, the SHA-256 of policy.json and an official Node.js release, and runs `isolation/integrated-broker.mjs` inside a separate XPC service with its own macOS sandbox container. A fictional-data spike showed that a plain child process of the sandboxed app could read the app's container, while that XPC service could not; the app's Vault therefore stays outside this companion's reach.

The integrated entry point runs the same fixed broker with:

- fixed data paths inside the helper's own container (`$HOME/Library/Application Support/encrypted-note-companion/plan.json` and `credential.env`), never chosen by a caller;
- one new grant of at most one hour for each owner Start, enforced before the credential read, before listening and before every admission, including direct loopback calls;
- a fresh delegated agent token for each Start, written owner-only (0600, plus the single agent read ACL) into the owner-prepared handoff folder, and removed on every stop;
- a bounded owner-control protocol on stdin/stdout, separate from the agent MCP channel: one `{"v":1,"op":"stop"}` command in; `ready`, `stopped`, `error` and `check` events with fixed codes out. Control EOF, a malformed command, a signal or the grant deadline stops the broker. Nothing from the plan, credential, token, provider or MCP traffic is printed.

Its `--check` mode reports readiness booleans and never reads the credential's contents. Code and runtime ancestors may be admin-group-writable (an app in /Applications) only after the agent account is confirmed not to be an administrator.

Reviewed plans are now version 2 with `grantExpiresAt`: a canonical UTC instant for the standalone owner launcher, or `null` in a plan installed into encrypted-note, where each Start issues the grant. Version-1 plans are refused.

## Prepared isolated Mac setup

[The reviewed setup package](isolation/PLAN.md) prepares a Standard-account boundary, an owner-controlled launcher that reads only the secure helper’s confirmed 0600 key file, and an agent-side local stdio bridge. No account, ACL, provider setting, key or runtime was changed. The generic environment-based entry points above remain development interfaces; the selected protected deployment uses this separate setup and keeps the provider key out of environment/configuration.
