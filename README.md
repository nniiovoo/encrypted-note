# Bounded OpenAI text operation

Development core for a separate optional agent companion. It does not integrate with encrypted-note or read its Vault, Notes, metadata, wallet material, recovery data or IPC.

The operation accepts exactly one nonempty, well-formed text string of at most 4096 UTF-8 bytes. Trusted code constructs only POST https://api.openai.com/v1/responses using fixed gpt-6-luna, reasoning none, service tier default and at most 512 output tokens. Tools are empty; storage, streaming and background mode are disabled. The provider destination, model, headers, files, history and policy cannot be overridden by input.

Admission permits one in-flight call, six per minute, 60 per hour and 100 per process. Admitted failures consume allowance. No redirects or automatic retries are followed; cancellation/timeouts abort work but may still be billed. Only bounded completed assistant text returns; raw provider errors, headers and exact-key echoes are rejected.

Run the fictional-provider tests with Node 22 or later:

    npm test

This slice has no third-party dependency or MCP runtime. Its tests inject a fictional provider or mock HTTPS; they never call OpenAI.

No real key, provider permissions/expiry/hard cap, protected key custody, agent execution boundary or live generation has been verified. Existing owner keys must never be reused. Source review and tests do not establish that a credential cannot be stolen or misused.
