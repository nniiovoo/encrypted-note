import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { fileURLToPath } from 'node:url';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { InMemoryTransport } from '@modelcontextprotocol/sdk/inMemory.js';
import { validatePlan, requireRole, parseConfirmedCredential, readBounded,
  validateAclListing, validateAclBatchListing, verifyCodeTree, assertProtectedPath,
  assertStandardIdentity, assertGrantActive, grantGuard, MAX_GRANT_MS } from '../isolation/common.mjs';
import { CompanionError, createGenerator, errorFromPublicText, guardGenerate, POLICY } from '../src/generate.mjs';
import { createMcpServer } from '../src/mcp.mjs';
import { startHttpServer, stopHttpServer } from '../src/http.mjs';
import { prepareOwnerFixture, verifyOwnerFixture, probeOwnerFixture } from '../isolation/fixtures.mjs';
import { bridgeGenerator, connectLocalBroker, tokenBridge } from '../isolation/agent-bridge.mjs';
const ownerUid = process.getuid();
const plan = {
  version: 2, ownerUid, agentUid: ownerUid + 1, agentShortName: 'enote-agent',
  credentialExpirySeconds: 3600, projectHardSpendLimitCents: 100,
  projectHardSpendCurrency: 'USD', projectHardSpendInterval: 'month',
  brokerHost: '127.0.0.1', brokerPort: 8789, providerModel: 'gpt-6-luna',
  providerScope: 'api.responses.write', bearerTokenFile: '/Users/Shared/FAKE-TOKEN-DIRECTORY/token',
  grantExpiresAt: null,
};
const T0 = Date.parse('2026-10-26T12:00:00.000Z');
test('isolation plan rejects missing identity, looser trial controls and unreviewed destinations', () => {
  validatePlan(plan);
  for (const changed of [
    { ownerUid: null }, { agentUid: ownerUid }, { agentUid: 0 }, { bearerTokenFile: null },
    { bearerTokenFile: '/Users/Shared/../unreviewed' }, { credentialExpirySeconds: 7200 },
    { projectHardSpendLimitCents: 200 }, { brokerHost: '0.0.0.0' }, { brokerPort: 443 },
    { providerModel: 'other' }, { providerScope: 'api.all.read' }, { unexpected: true },
    { grantExpiresAt: 'tomorrow' }, { grantExpiresAt: '2026-10-26T12:00:00+01:00' },
    { grantExpiresAt: 1793016000000 }, { version: 3 },
  ]) assert.throws(() => validatePlan({ ...plan, ...changed }));
  const { grantExpiresAt: _omitted, ...missing } = plan;
  assert.throws(() => validatePlan(missing));
  assert.throws(() => requireRole(plan, 'agent', ownerUid));
  assert.throws(() => requireRole(plan, 'owner', 0));
});
test('confirmed credential parser accepts only one bounded separate-key assignment without eval/export', () => {
  const key = 'FAKE_OPENAI_AGENT_KEY_FOR_TESTS_ONLY';
  for (const value of [key, '"' + key + '"', "'" + key + "'"]) {
    assert.equal(parseConfirmedCredential('OPENAI_AGENT_API_KEY=' + value + '\n'), key);
  }
  for (const text of [
    'OPENAI_API_KEY=' + key, 'export OPENAI_AGENT_API_KEY=' + key,
    'OPENAI_AGENT_API_KEY=$(FAKE_COMMAND)', 'OPENAI_AGENT_API_KEY=' + key + '\nOTHER=FAKE',
    'OPENAI_AGENT_API_KEY=' + key + '\r\nFAKE', 'a'.repeat(2049),
  ]) assert.throws(() => parseConfirmedCredential(text));
});
test('bounded file reader rejects growth, oversize and invalid UTF-8 using only fixed allocation', async () => {
  const before = { size: 4, mtimeMs: 1 };
  const handle = {
    read: async buffer => { buffer.write('FAKE'); return { bytesRead: 4 }; },
    stat: async () => before,
  };
  assert.equal(await readBounded(handle, 4, before), 'FAKE');
  await assert.rejects(readBounded({ ...handle, stat: async () => ({ size: 5, mtimeMs: 2 }) }, 4, before));
  await assert.rejects(readBounded({
    read: async buffer => { buffer.fill(65); return { bytesRead: 5 }; },
    stat: async () => ({ size: 5, mtimeMs: 1 }),
  }, 4, { size: 5, mtimeMs: 1 }));
  await assert.rejects(readBounded({
    read: async buffer => { buffer[0] = 255; return { bytesRead: 1 }; },
    stat: async () => ({ size: 1, mtimeMs: 1 }),
  }, 4, { size: 1, mtimeMs: 1 }));
});
test('ACL checks deny unexpected grants and permit only the exact delegated-token read/search entries', () => {
  validateAclListing('drwx------ owner\n 0: group:everyone deny delete\n');
  validateAclListing('-rw------- owner\n 0: user:enote-agent allow read\n', 'user:enote-agent allow read');
  validateAclListing('drwx------ owner\n 0: user:enote-agent allow search\n', 'user:enote-agent allow search');
  for (const entry of ['user:enote-agent allow write', 'group:staff allow read',
                       'user:enote-agent allow read,write', 'user:other allow read']) {
    assert.throws(() => validateAclListing('-rw------- owner\n 0: ' + entry + '\n',
      'user:enote-agent allow read'));
  }
});
test('protected path checks include controlling ancestors and symlinks', async () => {
  const checked = [];
  const good = async () => ({ uid: ownerUid, mode: 0o700, isSymbolicLink: () => false, isDirectory: () => true });
  await assertProtectedPath('/FAKE/parent/code', ownerUid, good,
    { aclCheck: async file => { checked.push(file); } });
  assert.deepEqual(checked, ['/FAKE/parent/code', '/FAKE/parent', '/FAKE', '/']);
  await assert.rejects(assertProtectedPath('/FAKE/code', ownerUid, async () => ({
    uid: ownerUid, mode: 0o777, isSymbolicLink: () => false, isDirectory: () => true,
  }), { aclCheck: async () => {} }));
  await assert.rejects(assertProtectedPath('/FAKE/code', ownerUid, async () => ({
    uid: ownerUid, mode: 0o700, isSymbolicLink: () => true, isDirectory: () => false,
  }), { aclCheck: async () => {} }));
});
test('agent identity checks do not treat an owner process or privileged account as isolated', async () => {
  await assert.rejects(assertStandardIdentity(plan, async () => { throw new Error('must not run'); }, ownerUid));
  const fake = async (_file, args) => {
    if (args[0] === '-Gn') return { stdout: 'staff everyone' };
    if (args[0] === '-un') return { stdout: 'enote-agent' };
    const error = new Error('FAKE permission denial'); error.code = 1; throw error;
  };
  await assertStandardIdentity(plan, fake, plan.agentUid);
  await assert.rejects(assertStandardIdentity(plan, async (_file, args) =>
    args[0] === '-Gn' ? { stdout: 'staff admin' } : { stdout: '' }, plan.agentUid));
  await assert.rejects(assertStandardIdentity(plan, async (_file, args) =>
    args[0] === '-Gn' ? { stdout: 'staff' } : args[0] === '-un' ? { stdout: 'enote-agent' } :
      { stdout: 'FAKE sudo authorization' }, plan.agentUid));
});
test('fictional denied-access probes require permission denial; missing fixtures and accidental access fail', async () => {
  const manifest = { version: 1, label: 'FAKE-ENOTE-ISOLATION-ONLY',
    ownerUid, agentUid: plan.agentUid, root: '/FAKE/enote-isolation-abc123' };
  const denied = async () => { const error = new Error('FAKE denial'); error.code = 'EACCES'; throw error; };
  const io = Object.fromEntries(['readFile', 'readdir', 'open', 'writeFile', 'rename', 'unlink'].map(name => [name, denied]));
  assert.equal((await probeOwnerFixture(manifest, io)).ownerAndCodeWriteDenied, true);
  await assert.rejects(probeOwnerFixture(manifest, { ...io, readFile: async () => Buffer.from('FAKE unexpectedly accessible') }));
  const missing = async () => { const error = new Error('FAKE missing'); error.code = 'ENOENT'; throw error; };
  await assert.rejects(probeOwnerFixture(manifest, { ...io, readFile: missing }));
  await assert.rejects(probeOwnerFixture({ ...manifest, root: '/etc' }, io));
});
test('new fictional fixtures have exact modes and owner verification detects changes or symlinks', async () => {
  const parent = await fs.mkdtemp(path.join(os.tmpdir(), 'enote-isolation-unit-'));
  try {
    const manifest = await prepareOwnerFixture(plan, parent, { protect: async () => {} });
    assert.equal((await fs.stat(manifest.root)).mode & 0o777, 0o700);
    assert.deepEqual(await verifyOwnerFixture(manifest), { ownerFixturesUnchanged: true });
    const member = path.join(manifest.root, 'owner-only-fake.txt');
    await fs.appendFile(member, 'FAKE unexpected change');
    await assert.rejects(verifyOwnerFixture(manifest));
    await fs.unlink(member);
    const otherFake = path.join(parent, 'other-fake.txt');
    await fs.writeFile(otherFake, 'FAKE only');
    await fs.symlink(otherFake, member);
    await assert.rejects(verifyOwnerFixture(manifest));
  } finally { await fs.rm(parent, { recursive: true, force: true }); }
});
test('agent bridge accepts only the bounded operation and redacts remote errors', async () => {
  let calls = 0;
  const client = { callTool: async args => { calls++; assert.equal(args.name, 'generate_text');
    return { structuredContent: { text: 'Fictional bridge output.' } }; } };
  const generate = bridgeGenerator(client);
  assert.deepEqual(await generate({ text: 'Fiction.' }), { text: 'Fictional bridge output.' });
  await assert.rejects(generate({ text: 'Fiction.', model: 'other' }));
  assert.equal(calls, 1);
  const failing = bridgeGenerator({ callTool: async () => { throw new Error('FAKE_PRIVATE_REMOTE_ERROR'); } });
  await assert.rejects(failing({ text: 'Fiction.' }), error => !error.message.includes('FAKE_PRIVATE_REMOTE_ERROR'));
});

const KEY = 'FAKE_OPENAI_AGENT_KEY_FOR_TESTS_ONLY';
const TOKEN_ONE = 'FAKE_DELEGATED_TOKEN_ONE_FOR_TESTS_ONLY_0123';
const TOKEN_TWO = 'FAKE_DELEGATED_TOKEN_TWO_FOR_TESTS_ONLY_0123';
const fixedText = code => new CompanionError(code).message;
const providerOutput = { status: 200, json: { model: POLICY.model, status: 'completed',
  output: [{ type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'Fictional bridge output.' }] }] } };
// An agent on the far side of the bridge's own MCP server.
async function agentThrough(generate) {
  const [clientTransport, serverTransport] = InMemoryTransport.createLinkedPair();
  const server = createMcpServer(generate);
  const agent = new Client({ name: 'fictional-agent', version: '1.0.0' });
  await server.connect(serverTransport);
  await agent.connect(clientTransport);
  return { agent,
    call: () => agent.callTool({ name: 'generate_text', arguments: { text: 'Fiction.' } }),
    close: async () => { await agent.close(); await server.close(); } };
}
const stopBroker = server => new Promise(resolve => { server.once('close', resolve); stopHttpServer(server); });
const errorText = result => { assert.equal(result.isError, true); return result.content[0].text; };

test('each fixed broker failure reaches the agent through the bridge with its own text and next step', async () => {
  let respond = async () => providerOutput;
  let active = true;
  const main = guardGenerate(createGenerator({ apiKey: KEY, now: () => 0, transport: () => respond() }), () => active);
  const timingOut = createGenerator({ apiKey: KEY, timeoutMs: 20, transport: () => new Promise(() => {}) });
  const cuttingOff = createGenerator({ apiKey: KEY, transport: async () => ({ status: 200, json: {
    model: POLICY.model, status: 'incomplete', incomplete_details: { reason: 'max_output_tokens' },
    output: [{ type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'FAKE_PRIVATE_MARKER cut off mid' }] }] } }) });
  let selected = main;
  const server = await startHttpServer({ generate: (...args) => selected(...args), bearerToken: TOKEN_ONE, port: 0 });
  const client = await connectLocalBroker(TOKEN_ONE, server.address().port);
  const { call, close } = await agentThrough(bridgeGenerator(client));
  const seen = [];
  const expectFailure = async (code, step) => {
    const text = errorText(await call());
    assert.equal(text, fixedText(code));
    assert.match(text, step);
    seen.push(text);
  };
  const askOwner = /Ask the owner to choose Start for up to one hour in encrypted-note\. Do not look for keys or tokens\./;
  try {
    respond = async () => ({ status: 401 });
    await expectFailure('provider_auth', /Tell the owner\. Do not retry\./);
    respond = async () => ({ status: 429 });
    await expectFailure('provider_limit', /Tell the owner\. Do not retry\./);
    respond = async () => ({ status: 500, json: { error: 'FAKE_PRIVATE_MARKER' } });
    await expectFailure('provider_unavailable', /Try once more at most\./);
    respond = async () => ({ status: 200, json: { model: 'FAKE_PRIVATE_MARKER' } });
    await expectFailure('invalid_response', /Try once more at most\./);
    selected = cuttingOff;
    await expectFailure('output_limit', /Ask for a shorter answer or split the request\. The same text will likely fail again\./);
    selected = timingOut;
    await expectFailure('timeout', /Try once more at most\./);
    selected = main;
    let entered; let release;
    const inFlight = new Promise(resolve => { entered = resolve; });
    const held = new Promise(resolve => { release = resolve; });
    respond = () => { entered(); return held; };
    const pending = call(); // the fifth admitted call on this generator
    await inFlight;
    await expectFailure('busy', /Wait for it to finish, then try again\./);
    release(providerOutput);
    assert.deepEqual((await pending).structuredContent, { text: 'Fictional bridge output.' });
    respond = async () => providerOutput;
    assert.equal((await call()).isError, undefined);
    await expectFailure('usage_limit', /Wait a minute, then try once more\. If it fails again, tell the owner\./);
    active = false;
    await expectFailure('grant_expired', askOwner);
    assert.equal(new Set(seen).size, seen.length);
    assert.equal(seen.join('\n').includes('FAKE_PRIVATE_MARKER'), false);
  } finally { await close(); await client.close(); await stopBroker(server); }
});
test('every fixed error says what to do next, and helper_off never says why the helper stopped', () => {
  const askOwner = /Ask the owner to choose Start for up to one hour in encrypted-note\. Do not look for keys or tokens\.$/;
  for (const [code, step] of [
    ['invalid_input', /^Provide one nonempty text string of at most 4096 UTF-8 bytes\.$/],
    ['busy', /Wait for it to finish, then try again\.$/],
    ['usage_limit', /Wait a minute, then try once more\. If it fails again, tell the owner\.$/],
    ['timeout', /Try once more at most\.$/],
    ['cancelled', /Send it again only if it is still needed\.$/],
    ['provider_auth', /Tell the owner\. Do not retry\.$/],
    ['provider_limit', /Tell the owner\. Do not retry\.$/],
    ['provider_unavailable', /Try once more at most\.$/],
    ['invalid_response', /Try once more at most\.$/],
    ['output_limit', /Ask for a shorter answer or split the request\. The same text will likely fail again\.$/],
    ['grant_expired', askOwner],
    ['helper_off', askOwner],
  ]) {
    assert.equal(new CompanionError(code).code, code);
    assert.match(fixedText(code), step);
  }
  assert.doesNotMatch(fixedText('helper_off'), /vault|lock|expire|stop/i);
});
test('the bridge passes only an exact fixed text; anything else from a broker becomes the generic text', async () => {
  const marker = 'FAKE_PRIVATE_MARKER';
  for (const result of [
    { isError: true, content: [{ type: 'text', text: marker }] },
    { isError: true, content: [{ type: 'text', text: fixedText('busy') + ' ' + marker }] },
    { isError: true, content: [{ type: 'text', text: 'busy' }] },
    { isError: true, content: [{ type: 'text', text: { toString: () => fixedText('busy') } }] },
    { isError: true, content: [] },
    { isError: true },
    { isError: true, structuredContent: { text: marker } },
    { structuredContent: { text: marker, extra: marker } },
    { content: [{ type: 'text', text: marker }] },
  ]) {
    const { call, close } = await agentThrough(bridgeGenerator({ callTool: async () => result }));
    try {
      const answer = await call();
      assert.equal(errorText(answer), fixedText('provider_unavailable'));
      assert.equal(JSON.stringify(answer).includes(marker), false);
    } finally { await close(); }
  }
  for (const text of [undefined, null, 42, '', '__proto__', 'constructor', 'helper_off']) {
    assert.equal(errorFromPublicText(text).code, 'provider_unavailable');
  }
  assert.equal(errorFromPublicText(fixedText('helper_off')).code, 'helper_off');
  assert.equal(errorFromPublicText(fixedText('output_limit')).code, 'output_limit');
  // Cancellation by the caller stays cancellation, with no broker text involved.
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(bridgeGenerator({ callTool: async () => { throw new Error(marker); } })(
    { text: 'Fiction.' }, { signal: controller.signal }), { code: 'cancelled' });
});
test('the bridge always serves the tool, follows each Start and shares one connection', { timeout: 120_000 }, async () => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'enote-bridge-unit-'));
  const tokenFile = path.join(directory, 'agent-token');
  const writeToken = async (token, mode = 0o600) => {
    await fs.rm(tokenFile, { force: true });
    await fs.writeFile(tokenFile, token + '\n', { mode });
    await fs.chmod(tokenFile, mode);
  };
  const port = await new Promise((resolve, reject) => {
    const probe = net.createServer();
    probe.once('error', reject);
    probe.listen(0, '127.0.0.1', () => { const free = probe.address().port; probe.close(() => resolve(free)); });
  });
  // Each Start is a new broker with new counters, on the same loopback port.
  const startBroker = (bearerToken, transport = async () => providerOutput) =>
    startHttpServer({ generate: createGenerator({ apiKey: KEY, transport }), bearerToken, port });
  let connects = 0;
  const bridge = tokenBridge(tokenFile, ownerUid, { connect: token => { connects++; return connectLocalBroker(token, port); } });
  const { agent, call, close } = await agentThrough(bridge.generate);
  const answers = [];
  const answer = async () => { const result = await call(); answers.push(JSON.stringify(result)); return result; };
  const helperOff = fixedText('helper_off');
  let server;
  try {
    // Helper off: the tool is listed, a call says what to do, nothing connects.
    assert.deepEqual((await agent.listTools()).tools.map(tool => tool.name), ['generate_text']);
    assert.equal(errorText(await answer()), helperOff);
    assert.match(helperOff, /Ask the owner to choose Start for up to one hour in encrypted-note\. Do not look for keys or tokens\./);
    await assert.rejects(bridge.generate({ text: ' ' }), { code: 'invalid_input' });
    assert.equal(connects, 0);
    // Token written, no listener yet: each call gets the fixed text and a failed connect isn't kept.
    // Sharing is checked below on a connection that succeeds: a refused one can end before a
    // slower parallel call has read the token, and that call then rightly tries again.
    await writeToken(TOKEN_ONE);
    assert.deepEqual((await Promise.all([answer(), answer(), answer()])).map(errorText), [helperOff, helperOff, helperOff]);
    const refused = connects;
    assert.ok(refused >= 1);
    // First Start: works, and the connection is kept while the token stays the same.
    server = await startBroker(TOKEN_ONE);
    assert.deepEqual((await answer()).structuredContent, { text: 'Fictional bridge output.' });
    assert.equal(connects, refused + 1);
    assert.deepEqual((await answer()).structuredContent, { text: 'Fictional bridge output.' });
    assert.equal(connects, refused + 1);
    // Stop removes the token: no connection attempt, and the old client is dropped.
    await fs.rm(tokenFile);
    await stopBroker(server);
    assert.equal(errorText(await answer()), helperOff);
    assert.equal(connects, refused + 1);
    // A new Start with a new token works through the same bridge.
    await writeToken(TOKEN_TWO);
    server = await startBroker(TOKEN_TWO);
    assert.deepEqual((await answer()).structuredContent, { text: 'Fictional bridge output.' });
    assert.equal(connects, refused + 2);
    // Broker gone but its token left behind.
    await stopBroker(server);
    assert.equal(errorText(await answer()), helperOff);
    // A token this broker doesn't accept is the same fixed text.
    server = await startBroker(TOKEN_ONE);
    assert.equal(errorText(await answer()), helperOff); // the same token, connected earlier, now refused
    await stopBroker(server);
    // After a token change, parallel calls share one new connection; the broker admits one of them.
    let release;
    const held = new Promise(resolve => { release = resolve; });
    await writeToken(TOKEN_ONE);
    server = await startBroker(TOKEN_ONE, () => held);
    const before = connects;
    const pending = [answer(), answer(), answer()];
    let settled = 0;
    for (const item of pending) void item.then(() => { if (++settled === 2) release(providerOutput); });
    const texts = (await Promise.all(pending)).map(result => result.isError ? errorText(result) : result.structuredContent.text);
    assert.deepEqual(texts.sort(), [fixedText('busy'), fixedText('busy'), 'Fictional bridge output.'].sort());
    assert.equal(connects, before + 1);
    // The same private-file checks as ever: a loose or symlinked token file is never used.
    const counted = connects;
    await writeToken(TOKEN_ONE, 0o644);
    assert.equal(errorText(await answer()), helperOff);
    await fs.rm(tokenFile);
    const elsewhere = path.join(directory, 'fake-elsewhere');
    await fs.writeFile(elsewhere, TOKEN_ONE + '\n', { mode: 0o600 });
    await fs.symlink(elsewhere, tokenFile);
    assert.equal(errorText(await answer()), helperOff);
    assert.equal(connects, counted);
    const everything = answers.join('\n');
    for (const secret of [KEY, TOKEN_ONE, TOKEN_TWO, 'Bearer ']) assert.equal(everything.includes(secret), false);
  } finally {
    await close();
    await bridge.close();
    if (server?.listening) await stopBroker(server);
    await fs.rm(directory, { recursive: true, force: true });
  }
});
test('the bridge entry stops only for a bad plan or identity, with one fixed line', async () => {
  const entry = fileURLToPath(new URL('../isolation/agent-bridge.mjs', import.meta.url));
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'enote-bridge-entry-'));
  const wrongAccount = path.join(directory, 'fake-plan.json');
  try {
    await fs.writeFile(wrongAccount, JSON.stringify(plan));
    await Promise.all([[], [path.join(directory, 'fake-missing-plan.json')], [wrongAccount]].map(args =>
      assert.rejects(promisify(execFile)(process.execPath, [entry, ...args], { env: { PATH: '/usr/bin:/bin' } }), error => {
        assert.equal(error.code, 1);
        assert.equal(error.stdout, '');
        assert.equal(error.stderr, 'Isolated agent bridge did not start. Check the plan path and that this is the planned standard agent account.\n');
        return true;
      })));
  } finally { await fs.rm(directory, { recursive: true, force: true }); }
});
test('the bridge entry still runs through a linked folder, instead of exiting with nothing', async () => {
  const entry = fileURLToPath(new URL('../isolation/agent-bridge.mjs', import.meta.url));
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'enote-bridge-link-'));
  try {
    const linked = path.join(directory, 'linked');
    await fs.symlink(path.dirname(entry), linked);
    await assert.rejects(promisify(execFile)(process.execPath, [path.join(linked, path.basename(entry))], { env: { PATH: '/usr/bin:/bin' } }), error => {
      assert.equal(error.code, 1);
      assert.match(error.stderr, /^Isolated agent bridge did not start\./);
      return true;
    });
  } finally { await fs.rm(directory, { recursive: true, force: true }); }
});

test('version-1 plans are refused with a request for a new reviewed plan, never migrated', () => {
  const { grantExpiresAt: _omitted, ...v1 } = { ...plan, version: 1 };
  assert.throws(() => validatePlan(v1), /version-2/);
  assert.throws(() => validatePlan({ ...plan, version: 1 }), /version-2/);
});
test('grants must be valid canonical UTC instants in the future and at most one hour away', () => {
  const at = ms => ({ ...plan, grantExpiresAt: new Date(ms).toISOString() });
  assert.equal(assertGrantActive(at(T0 + 1000), T0), T0 + 1000);
  assert.equal(assertGrantActive(at(T0 + MAX_GRANT_MS), T0), T0 + MAX_GRANT_MS);
  assert.equal(assertGrantActive({ ...plan, grantExpiresAt: '2026-10-26T12:30:00Z' }, T0),
    Date.parse('2026-10-26T12:30:00Z'));
  for (const bad of [
    { ...plan, grantExpiresAt: null }, at(T0), at(T0 - 1), at(T0 + MAX_GRANT_MS + 1),
    { ...plan, grantExpiresAt: '2026-02-30T00:00:00Z' }, { ...plan, grantExpiresAt: '2026-10-26 12:30:00Z' },
  ]) assert.throws(() => assertGrantActive(bad, T0), /grant/);
  assert.throws(() => assertGrantActive(at(T0 + 1000), NaN));
});
test('an expired grant stays denied after a simulated restart; restarting never adds time', () => {
  const expired = { ...plan, grantExpiresAt: new Date(T0 + 1000).toISOString() };
  assert.throws(() => assertGrantActive(expired, T0 + 1000));
  // A fresh process re-reads the same reviewed plan: still expired.
  assert.throws(() => assertGrantActive(structuredClone(expired), T0 + 5000));
});
test('the admission guard ends at the absolute deadline or after one monotonic hour, whichever is first', () => {
  let wall = T0; let mono = 0;
  const active = grantGuard(T0 + 10_000, { now: () => wall, mono: () => mono });
  assert.equal(active(), true);
  wall = T0 + 10_000; assert.equal(active(), false);
  wall = T0 - 86_400_000; mono = MAX_GRANT_MS; // clock moved backwards a day; an hour passed
  assert.equal(active(), false);
  const fresh = grantGuard(T0 + 10_000, { now: () => T0 - 5_000_000, mono: () => 0 });
  assert.equal(fresh(), true); // a backwards clock cannot extend beyond the monotonic hour
});
test('an expired grant reaches neither validation counters nor the provider', async () => {
  let providerCalls = 0;
  const generate = createGenerator({ apiKey: 'FAKE_OPENAI_AGENT_KEY_FOR_TESTS_ONLY', transport: async () => {
    providerCalls++; return { status: 200, json: { model: POLICY.model, status: 'completed',
      output: [{ type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'Fiction.' }] }] } };
  } });
  let active = true;
  const guarded = guardGenerate(generate, () => active);
  assert.deepEqual(await guarded({ text: 'Fiction.' }), { text: 'Fiction.' });
  active = false;
  await assert.rejects(guarded({ text: 'Fiction.' }), error => error.code === 'grant_expired');
  await assert.rejects(guarded({ text: 'Fiction.', model: 'other' }), error => error.code === 'grant_expired');
  assert.equal(providerCalls, 1);
});

test('a batch ACL listing refuses any allow entry and is not fooled by a file name', () => {
  validateAclBatchListing('-rw-r--r-- 1 o s 0 Jan 1 /fake/a\ndrwxr-xr-x 1 o s 0 Jan 1 /fake/b\n');
  validateAclBatchListing('drwx------+ 1 o s 0 Jan 1 /fake/a\n 0: group:everyone deny delete\n');
  validateAclBatchListing('-rw-r--r-- 1 o s 0 Jan 1 /fake/allow-list/allow.js\n');
  assert.throws(() => validateAclBatchListing('-rw-r--r-- 1 o s 0 Jan 1 /fake/a\n-rw-r--r--+ 1 o s 0 Jan 1 /fake/b\n 0: user:enote-agent allow read\n'), /allow/);
  // A name holding a newline and an indented "entry" can only cause a refusal.
  assert.throws(() => validateAclBatchListing('-rw-r--r-- 1 o s 0 Jan 1 /fake/a\n 0: user:x allow read\n'), /allow/);
});

const fakeCodeTree = async (count) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'enote-code-'));
  await fs.mkdir(path.join(root, 'package', 'nested'), { recursive: true });
  for (let i = 0; i < count; i += 1) await fs.writeFile(path.join(root, 'package', 'nested', `fake-${i}.js`), '// fake\n');
  await fs.writeFile(path.join(root, 'top.js'), '// fake\n');
  return root;
};

test('the code check lists every entry once, in bounded batches, and skips only npm shims', async () => {
  const root = await fakeCodeTree(300);
  try {
    await fs.mkdir(path.join(root, 'node_modules', '.bin'), { recursive: true });
    await fs.symlink('/fake/shim', path.join(root, 'node_modules', '.bin', 'shim'));
    const batches = [];
    await verifyCodeTree(root, ownerUid, { ancestorsChecked: true, aclBatch: async files => { batches.push(files); } });
    const seen = batches.flat();
    // 300 files, top.js, package, nested and node_modules.
    assert.equal(seen.length, 304);
    assert.equal(new Set(seen).size, 304);
    assert.ok(!seen.some(file => file.includes('.bin')));
    assert.ok(batches.every(files => files.length > 0 && files.length <= 128));
    assert.ok(batches.length <= 3, `${batches.length} processes for 304 entries`);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});

test('the code check refuses an unprotected entry, or a failed ACL listing', async () => {
  const noAcl = { ancestorsChecked: true, aclBatch: async () => {} };
  for (const spoil of [
    root => fs.chmod(path.join(root, 'top.js'), 0o666),
    root => fs.symlink('/fake/elsewhere', path.join(root, 'package', 'link')),
  ]) {
    const root = await fakeCodeTree(1);
    try {
      await spoil(root);
      await assert.rejects(verifyCodeTree(root, ownerUid, noAcl), /not protected/);
    } finally { await fs.rm(root, { recursive: true, force: true }); }
  }
  const root = await fakeCodeTree(1);
  try {
    await assert.rejects(verifyCodeTree(root, ownerUid + 1, noAcl), /not protected/);
    await assert.rejects(verifyCodeTree(root, ownerUid, { ancestorsChecked: true, aclBatch: async () => { throw new Error('listing failed'); } }), /listing failed/);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});

test('the code check finds a real ACL grant anywhere in a batch', { skip: process.platform !== 'darwin' }, async t => {
  const root = await fakeCodeTree(200);
  try {
    await fs.writeFile(path.join(root, 'package', 'allow-list.js'), '// a name is not a grant\n');
    await verifyCodeTree(root, ownerUid, { ancestorsChecked: true });
    try {
      await promisify(execFile)('/bin/chmod', ['+a', 'user:nobody allow read', path.join(root, 'package', 'nested', 'fake-150.js')]);
    } catch { t.skip('this disk has no ACLs'); return; }
    await assert.rejects(verifyCodeTree(root, ownerUid, { ancestorsChecked: true }), /allow/);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});

test('the code check of a large real tree stays fast', { skip: process.platform !== 'darwin' }, async () => {
  const root = await fakeCodeTree(1000);
  try {
    const started = performance.now();
    await verifyCodeTree(root, ownerUid, { ancestorsChecked: true });
    // The helper's readiness check gets 10 seconds for this and more; one process per entry took
    // about 4 s for 1,000 entries on the Mac where this was found.
    const elapsed = performance.now() - started;
    assert.ok(elapsed < 3000, `took ${Math.round(elapsed)} ms`);
  } finally { await fs.rm(root, { recursive: true, force: true }); }
});
