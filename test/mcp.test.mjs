import test from 'node:test';
import assert from 'node:assert/strict';
import { PassThrough } from 'node:stream';
import http from 'node:http';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
import { createGenerator, POLICY } from '../src/generate.mjs';
import { createMcpServer } from '../src/mcp.mjs';
import { boundedInput, startStdio } from '../src/stdio.mjs';
import { startHttpServer } from '../src/http.mjs';

const TOKEN = 'FAKE_HTTP_TOKEN_FOR_TESTS_ONLY_1234567890';
const KEY = 'FAKE_OPENAI_AGENT_KEY_FOR_TESTS_ONLY';
function generator() {
  return createGenerator({ apiKey: KEY, transport: async () => ({
    status: 200, json: { model: POLICY.model, status: 'completed',
      output: [{ type: 'message', role: 'assistant',
        content: [{ type: 'output_text', text: 'Fictional generated text.' }] }] },
  }) });
}
test('official MCP client sees exactly one bounded text tool', async () => {
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  const server = createMcpServer(generator());
  const client = new Client({ name: 'fictional-test-client', version: '1.0.0' });
  try {
    await server.connect(serverTransport);
    await client.connect(clientTransport);
    const tools = await client.listTools();
    assert.deepEqual(tools.tools.map(tool => tool.name), ['generate_text']);
    assert.equal(tools.tools[0].inputSchema.additionalProperties, false);
    assert.equal(tools.tools[0].annotations.readOnlyHint, false);
    assert.equal(tools.tools[0].annotations.idempotentHint, false);
    const result = await client.callTool({ name: 'generate_text', arguments: { text: 'A fictional greeting.' } });
    assert.deepEqual(result.structuredContent, { text: 'Fictional generated text.' });
    assert.equal(result.isError, undefined);
    const invalid = await client.callTool({ name: 'generate_text',
      arguments: { text: 'Fiction.', model: 'unallowed-model', url: 'https://unallowed.invalid/' } });
    assert.equal(invalid.isError, true);
  } finally { await client.close(); await server.close(); }
});
test('stdio input has a per-message bound including partial lines', async () => {
  const input = new PassThrough();
  const limited = boundedInput(input);
  const error = new Promise(resolve => limited.once('error', resolve));
  limited.resume();
  input.write(Buffer.alloc(POLICY.maxMcpMessageBytes, 65));
  input.write(Buffer.from('A'));
  assert.match((await error).message, /too large/);
  input.destroy();
});
test('stdio input accepts bounded successive messages', async () => {
  const input = new PassThrough();
  const limited = boundedInput(input);
  const chunks = [];
  limited.on('data', chunk => chunks.push(chunk));
  const ended = new Promise(resolve => limited.once('end', resolve));
  input.end('{"jsonrpc":"2.0"}\n{"jsonrpc":"2.0"}\n');
  await ended;
  assert.equal(Buffer.concat(chunks).toString(), '{"jsonrpc":"2.0"}\n{"jsonrpc":"2.0"}\n');
});
test('optional loopback Streamable HTTP works with the official MCP client', async () => {
  const server = await startHttpServer({ generate: generator(), bearerToken: TOKEN, port: 0 });
  const client = new Client({ name: 'fictional-http-test', version: '1.0.0' });
  const address = 'http://127.0.0.1:' + server.address().port + '/mcp';
  try {
    await client.connect(new StreamableHTTPClientTransport(new URL(address), {
      requestInit: { headers: { authorization: 'Bearer ' + TOKEN } },
    }));
    const tools = await client.listTools();
    assert.deepEqual(tools.tools.map(tool => tool.name), ['generate_text']);
    const result = await client.callTool({ name: 'generate_text', arguments: { text: 'Fiction.' } });
    assert.deepEqual(result.structuredContent, { text: 'Fictional generated text.' });
  } finally {
    await client.close();
    await new Promise(resolve => { server.close(resolve); server.closeAllConnections(); });
  }
});
test('optional HTTP denies missing/wrong authorization, unsafe Host/Origin, query paths and oversized bodies', async () => {
  let calls = 0;
  const server = await startHttpServer({ generate: async () => { calls++; return { text: 'Fiction.' }; },
    bearerToken: TOKEN, port: 0 });
  const port = server.address().port;
  const address = 'http://127.0.0.1:' + port + '/mcp';
  const headers = { 'content-type': 'application/json', authorization: 'Bearer ' + TOKEN,
    accept: 'application/json, text/event-stream' };
  const body = JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call',
    params: { name: 'generate_text', arguments: { text: 'Fiction.' } } });
  try {
    const cases = [
      { status: 401, headers: { ...headers, authorization: '' } },
      { status: 401, headers: { ...headers, authorization: 'Bearer FAKE_WRONG_TOKEN_12345678901234567890' } },
      { status: 403, headers: { ...headers, host: 'unallowed.invalid' } },
      { status: 403, headers: { ...headers, origin: 'https://unallowed.invalid' } },
      { status: 403, headers: { ...headers, origin: 'null' } },
    ];
    for (const item of cases) {
      const result = await new Promise((resolve, reject) => {
        const request = http.request(address, { method: 'POST', headers: item.headers }, response => {
          const chunks = []; response.on('data', chunk => chunks.push(chunk));
          response.on('end', () => resolve({ status: response.statusCode, text: Buffer.concat(chunks).toString() }));
        });
        request.on('error', reject); request.end(body);
      });
      assert.equal(result.status, item.status);
      assert.equal(result.text.includes(TOKEN), false);
    }
    assert.equal((await fetch(address + '?token=FAKE_QUERY_TOKEN',
      { method: 'POST', headers, body })).status, 404);
    assert.equal((await fetch(address, { method: 'POST', headers,
      body: 'a'.repeat(POLICY.maxMcpMessageBytes + 1) })).status, 400);
    assert.equal((await fetch(address, { method: 'POST', headers, body: '[]' })).status, 400);
    assert.equal((await fetch(address, { method: 'GET', headers })).status, 405);
    assert.equal(calls, 0);
  } finally {
    await new Promise(resolve => { server.close(resolve); server.closeAllConnections(); });
  }
});

test('default stdio entry works with the official subprocess MCP client and fictional provider', async () => {
  const transport = new StdioClientTransport({ command: process.execPath, args: ['fixtures/fake-stdio.mjs'], stderr: 'pipe' });
  let errors = '';
  transport.stderr?.on('data', chunk => { errors += chunk.toString(); });
  const client = new Client({ name: 'fictional-stdio-client', version: '1.0.0' });
  try {
    await client.connect(transport);
    const tools = await client.listTools();
    assert.deepEqual(tools.tools.map(tool => tool.name), ['generate_text']);
    const result = await client.callTool({ name: 'generate_text', arguments: { text: 'Fiction.' } });
    assert.deepEqual(result.structuredContent, { text: 'Fictional stdio output.' });
    assert.equal(errors, '');
  } finally { await client.close(); }
});

test('stdio EOF cancels an admitted provider request', async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  output.resume();
  let start;
  let aborted;
  const started = new Promise(resolve => { start = resolve; });
  const cancelled = new Promise(resolve => { aborted = resolve; });
  const generate = createGenerator({ apiKey: KEY, transport: async ({ signal }) => {
    signal.addEventListener('abort', aborted, { once: true });
    start();
    return new Promise(() => {});
  } });
  const server = await startStdio({ generate, input, output });
  try {
    input.write(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize',
      params: { protocolVersion: '2025-03-26', capabilities: {},
        clientInfo: { name: 'fictional-eof-test', version: '1.0.0' } } }) + '\n');
    input.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
    input.write(JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'tools/call',
      params: { name: 'generate_text', arguments: { text: 'Fiction.' } } }) + '\n');
    await started;
    input.end();
    await Promise.race([cancelled, new Promise((_, reject) =>
      setTimeout(() => reject(new Error('stdio did not cancel')), 500))]);
  } finally { await server.close(); input.destroy(); output.destroy(); }
});
test('optional HTTP forwards cancellation across stateless requests', async () => {
  let start;
  let capturedSignal;
  const started = new Promise(resolve => { start = resolve; });
  const generate = createGenerator({ apiKey: KEY, transport: async ({ signal }) => {
    capturedSignal = signal; start(); return new Promise(() => {});
  } });
  const server = await startHttpServer({ generate, bearerToken: TOKEN, port: 0 });
  const address = 'http://127.0.0.1:' + server.address().port + '/mcp';
  const headers = { authorization: 'Bearer ' + TOKEN, 'content-type': 'application/json',
    accept: 'application/json, text/event-stream' };
  try {
    const pending = fetch(address, { method: 'POST', headers, body: JSON.stringify({
      jsonrpc: '2.0', id: 'fictional-pending-request', method: 'tools/call',
      params: { name: 'generate_text', arguments: { text: 'Fiction.' } },
    }) });
    await started;
    const cancellation = await fetch(address, { method: 'POST', headers, body: JSON.stringify({
      jsonrpc: '2.0', method: 'notifications/cancelled',
      params: { requestId: 'fictional-pending-request', reason: 'Fictional cancellation.' },
    }) });
    assert.equal(cancellation.status, 202);
    assert.equal(capturedSignal.aborted, true);
    const response = await pending;
    assert.equal((await response.json()).result.isError, true);
  } finally {
    await new Promise(resolve => { server.close(resolve); server.closeAllConnections(); });
  }
});

test('abrupt stdio input close cancels an admitted provider request', async () => {
  const input = new PassThrough();
  const output = new PassThrough();
  output.resume();
  let start;
  let aborted;
  const started = new Promise(resolve => { start = resolve; });
  const cancelled = new Promise(resolve => { aborted = resolve; });
  const generate = createGenerator({ apiKey: KEY, transport: async ({ signal }) => {
    signal.addEventListener('abort', aborted, { once: true });
    start();
    return new Promise(() => {});
  } });
  const server = await startStdio({ generate, input, output });
  try {
    input.write(JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'initialize',
      params: { protocolVersion: '2025-03-26', capabilities: {},
        clientInfo: { name: 'fictional-eof-test', version: '1.0.0' } } }) + '\n');
    input.write(JSON.stringify({ jsonrpc: '2.0', method: 'notifications/initialized' }) + '\n');
    input.write(JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'tools/call',
      params: { name: 'generate_text', arguments: { text: 'Fiction.' } } }) + '\n');
    await started;
    input.destroy();
    await Promise.race([cancelled, new Promise((_, reject) =>
      setTimeout(() => reject(new Error('stdio did not cancel')), 500))]);
  } finally { await server.close(); input.destroy(); output.destroy(); }
});
