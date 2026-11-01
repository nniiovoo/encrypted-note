import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { PassThrough } from 'node:stream';
import { execFile, spawn } from 'node:child_process';
import { createInterface } from 'node:readline';
import { promisify } from 'node:util';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';
import { CompanionError, createGenerator, POLICY } from '../src/generate.mjs';
import { startHttpServer, stopHttpServer } from '../src/http.mjs';
import { MAX_GRANT_MS } from '../isolation/common.mjs';
import { connectLocalBroker, tokenBridge } from '../isolation/agent-bridge.mjs';
import {
  startIntegratedBroker, checkReadiness, encodeEvent, parseControlLine, dataPaths,
  checkHandoffDirectory, assertAgentNotAdmin, ERROR_CODES, MAX_CONTROL_LINE_BYTES,
} from '../isolation/integrated-broker.mjs';

const run = promisify(execFile);
const ENTRY = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'isolation', 'integrated-broker.mjs');
const BRIDGE = path.join(path.dirname(ENTRY), 'agent-bridge.mjs');
const KEY = 'FAKE_OPENAI_AGENT_KEY_FOR_TESTS_ONLY';
const TOKEN = 'FAKE_DELEGATED_TOKEN_FOR_TESTS_ONLY_0123456789';
const MARKERS = [KEY, TOKEN, 'FAKE_PROVIDER_PRIVATE_ERROR', 'Bearer '];
const uid = process.getuid();
const T0 = Date.parse('2026-10-26T12:00:00.000Z');

async function fixture({ credential = 'OPENAI_AGENT_API_KEY=' + KEY + '\n', planChanges = {}, handoffMode = 0o700 } = {}) {
  // Real path: /var is a symlink, and protected-path checks refuse symlinked ancestors.
  const root = await fs.mkdtemp(path.join(await fs.realpath(os.tmpdir()), 'enote-companion-integrated-'));
  const home = path.join(root, 'home');
  const handoff = path.join(root, 'handoff');
  const paths = dataPaths(home);
  await fs.mkdir(paths.dir, { recursive: true, mode: 0o700 });
  await fs.mkdir(handoff, { mode: handoffMode });
  await fs.chmod(handoff, handoffMode);
  const plan = {
    version: 2, ownerUid: uid, agentUid: uid + 1, agentShortName: 'enote-agent',
    credentialExpirySeconds: 3600, projectHardSpendLimitCents: 100, projectHardSpendCurrency: 'USD',
    projectHardSpendInterval: 'month', brokerHost: '127.0.0.1', brokerPort: 8789,
    providerModel: 'gpt-6-luna', providerScope: 'api.responses.write',
    bearerTokenFile: path.join(handoff, 'agent-token'), grantExpiresAt: null, ...planChanges,
  };
  await fs.writeFile(paths.plan, JSON.stringify(plan), { mode: 0o600 });
  if (credential !== null) {
    await fs.writeFile(paths.credential, credential, { mode: 0o600 });
    await fs.chmod(paths.credential, 0o600);
  }
  return { root, home, handoff, paths, plan, cleanup: () => fs.rm(root, { recursive: true, force: true }) };
}
function fakeIo(overrides = {}) {
  const acl = [];
  return {
    acl,
    io: {
      stat: fs.lstat,
      aclCheck: async (file, permitted) => { acl.push([file, permitted]); },
      addReadAcl: async () => {},
      verifyCode: async () => {},
      assertAgentNotAdmin: async () => {},
      ...overrides,
    },
  };
}
function provider(calls, response) {
  return async ({ apiKey }) => {
    calls.push(apiKey === KEY);
    return response ?? { status: 200, json: { model: POLICY.model, status: 'completed',
      output: [{ type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'Fictional output.' }] }] } };
  };
}
async function start(fx, { calls = [], response, io, now, mono, setTimer } = {}) {
  const lines = [];
  const input = new PassThrough();
  const broker = await startIntegratedBroker({
    home: fx.home, uid, transport: provider(calls, response), io: io ?? fakeIo().io, input,
    emit: line => lines.push(line), createToken: () => TOKEN, port: 0, signals: { once() {}, removeListener() {} },
    ...(now ? { now } : {}), ...(mono ? { mono } : {}), ...(setTimer ? { setTimer, clearTimer: () => {} } : {}),
  });
  return { broker, lines, input, calls, url: 'http://127.0.0.1:' + broker.server.address().port + '/mcp' };
}
async function callTool(url, token, args = { text: 'Fiction.' }) {
  const client = new Client({ name: 'fictional-integrated-test', version: '1.0.0' });
  await client.connect(new StreamableHTTPClientTransport(new URL(url), {
    requestInit: { headers: { authorization: 'Bearer ' + token } },
  }));
  try { return await client.callTool({ name: 'generate_text', arguments: args }); }
  finally { await client.close(); }
}
const parsed = lines => lines.map(line => JSON.parse(line));
const noMarkers = text => MARKERS.every(marker => !text.includes(marker));

test('control protocol accepts exactly one stop command and encodes only fixed fields', () => {
  assert.equal(parseControlLine('{"v":1,"op":"stop"}'), 'stop');
  for (const line of ['', 'stop', '{"v":1,"op":"start"}', '{"v":2,"op":"stop"}', '{"v":1,"op":"stop","x":1}',
    '[]', 'null', '{"v":1,"op":"stop"' + ' '.repeat(MAX_CONTROL_LINE_BYTES) + '}']) {
    assert.throws(() => parseControlLine(line));
  }
  assert.equal(encodeEvent({ event: 'ready', expiresAt: 1, extra: KEY }), '{"v":1,"event":"ready","expiresAt":1}\n');
  assert.equal(encodeEvent({ event: 'error', code: KEY }), '{"v":1,"event":"error","code":"internal"}\n');
  assert.equal(encodeEvent({ event: 'stopped', reason: KEY }), '{"v":1,"event":"error","code":"internal"}\n');
  assert.equal(encodeEvent({ event: 'check', plan: 'yes', credential: true, secret: KEY }),
    '{"v":1,"event":"check","plan":false,"credential":true,"handoff":false,"code":false,"agent":false}\n');
  assert.equal(encodeEvent({ event: 'unknown' }), '{"v":1,"event":"error","code":"internal"}\n');
});

test('start issues one fresh grant, hands off a private token and serves exactly the fixed operation', async () => {
  const fx = await fixture();
  const { acl, io } = fakeIo();
  try {
    const { broker, lines, input, calls, url } = await start(fx, { io });
    const ready = parsed(lines)[0];
    assert.equal(ready.event, 'ready');
    assert.ok(ready.expiresAt * 1000 <= Date.now() + MAX_GRANT_MS && ready.expiresAt * 1000 > Date.now());
    const info = await fs.stat(fx.plan.bearerTokenFile);
    assert.equal(info.mode & 0o777, 0o600);
    assert.equal((await fs.readFile(fx.plan.bearerTokenFile, 'utf8')).trim(), TOKEN);
    assert.deepEqual(acl.find(([file]) => file === fx.plan.bearerTokenFile), [fx.plan.bearerTokenFile, 'user:enote-agent allow read']);
    assert.deepEqual(acl.find(([file]) => file === fx.handoff), [fx.handoff, 'user:enote-agent allow search']);
    const result = await callTool(url, TOKEN);
    assert.deepEqual(result.structuredContent, { text: 'Fictional output.' });
    // Direct loopback calls that bypass the agent bridge meet the same authorization.
    const wrong = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json',
      authorization: 'Bearer FAKE_WRONG_TOKEN_0123456789012345678901', accept: 'application/json, text/event-stream' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name: 'generate_text', arguments: { text: 'x' } } }) });
    assert.equal(wrong.status, 401);
    const extra = await callTool(url, TOKEN, { text: 'Fiction.', model: 'other', url: 'https://unallowed.invalid/' });
    assert.equal(extra.isError, true);
    assert.deepEqual(calls, [true]);
    input.write('{"v":1,"op":"stop"}\n');
    assert.equal(await broker.done, 'owner');
    assert.deepEqual(parsed(lines).at(-1), { v: 1, event: 'stopped', reason: 'owner' });
    await assert.rejects(fs.stat(fx.plan.bearerTokenFile), { code: 'ENOENT' });
    await assert.rejects(fetch(url, { method: 'POST' }));
    assert.ok(noMarkers(lines.join('')));
  } finally { await fx.cleanup(); }
});

test('control pipe EOF, malformed control and oversized control all stop the broker', async () => {
  for (const [send, reason] of [[null, 'parent'], ['{"v":1,"op":"restart"}\n', 'error'], ['x'.repeat(1024), 'error']]) {
    const fx = await fixture();
    try {
      const { broker, lines, input } = await start(fx);
      if (send === null) input.end(); else input.write(send);
      assert.equal(await broker.done, reason);
      assert.deepEqual(parsed(lines).at(-1), { v: 1, event: 'stopped', reason });
      await assert.rejects(fs.stat(fx.plan.bearerTokenFile), { code: 'ENOENT' });
    } finally { await fx.cleanup(); }
  }
});

test('expiry at the broker denies direct calls, reaches no provider and stops at the deadline', async () => {
  const fx = await fixture();
  let wall = Date.now(); let mono = 0;
  const timers = [];
  let running;
  try {
    running = await start(fx, { now: () => wall, mono: () => mono,
      setTimer: (callback, delay) => { timers.push({ callback, delay }); return timers.length; } });
    const { broker, lines, calls, url } = running;
    assert.equal(timers.length, 1);
    assert.ok(timers[0].delay <= MAX_GRANT_MS && timers[0].delay > MAX_GRANT_MS - 5000);
    // The ready event reports whole seconds; one second later the absolute deadline has passed.
    const expiresAt = (parsed(lines)[0].expiresAt + 1) * 1000;
    wall = expiresAt;
    const denied = await callTool(url, TOKEN);
    assert.equal(denied.isError, true);
    assert.match(denied.content[0].text, /grant has ended/);
    wall = expiresAt - 60_000; mono = MAX_GRANT_MS; // clock rolled back, but an hour of monotonic time passed
    assert.equal((await callTool(url, TOKEN)).isError, true);
    assert.deepEqual(calls, []);
    timers[0].callback();
    assert.equal(await broker.done, 'expired');
    assert.deepEqual(parsed(lines).at(-1), { v: 1, event: 'stopped', reason: 'expired' });
    await assert.rejects(fs.stat(fx.plan.bearerTokenFile), { code: 'ENOENT' });
  } finally { await running?.broker.stop('owner'); await fx.cleanup(); }
});

// A real process on a real stdin pipe, doing what the entry point does: wait for done, then return.
// The parent never closes that pipe, as the helper doesn't until the process has ended.
function brokerProcess(fx, { expireDuringCall = false } = {}) {
  const source = `
    import fs from 'node:fs/promises';
    import { startIntegratedBroker } from ${JSON.stringify(pathToFileURL(ENTRY).href)};
    let expire = () => {};
    const { done, server } = await startIntegratedBroker({
      home: ${JSON.stringify(fx.home)}, uid: ${uid}, port: 0, createToken: () => ${JSON.stringify(TOKEN)},
      io: { stat: fs.lstat, aclCheck: async () => {}, addReadAcl: async () => {},
        verifyCode: async () => {}, assertAgentNotAdmin: async () => {} },
      // Never answers, so this call is still in flight when the broker stops.
      transport: () => { expire(); return new Promise(() => {}); },
      ${expireDuringCall ? 'setTimer: callback => { expire = callback; return 0; },' : ''}
    });
    fetch('http://127.0.0.1:' + server.address().port + '/mcp', { method: 'POST',
      headers: { authorization: 'Bearer ${TOKEN}', 'content-type': 'application/json',
        accept: 'application/json, text/event-stream' },
      body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call',
        params: { name: 'generate_text', arguments: { text: 'Fiction.' } } }) }).catch(() => {});
    await done;
    process.exitCode = 0;
  `;
  return observed(spawn(process.execPath, ['--input-type=module', '-e', source], { stdio: ['pipe', 'pipe', 'pipe'] }));
}
function observed(child) {
  const output = { stdout: '', stderr: '' };
  child.stderr.on('data', chunk => { output.stderr += chunk; });
  // 'close' comes after the process has ended and its output has been read, and doesn't wait for stdin.
  const closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
  const ready = new Promise((resolve, reject) => {
    child.stdout.on('data', chunk => {
      output.stdout += chunk;
      if (output.stdout.includes('"ready"')) resolve();
    });
    void closed.then(() => reject(new Error('The broker process ended before ready.')));
  });
  return { child, output, ready, closed };
}
test('the broker process exits after its final event while the control pipe stays open', { timeout: 300_000 }, async () => {
  await Promise.all([
    ['expired', { expireDuringCall: true }, () => {}],
    ['owner', {}, child => child.stdin.write('{"v":1,"op":"stop"}\n')],
    ['owner', {}, child => child.kill('SIGTERM')],
    ['error', {}, child => child.stdin.write('{"v":1,"op":"restart"}\n')],
  ].map(async ([reason, options, act]) => {
    const fx = await fixture();
    const { child, output, ready, closed } = brokerProcess(fx, options);
    let timer;
    try {
      await ready;
      act(child);
      const ended = await Promise.race([closed, new Promise(resolve => {
        timer = setTimeout(() => resolve('still running'), 30_000);
      })]);
      assert.deepEqual(ended, { code: 0, signal: null }, reason);
      assert.deepEqual(parsed(output.stdout.trimEnd().split('\n')).at(-1), { v: 1, event: 'stopped', reason });
      assert.equal(output.stderr, '');
      assert.ok(noMarkers(output.stdout));
      await assert.rejects(fs.stat(fx.plan.bearerTokenFile), { code: 'ENOENT' });
    } finally {
      clearTimeout(timer);
      child.kill('SIGKILL');
      await closed;
      await fx.cleanup();
    }
  }));
});

// The real file with its real checks: fictional plan and key, real ACLs, the reviewed port and a
// stdin pipe that is never closed. dsmemberutil needs an account that exists, so the plan names
// `nobody`, which is not an administrator. The one seam is --import, passed only here: it shortens
// the one timer set for over two minutes, the grant's lifetime. The broker itself has no such switch.
const SHORT_GRANT_MS = 5000;
const SHORTEN_GRANT_TIMER = 'data:text/javascript,' + encodeURIComponent(`
  const realSetTimeout = globalThis.setTimeout;
  globalThis.setTimeout = (callback, delay, ...rest) =>
    realSetTimeout(callback, delay > 120000 ? ${SHORT_GRANT_MS} : delay, ...rest);
`);
test('the real entry point exits by itself and removes the token when its grant expires', {
  skip: process.platform !== 'darwin' && 'Needs the Mac account and ACL tools.', timeout: 300_000,
}, async t => {
  const fx = await fixture({ planChanges: { agentShortName: 'nobody' } });
  await run('/bin/chmod', ['+a', 'user:nobody allow search', fx.handoff]);
  const { child, output, ready, closed } = observed(spawn(process.execPath, ['--import', SHORTEN_GRANT_TIMER, ENTRY], {
    env: { HOME: fx.home, PATH: '/usr/bin:/bin' }, stdio: ['pipe', 'pipe', 'pipe'],
  }));
  let timer;
  try {
    try { await ready; }
    catch {
      const { code } = JSON.parse(output.stdout);
      // Not a broker fault: the port is in use here, or this checkout or Node isn't protected.
      if (['port_unavailable', 'code_unprotected'].includes(code)) { t.skip('The real checks refused: ' + code); return; }
      assert.fail('The real entry point refused: ' + code);
    }
    const token = (await fs.readFile(fx.plan.bearerTokenFile, 'utf8')).trim();
    // The bridge's own route to the fixed port. Listing makes no provider call; nothing here calls the tool.
    const client = await connectLocalBroker(token);
    try { assert.deepEqual((await client.listTools()).tools.map(tool => tool.name), ['generate_text']); }
    finally { await client.close(); }
    const ended = await Promise.race([closed, new Promise(resolve => {
      timer = setTimeout(() => resolve('still running'), 30_000);
    })]);
    assert.deepEqual(ended, { code: 0, signal: null });
    assert.equal(child.stdin.writableEnded, false);
    const events = parsed(output.stdout.trimEnd().split('\n'));
    assert.deepEqual(events.map(event => event.event), ['ready', 'stopped']);
    assert.deepEqual(events[1], { v: 1, event: 'stopped', reason: 'expired' });
    assert.equal(output.stderr, '');
    assert.equal(output.stdout.includes(token), false);
    assert.ok(noMarkers(output.stdout));
    await assert.rejects(fs.stat(fx.plan.bearerTokenFile), { code: 'ENOENT' });
    // The agent's next call: with the token gone, the bridge connects to nothing.
    await assert.rejects(tokenBridge(fx.plan.bearerTokenFile, uid).generate({ text: 'Fiction.' }), { code: 'helper_off' });
  } finally {
    clearTimeout(timer);
    child.kill('SIGKILL');
    await closed;
    await fx.cleanup();
  }
});

// The real bridge file over real stdio and the fixed port, with the real HTTP server and a fictional
// provider as the broker. Only its account check is stood in for, through --import: this Mac's account
// is an administrator, so a run in a real Standard account stays a hands-on check.
const STAND_IN_AGENT_ACCOUNT = 'data:text/javascript,' + encodeURIComponent(`
  import childProcess from 'node:child_process';
  import { syncBuiltinESMExports } from 'node:module';
  import { promisify } from 'node:util';
  const execFile = () => { throw new Error('Not used by the bridge.'); };
  execFile[promisify.custom] = async (file, args) => {
    if (file !== '/usr/bin/id') throw Object.assign(new Error('No sudo.'), { code: 1 });
    return { stdout: args[0] === '-Gn' ? 'staff\\n' : 'enote-agent\\n', stderr: '' };
  };
  childProcess.execFile = execFile;
  syncBuiltinESMExports();
  process.getuid = () => ${uid + 1};
`);
const portFree = port => new Promise(resolve => {
  const probe = net.createServer();
  probe.once('error', () => resolve(false));
  probe.listen(port, '127.0.0.1', () => probe.close(() => resolve(true)));
});
test('the real bridge file serves the tool with the helper off, follows each Start and leaves with its client', { timeout: 120_000 }, async t => {
  const fx = await fixture();
  // Checked first, so no fictional token is ever offered to whatever already holds the port.
  if (!await portFree(fx.plan.brokerPort)) {
    await fx.cleanup();
    t.skip('Port 8789 is in use here.');
    return;
  }
  const secondToken = 'FAKE_SECOND_DELEGATED_TOKEN_FOR_TESTS_ONLY_0123';
  const helperOff = new CompanionError('helper_off').message;
  const startBroker = async token => {
    await fs.writeFile(fx.plan.bearerTokenFile, token + '\n', { mode: 0o600 });
    return startHttpServer({ generate: createGenerator({ apiKey: KEY, transport: provider([]) }),
      bearerToken: token, port: fx.plan.brokerPort });
  };
  const stopBroker = server => new Promise(resolve => { server.once('close', resolve); stopHttpServer(server); });
  const child = spawn(process.execPath, ['--import', STAND_IN_AGENT_ACCOUNT, BRIDGE, fx.paths.plan],
    { env: { PATH: '/usr/bin:/bin' }, stdio: ['pipe', 'pipe', 'pipe'] });
  const output = { stdout: '', stderr: '' };
  child.stderr.on('data', chunk => { output.stderr += chunk; });
  const closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
  const waiting = new Map();
  createInterface({ input: child.stdout }).on('line', line => {
    output.stdout += line + '\n';
    const { id, result } = JSON.parse(line);
    waiting.get(id)?.(result);
  });
  const send = message => child.stdin.write(JSON.stringify({ jsonrpc: '2.0', ...message }) + '\n');
  let nextId = 0;
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = ++nextId;
    waiting.set(id, resolve);
    void closed.then(() => reject(new Error('The bridge ended early.')));
    send({ id, method, params });
  });
  const call = async () => {
    const result = await request('tools/call', { name: 'generate_text', arguments: { text: 'Fiction.' } });
    return result.isError ? result.content[0].text : result.structuredContent.text;
  };
  let server;
  let timer;
  try {
    await request('initialize', { protocolVersion: '2025-06-18', capabilities: {},
      clientInfo: { name: 'fictional-agent', version: '1.0.0' } });
    send({ method: 'notifications/initialized' });
    // Helper off, no token: the tool is listed and a call says what to do.
    assert.deepEqual((await request('tools/list', {})).tools.map(tool => tool.name), ['generate_text']);
    assert.equal(await call(), helperOff);
    // Token written but no listener.
    await fs.writeFile(fx.plan.bearerTokenFile, TOKEN + '\n', { mode: 0o600 });
    assert.equal(await call(), helperOff);
    await fs.rm(fx.plan.bearerTokenFile);
    server = await startBroker(TOKEN);
    assert.equal(await call(), 'Fictional output.');
    // Stop removes the token; the next Start has a new one. Same process, no restart.
    await fs.rm(fx.plan.bearerTokenFile);
    await stopBroker(server);
    assert.equal(await call(), helperOff);
    server = await startBroker(secondToken);
    assert.equal(await call(), 'Fictional output.');
    // Still connected when the agent closes its end, the normal way out.
    child.stdin.end();
    const ended = await Promise.race([closed, new Promise(resolve => {
      timer = setTimeout(() => resolve('still running'), 30_000);
    })]);
    assert.deepEqual(ended, { code: 0, signal: null });
    assert.equal(output.stderr, '');
    for (const secret of [KEY, TOKEN, secondToken, 'Bearer ']) assert.equal(output.stdout.includes(secret), false);
  } catch (error) {
    if (error.code !== 'EADDRINUSE') throw error;
    t.skip('Port 8789 was taken during this test.');
  } finally {
    clearTimeout(timer);
    child.kill('SIGKILL');
    await closed;
    if (server?.listening) await stopBroker(server);
    await fx.cleanup();
  }
});

test('every refusal path returns one fixed code and leaves no token behind', async () => {
  const cases = [
    ['plan_missing', async fx => fs.unlink(fx.paths.plan)],
    ['plan_invalid', async fx => fs.writeFile(fx.paths.plan, JSON.stringify({ ...fx.plan, version: 1 }))],
    ['plan_invalid', async fx => fs.writeFile(fx.paths.plan, '{"FAKE":')],
    ['wrong_identity', async fx => fs.writeFile(fx.paths.plan, JSON.stringify({ ...fx.plan, ownerUid: uid + 2, agentUid: uid + 3 }))],
    ['credential_missing', async fx => fs.unlink(fx.paths.credential)],
    ['credential_invalid', async fx => fs.writeFile(fx.paths.credential, 'OPENAI_API_KEY=' + KEY + '\n')],
    ['credential_invalid', async fx => fs.chmod(fx.paths.credential, 0o644)],
    ['handoff_unavailable', async fx => fs.chmod(fx.handoff, 0o755)],
    ['handoff_unavailable', async fx => fs.rm(fx.handoff, { recursive: true })],
  ];
  for (const [code, change] of cases) {
    const fx = await fixture();
    try {
      await change(fx);
      await assert.rejects(start(fx), error => error.code === code, code);
      await assert.rejects(fs.stat(fx.plan.bearerTokenFile), { code: 'ENOENT' });
    } finally { await fx.cleanup(); }
  }
  for (const [code, override] of [
    ['code_unprotected', { verifyCode: async () => { throw new Error('FAKE'); } }],
    ['agent_unverified', { assertAgentNotAdmin: async () => { throw new Error('FAKE'); } }],
    ['handoff_unavailable', { addReadAcl: async () => { throw new Error('FAKE'); } }],
  ]) {
    const fx = await fixture();
    try {
      await assert.rejects(start(fx, { io: fakeIo(override).io }), error => error.code === code, code);
      await assert.rejects(fs.stat(fx.plan.bearerTokenFile), { code: 'ENOENT' });
    } finally { await fx.cleanup(); }
  }
  assert.ok(ERROR_CODES.includes('port_unavailable'));
});

test('a busy reviewed port fails closed and removes the handed-off token', async () => {
  const fx = await fixture();
  const blocker = net.createServer();
  await new Promise(resolve => blocker.listen(0, '127.0.0.1', resolve));
  try {
    const lines = [];
    await assert.rejects(startIntegratedBroker({ home: fx.home, uid, transport: provider([]), io: fakeIo().io,
      input: new PassThrough(), emit: line => lines.push(line), createToken: () => TOKEN,
      port: blocker.address().port, signals: { once() {}, removeListener() {} } }), error => error.code === 'port_unavailable');
    await assert.rejects(fs.stat(fx.plan.bearerTokenFile), { code: 'ENOENT' });
  } finally { blocker.close(); await fx.cleanup(); }
});

test('fictional key, token and provider-error markers never reach protocol output or tool results', async () => {
  const fx = await fixture();
  try {
    const failing = await start(fx, { response: { status: 500, json: { error: 'FAKE_PROVIDER_PRIVATE_ERROR ' + KEY } } });
    const failed = await callTool(failing.url, TOKEN);
    assert.equal(failed.isError, true);
    assert.ok(noMarkers(JSON.stringify(failed)));
    failing.input.end(); await failing.broker.done;
    const leaking = await start(fx, { response: { status: 200, json: { model: POLICY.model, status: 'completed',
      output: [{ type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'echo ' + KEY }] }] } } });
    const echoed = await callTool(leaking.url, TOKEN);
    assert.equal(echoed.isError, true);
    assert.ok(noMarkers(JSON.stringify(echoed)));
    leaking.input.end(); await leaking.broker.done;
    assert.ok(noMarkers(failing.lines.join('') + leaking.lines.join('')));
  } finally { await fx.cleanup(); }
});

test('readiness reports booleans only and never reads the credential contents', async () => {
  const fx = await fixture();
  try {
    assert.deepEqual(await checkReadiness({ home: fx.home, uid, io: fakeIo().io }),
      { plan: true, credential: true, handoff: true, code: true, agent: true });
    await fs.chmod(fx.paths.credential, 0o644);
    assert.equal((await checkReadiness({ home: fx.home, uid, io: fakeIo().io })).credential, false);
    assert.deepEqual(await checkReadiness({ home: fx.home, uid, io: fakeIo({ assertAgentNotAdmin: async () => { throw new Error('FAKE'); } }).io }),
      { plan: true, credential: false, handoff: true, code: false, agent: false });
    await fs.unlink(fx.paths.plan);
    assert.deepEqual(await checkReadiness({ home: fx.home, uid, io: fakeIo().io }),
      { plan: false, credential: false, handoff: false, code: false, agent: false });
  } finally { await fx.cleanup(); }
});

test('handoff directory must be owner-only with exactly the agent search entry', async () => {
  const fx = await fixture();
  try {
    await checkHandoffDirectory(fx.plan, { aclCheck: async () => {} });
    await assert.rejects(checkHandoffDirectory(fx.plan, {
      aclCheck: async (file, permitted) => { if (file === fx.handoff) assert.equal(permitted, 'user:enote-agent allow search'); throw new Error('FAKE ACL'); },
    }));
  } finally { await fx.cleanup(); }
});

test('the agent account must be confirmed not to be an administrator', async () => {
  const plan = { agentShortName: 'enote-agent' };
  await assertAgentNotAdmin(plan, async () => ({ stdout: 'user is not a member of the group\n' }));
  await assert.rejects(assertAgentNotAdmin(plan, async () => ({ stdout: 'user is a member of the group\n' })));
  await assert.rejects(assertAgentNotAdmin(plan, async () => { throw new Error('FAKE'); }));
});

test('the production entry point prints one fixed protocol line and nothing else', async () => {
  const empty = await fs.mkdtemp(path.join(os.tmpdir(), 'enote-companion-entry-'));
  try {
    const env = { HOME: empty, PATH: '/usr/bin:/bin' };
    const started = await run(process.execPath, [ENTRY], { env }).catch(error => error);
    assert.equal(started.stdout, '{"v":1,"event":"error","code":"plan_missing"}\n');
    assert.equal(started.stderr, '');
    const checked = await run(process.execPath, [ENTRY, '--check'], { env });
    assert.equal(checked.stdout, '{"v":1,"event":"check","plan":false,"credential":false,"handoff":false,"code":false,"agent":false}\n');
    const extra = await run(process.execPath, [ENTRY, '--provider', 'https://unallowed.invalid'], { env }).catch(error => error);
    assert.equal(extra.stdout, '{"v":1,"event":"error","code":"internal"}\n');
  } finally { await fs.rm(empty, { recursive: true, force: true }); }
});
