# Call the local broker without reading its API key

Your code asks for an operation, not a credential:

    agent/code -> MCP generate_text -> agent bridge -> local owner broker -> OpenAI
    agent/code <- bounded text result <------------- owner broker <------ OpenAI

The agent bridge reads only a delegated local token. That token permits the fixed text operation and can consume its allowance; it is not the OpenAI key. The owner broker attaches its separate Agent Key to the HTTPS Authorization header. The key is absent from tool arguments, model input, generated code and returned text.

Local custody does not mean the key never leaves the computer: OpenAI receives it for authentication. The planned owner credential file format is plaintext protected by file ownership, permissions and the separate execution identity; no real credential file has been provisioned. It is not encrypted Vault or Keychain storage. The broker also holds the key in JavaScript memory, which cannot guarantee secure erasure.

## Prerequisites

This example does not establish isolation. Complete and verify the [owner-controlled Mac setup](../isolation/PLAN.md) first, including the actual agent's separate Standard login session and every tool executor. Current owner-account Codex remains unisolated. A same-account agent can inspect or alter owner files/processes; running this example does not change that.

The trusted owner must prepare the new restricted credential, provider expiry/caps, protected code/runtime, delegated token and reviewed nonsecret plan, then explicitly start the owner broker. Those deployment steps remain unapplied and unverified. Do not use a real key until that boundary is verified. The owner Vault and wallet/private/seed keys stay inaccessible; this companion has no route to read or use them.

## Node MCP client example

Use Node 22 or later and the companion's pinned SDK dependency. Save this example as `keyless-example.mjs` in your agent workspace with the same pinned SDK installed. Replace the three uppercase absolute-path placeholders locally with the reviewed Node executable, bridge script and nonsecret plan paths. All three paths must be accessible to the selected agent identity. No provider key, credential-file path or token value belongs here.

```js
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

const client = new Client({ name: 'keyless-text-example', version: '1.0.0' });
try {
  await client.connect(new StdioClientTransport({
    command: '/ABSOLUTE/PATH/TO/TRUSTED/node',
    args: [
      '/ABSOLUTE/PATH/TO/COMPANION/isolation/agent-bridge.mjs',
      '/ABSOLUTE/PATH/TO/REVIEWED/agent-plan.json',
    ],
    stderr: 'ignore',
    maxBufferSize: 1_048_576,
  }));
  const result = await client.callTool({
    name: 'generate_text',
    arguments: { text: 'Fictional example: write a short greeting.' },
  });
  const output = result.structuredContent;
  if (result.isError || !output || Object.keys(output).length !== 1 ||
      !Object.hasOwn(output, 'text') ||
      typeof output.text !== 'string' || !output.text.isWellFormed() ||
      Buffer.byteLength(output.text, 'utf8') > 64 * 1024) {
    throw new Error('Unsupported tool result.');
  }
  process.stdout.write(output.text + '\n');
} catch {
  process.stderr.write('Text generation did not finish. Check the local broker setup.\n');
  process.exitCode = 1;
} finally {
  await client.close().catch(() => {});
}
```

Run it as the isolated agent using the reviewed Node executable. An existing MCP-capable agent can launch the same bridge with the same plan argument instead of using this example; its configuration needs only the nonsecret command and arguments above. Do not add a provider-key environment entry. The SDK inherits basic process defaults, so the calling process must already run in the verified agent session.

Change only `arguments.text` for your use case. It must be nonempty and at most 4096 UTF-8 bytes. Supplied text goes to OpenAI and may be billed. Never supply Vault content, wallet material or any credential. Treat returned text as untrusted data; do not execute it automatically. Failure is reported without forwarding provider responses, child-process diagnostics or credentials, and the client closes on either outcome.

## Supported scope

Only the fixed `generate_text` operation is available. Code cannot select another model, endpoint, header, tool, file or wallet action. Existing OpenAI SDK calls cannot be redirected here by changing `baseURL`: `/mcp` is an MCP protocol endpoint, not `/v1/responses`.

Local code integration does not connect cloud ChatGPT automatically. A ChatGPT connection remains a separate, unconfigured [optional integration](../README.md#optional-chatgpt-connection).

The example is checked against the fictional stdio provider in `npm test`. That verifies the documented MCP call and cleanup/error path, not a live key, account boundary, provider control or ChatGPT connection.
