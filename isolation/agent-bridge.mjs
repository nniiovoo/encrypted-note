import { realpathSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
import { startStdio } from '../src/stdio.mjs';
import { CompanionError, errorFromPublicText, POLICY, validateInput } from '../src/generate.mjs';
import { loadPlan, assertStandardIdentity, readAgentToken } from './common.mjs';

// port is for tests only; the bridge itself always uses the fixed loopback port.
export async function connectLocalBroker(token, port = 8789) {
  const client = new Client({ name: 'isolated-agent-text-bridge', version: '0.1.0' });
  await client.connect(new StreamableHTTPClientTransport(new URL('http://127.0.0.1:' + port + '/mcp'), {
    requestInit: { headers: { authorization: 'Bearer ' + token }, redirect: 'error' },
  }));
  return client;
}
export function bridgeGenerator(client) {
  return async (args, { signal } = {}) => {
    validateInput(args);
    let result;
    try { result = await client.callTool({ name: 'generate_text', arguments: args }, undefined, { signal }); }
    catch { throw new CompanionError(signal?.aborted ? 'cancelled' : 'helper_off'); }
    if (result?.isError) throw errorFromPublicText(result.content?.[0]?.text);
    const content = result?.structuredContent;
    if (!content || Object.keys(content).length !== 1 ||
        typeof content.text !== 'string' || !content.text.isWellFormed() ||
        Buffer.byteLength(content.text) > POLICY.maxOutputBytes) {
      throw new CompanionError('provider_unavailable');
    }
    return { text: content.text };
  };
}
// Reads the delegated token on every call, so the tool is always served and a new Start needs
// no restart of the agent's MCP client. No token or no listener gives the fixed helper_off text.
export function tokenBridge(tokenFile, ownerUid, { read = readAgentToken, connect = connectLocalBroker } = {}) {
  let current;
  // Takes effect before any await, so parallel calls share one connection.
  const replace = next => {
    const old = current;
    current = next;
    return old?.client.then(client => client.close()).catch(() => {});
  };
  const generate = async (args, options) => {
    validateInput(args);
    let token;
    try { token = await read(tokenFile, ownerUid); }
    catch { void replace(); throw new CompanionError('helper_off'); }
    if (current?.token !== token) void replace({ token, client: connect(token) });
    const connecting = current;
    let client;
    try { client = await connecting.client; }
    catch {
      if (current === connecting) current = undefined;
      throw new CompanionError('helper_off');
    }
    return bridgeGenerator(client)(args, options);
  };
  return { generate, close: () => replace() };
}
// Node resolves links for this module's own URL, so a linked path must count as this file too.
if (process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
  try {
    if (process.argv.length !== 3) throw new Error('Plan required.');
    const plan = await loadPlan(process.argv[2]);
    await assertStandardIdentity(plan);
    const bridge = tokenBridge(plan.bearerTokenFile, plan.ownerUid);
    const server = await startStdio({ generate: bridge.generate });
    server.server.onclose = () => { void bridge.close(); };
  } catch {
    process.stderr.write('Isolated agent bridge did not start. Check the plan path and that this is the planned standard agent account.\n');
    process.exitCode = 1;
  }
}
