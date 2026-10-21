import { pathToFileURL } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
import { startStdio } from '../src/stdio.mjs';
import { CompanionError, POLICY, validateInput } from '../src/generate.mjs';
import { loadPlan, assertStandardIdentity, readAgentToken } from './common.mjs';

export async function connectLocalBroker(token) {
  const client = new Client({ name: 'isolated-agent-text-bridge', version: '0.1.0' });
  await client.connect(new StreamableHTTPClientTransport(new URL('http://127.0.0.1:8789/mcp'), {
    requestInit: { headers: { authorization: 'Bearer ' + token }, redirect: 'error' },
  }));
  return client;
}
export function bridgeGenerator(client) {
  return async (args, { signal } = {}) => {
    validateInput(args);
    let result;
    try { result = await client.callTool({ name: 'generate_text', arguments: args }, undefined, { signal }); }
    catch { throw new CompanionError(signal?.aborted ? 'cancelled' : 'provider_unavailable'); }
    const content = result?.structuredContent;
    if (result?.isError || !content || Object.keys(content).length !== 1 ||
        typeof content.text !== 'string' || !content.text.isWellFormed() ||
        Buffer.byteLength(content.text) > POLICY.maxOutputBytes) {
      throw new CompanionError('provider_unavailable');
    }
    return { text: content.text };
  };
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  let client;
  try {
    if (process.argv.length !== 3) throw new Error('Plan required.');
    const plan = await loadPlan(process.argv[2]);
    await assertStandardIdentity(plan);
    const token = await readAgentToken(plan.bearerTokenFile, plan.ownerUid);
    client = await connectLocalBroker(token);
    const server = await startStdio({ generate: bridgeGenerator(client) });
    server.server.onclose = () => { void client.close(); };
  } catch {
    await client?.close();
    process.stderr.write('Isolated agent bridge did not start. Check the planned standard identity and delegated token.\n');
    process.exitCode = 1;
  }
}
