# Restricted OpenAI text companion

A separate optional local MCP tool for short text generation, outside encrypted-note. It does not read that app's Vault, Notes, metadata, wallet/recovery material, backups or IPC.

The only tool is generate_text({text}). It accepts at most 4096 UTF-8 input bytes and sends one trusted request to https://api.openai.com/v1/responses using fixed gpt-6-luna, reasoning none, service tier default and at most 512 output tokens. Tools, response storage, streaming and background mode are disabled. Input cannot override destinations, models, files, history or policy.

The default transport is stdio. Optional Streamable HTTP starts only explicitly, binds loopback and checks a distinct bearer token, Host, Origin, path and message size. Both transports propagate cancellation and disconnects to admitted provider work. No public listener, account system, tunnel or automatic startup is configured.

Use Node 22 or later:

    npm ci --ignore-scripts
    npm test

Tests use fictional providers, including official MCP clients over an actual stdio subprocess and optional loopback HTTP. They never call OpenAI.

Exact-pinned dependencies: @modelcontextprotocol/sdk 1.32.1 provides official MCP protocol/transports; zod 4.6.7 provides input/output schemas. package-lock.json pins their transitive resolution. No OpenAI SDK or direct web framework dependency is added.

These entry points are development interfaces. A same-account unrestricted agent can inspect or alter an owner process, environment or code. No protected runtime or real-key deployment is established here. No owner key may be reused. Provider scope, one-hour expiry, $1 enforced monthly hard cap, model/tool restrictions, revocation and actual agent isolation must be independently configured and verified before a live trial.

Local stdio does not itself establish a ChatGPT cloud connection. No key, account, live generation, provider limit or cloud connection was exercised.
