import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { once } from 'node:events';
import { launchOwnerBroker } from '../isolation/owner-launcher.mjs';
import { stopHttpServer } from '../src/http.mjs';
import { writeAgentToken, removeAgentToken } from '../isolation/token-handoff.mjs';

const KEY = 'FAKE_OPENAI_AGENT_KEY_FOR_TESTS_ONLY';
const OLD_TOKEN = 'FAKE_PREVIOUS_LAUNCH_TOKEN_FOR_TESTS_ONLY_0123456789';
const uid = process.getuid();
async function fixture() {
  const root = await fs.mkdtemp(path.join(await fs.realpath(os.tmpdir()), 'enote-companion-owner-'));
  const handoff = path.join(root, 'handoff');
  await fs.mkdir(handoff, { mode: 0o700 });
  const credentialFile = path.join(root, 'credential.env');
  const tokenFile = path.join(handoff, 'agent-token');
  await fs.writeFile(credentialFile, 'OPENAI_AGENT_API_KEY=' + KEY + '\n', { mode: 0o600 });
  await fs.writeFile(tokenFile, OLD_TOKEN + '\n', { mode: 0o600 });
  const plan = {
    version: 2, ownerUid: uid, agentUid: uid + 1, agentShortName: 'enote-agent',
    credentialExpirySeconds: 3600, projectHardSpendLimitCents: 100, projectHardSpendCurrency: 'USD',
    projectHardSpendInterval: 'month', brokerHost: '127.0.0.1', brokerPort: 8789,
    providerModel: 'gpt-6-luna', providerScope: 'api.responses.write',
    bearerTokenFile: tokenFile, grantExpiresAt: new Date(Date.now() + 60_000).toISOString(),
  };
  const output = [];
  const io = { stat: fs.lstat, aclCheck: async () => {}, addReadAcl: async () => {}, verifyCode: async () => {} };
  return { root, handoff, plan, tokenFile, output, io,
    options: { credentialFile, tokenFile, output: { write: text => output.push(text) }, io, port: 0,
      transport: async () => { throw new Error('FAKE_UNEXPECTED_PROVIDER_CALL'); } },
    cleanup: () => fs.rm(root, { recursive: true, force: true }) };
}
async function close(server) {
  if (!server?.listening) return;
  const closed = once(server, 'close');
  stopHttpServer(server);
  await closed;
}
async function waitForRemoval(file) {
  for (let i = 0; i < 500; i++) {
    try { await fs.lstat(file); }
    catch (error) { if (error.code === 'ENOENT') return; throw error; }
    await new Promise(resolve => setTimeout(resolve, 10));
  }
  assert.fail('Launch authority was not removed after the listener closed.');
}
async function availablePort() {
  const reservation = net.createServer();
  await new Promise(resolve => reservation.listen(0, '127.0.0.1', resolve));
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  return port;
}
async function authStatus(server, token) {
  return (await fetch('http://127.0.0.1:' + server.address().port + '/mcp', {
    method: 'POST', headers: { authorization: 'Bearer ' + token, 'content-type': 'application/json',
      accept: 'application/json, text/event-stream' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/list', params: {} }),
  })).status;
}

test('standalone restart rejects prior launch authority and publishes a fresh token', async () => {
  const fx = await fixture();
  let server;
  const grantExpiresAt = fx.plan.grantExpiresAt;
  try {
    server = await launchOwnerBroker(fx.plan, fx.options);
    const first = (await fs.readFile(fx.tokenFile, 'utf8')).trim();
    await close(server);
    await waitForRemoval(fx.tokenFile);
    server = await launchOwnerBroker(fx.plan, fx.options);
    const second = (await fs.readFile(fx.tokenFile, 'utf8')).trim();
    assert.notEqual(first, second);
    assert.notEqual(first, OLD_TOKEN);
    assert.match(first, /^[A-Za-z0-9_-]{43}$/);
    assert.match(second, /^[A-Za-z0-9_-]{43}$/);
    assert.equal(await authStatus(server, first), 401);
    assert.equal(await authStatus(server, second), 200);
    assert.equal(fx.plan.grantExpiresAt, grantExpiresAt);
    assert.deepEqual(fx.output, Array(2).fill('Owner broker is listening on reviewed loopback port 8789 until its grant ends (at most one hour).\n'));
    for (const marker of [KEY, first, second, 'Bearer ']) assert.equal(fx.output.join('').includes(marker), false);
  } finally { await close(server); await waitForRemoval(fx.tokenFile); await fx.cleanup(); }
});

test('standalone publishes token read access only after binding the real listener', async () => {
  const fx = await fixture();
  const port = await availablePort();
  let server;
  let publicationSawBoundListener = false;
  try {
    fx.io.addReadAcl = async (file, agent) => {
      assert.equal(file, fx.tokenFile);
      assert.equal(agent, 'enote-agent');
      const contender = net.createServer();
      publicationSawBoundListener = await new Promise(resolve => {
        contender.once('error', error => resolve(error.code === 'EADDRINUSE'));
        contender.listen(port, '127.0.0.1', () => contender.close(() => resolve(false)));
      });
    };
    server = await launchOwnerBroker(fx.plan, { ...fx.options, port });
    assert.equal(publicationSawBoundListener, true);
    assert.equal((await fs.stat(fx.tokenFile)).mode & 0o777, 0o600);
  } finally { await close(server); await waitForRemoval(fx.tokenFile); await fx.cleanup(); }
});

test('a failed standalone bind preserves an existing launch token without publishing authority', async () => {
  const fx = await fixture();
  const incumbent = await launchOwnerBroker(fx.plan, fx.options);
  const currentToken = await fs.readFile(fx.tokenFile, 'utf8');
  let publications = 0;
  fx.io.addReadAcl = async () => { publications++; };
  try {
    await assert.rejects(launchOwnerBroker(fx.plan, { ...fx.options, port: incumbent.address().port }), { code: 'EADDRINUSE' });
    assert.equal(publications, 0);
    assert.equal(await fs.readFile(fx.tokenFile, 'utf8'), currentToken);
    assert.equal(await authStatus(incumbent, currentToken.trim()), 200);
    assert.equal(fx.output.length, 1);
  } finally { await close(incumbent); await waitForRemoval(fx.tokenFile); await fx.cleanup(); }
});

test('a failed standalone publication closes its listener and removes its own partial token', async () => {
  const fx = await fixture();
  const port = await availablePort();
  fx.io.addReadAcl = async () => { throw new Error('FAKE_ACL_FAILURE'); };
  try {
    await assert.rejects(launchOwnerBroker(fx.plan, { ...fx.options, port }), /FAKE_ACL_FAILURE/);
    await assert.rejects(fs.lstat(fx.tokenFile), { code: 'ENOENT' });
    await assert.rejects(fetch('http://127.0.0.1:' + port + '/mcp'));
    const probe = net.createServer();
    await new Promise((resolve, reject) => { probe.once('error', reject); probe.listen(port, '127.0.0.1', resolve); });
    await new Promise(resolve => probe.close(resolve));
    assert.deepEqual(fx.output, []);
  } finally { await fx.cleanup(); }
});

test('standalone creates first-launch authority and stops at the original reviewed grant deadline', { timeout: 10_000 }, async () => {
  const fx = await fixture();
  await fs.unlink(fx.tokenFile);
  const deadline = Date.now() + 2000;
  fx.plan.grantExpiresAt = new Date(deadline).toISOString();
  let server;
  try {
    server = await launchOwnerBroker(fx.plan, fx.options);
    assert.equal(server.listening, true);
    assert.equal(await authStatus(server, (await fs.readFile(fx.tokenFile, 'utf8')).trim()), 200);
    await once(server, 'close');
    await waitForRemoval(fx.tokenFile);
    assert.ok(Date.now() >= deadline);
    assert.equal(fx.plan.grantExpiresAt, new Date(deadline).toISOString());
  } finally { await close(server); await fx.cleanup(); }
});

test('token cleanup accepts only its inode/dev lease and cannot delete a successor publication', async () => {
  const fx = await fixture();
  let first; let second;
  try {
    first = await writeAgentToken(fx.plan, OLD_TOKEN, fx.io);
    const info = await fs.lstat(fx.tokenFile);
    assert.equal(first.dev, info.dev);
    assert.equal(first.ino, info.ino);
    await removeAgentToken(fx.tokenFile);
    await removeAgentToken({ ...first });
    assert.equal(await fs.readFile(fx.tokenFile, 'utf8'), OLD_TOKEN + '\n');
    const successorToken = 'FAKE_SUCCESSOR_TOKEN_FOR_TESTS_ONLY_0123456789';
    second = await writeAgentToken(fx.plan, successorToken, fx.io);
    assert.notEqual(second.ino, first.ino);
    await removeAgentToken(first);
    await removeAgentToken(first);
    assert.equal(await fs.readFile(fx.tokenFile, 'utf8'), successorToken + '\n');
    await removeAgentToken(second);
    await assert.rejects(fs.lstat(fx.tokenFile), { code: 'ENOENT' });
  } finally { await removeAgentToken(first); await removeAgentToken(second); await fx.cleanup(); }
});

test('standalone close reserves the listener until checked token cleanup completes', { timeout: 10_000 }, async () => {
  const fx = await fixture();
  let armed = false;
  let announcePaused; let resume;
  const paused = new Promise(resolve => { announcePaused = resolve; });
  const released = new Promise(resolve => { resume = resolve; });
  fx.io.stat = async file => {
    const info = await fs.lstat(file);
    if (armed && file === fx.tokenFile) {
      armed = false;
      announcePaused();
      await released;
    }
    return info;
  };
  let server; let attempted; let successor; let closed;
  try {
    server = await launchOwnerBroker(fx.plan, fx.options);
    const port = server.address().port;
    armed = true;
    closed = once(server, 'close');
    assert.equal(server.close(), server);
    await paused;
    let bindError;
    try { attempted = await launchOwnerBroker(fx.plan, { ...fx.options, io: { ...fx.io, stat: fs.lstat }, port }); }
    catch (error) { bindError = error; }
    assert.equal(bindError?.code, 'EADDRINUSE');
    assert.equal(server.listening, true);
    resume();
    await closed;
    successor = await launchOwnerBroker(fx.plan, { ...fx.options, port });
    const token = (await fs.readFile(fx.tokenFile, 'utf8')).trim();
    assert.match(token, /^[A-Za-z0-9_-]{43}$/);
    assert.equal(await authStatus(successor, token), 200);
  } finally {
    resume();
    await closed;
    await close(attempted);
    await close(successor);
    await close(server);
    await fx.cleanup();
  }
});

test('standalone post-publication expiry reserves the listener through token cleanup', { timeout: 10_000 }, async () => {
  const fx = await fixture();
  const port = await availablePort();
  let wall = Date.now();
  const deadline = wall + 60_000;
  fx.plan.grantExpiresAt = new Date(deadline).toISOString();
  let armed = false;
  let announcePaused; let resume;
  const paused = new Promise(resolve => { announcePaused = resolve; });
  const released = new Promise(resolve => { resume = resolve; });
  fx.io.aclCheck = async file => {
    if (file === fx.tokenFile) {
      wall = deadline;
      armed = true;
    }
  };
  fx.io.stat = async file => {
    const info = await fs.lstat(file);
    if (armed && file === fx.tokenFile) {
      armed = false; announcePaused(); await released;
    }
    return info;
  };
  const failedStartup = launchOwnerBroker(fx.plan, { ...fx.options, port, now: () => wall }).then(() => undefined, error => error);
  let attempted; let successor;
  const freshIo = { ...fx.io, stat: fs.lstat, aclCheck: async () => {} };
  try {
    await paused;
    const successorPlan = { ...fx.plan, grantExpiresAt: new Date(Date.now() + 60_000).toISOString() };
    let bindError;
    try { attempted = await launchOwnerBroker(successorPlan, { ...fx.options, io: freshIo, port }); }
    catch (error) { bindError = error; }
    assert.equal(bindError?.code, 'EADDRINUSE');
    resume();
    assert.match((await failedStartup)?.message, /approved agent grant/);
    await assert.rejects(fs.lstat(fx.tokenFile), { code: 'ENOENT' });
    successor = await launchOwnerBroker(successorPlan, { ...fx.options, io: freshIo, port });
    const token = (await fs.readFile(fx.tokenFile, 'utf8')).trim();
    assert.equal(await authStatus(successor, token), 200);
  } finally {
    resume(); await failedStartup;
    await close(attempted); await close(successor); await fx.cleanup();
  }
});

for (const failure of [false, true]) {
  test('standalone publication validation gates provider admissions' + (failure ? ' on failure' : ' before ready'), { timeout: 10_000 }, async () => {
    const fx = await fixture();
    const port = await availablePort();
    let checking; let finishCheck; let cleaning; let finishCleanup;
    const validation = new Promise(resolve => { checking = resolve; });
    const checked = new Promise(resolve => { finishCheck = resolve; });
    const cleanup = new Promise(resolve => { cleaning = resolve; });
    const cleaned = new Promise(resolve => { finishCleanup = resolve; });
    let armCleanup = false; let calls = 0;
    fx.io.aclCheck = async file => {
      if (file === fx.tokenFile) {
        checking(); await checked;
        if (failure) { armCleanup = true; throw new Error('FAKE_FINAL_ACL_FAILURE'); }
      }
    };
    fx.io.stat = async file => {
      const info = await fs.lstat(file);
      if (armCleanup && file === fx.tokenFile) { armCleanup = false; cleaning(); await cleaned; }
      return info;
    };
    const startup = launchOwnerBroker(fx.plan, { ...fx.options, port,
      transport: async () => { calls++; throw new Error('FAKE_UNEXPECTED_PROVIDER_CALL'); },
    }).then(server => ({ server }), error => ({ error }));
    let server; let attempted; let successor;
    try {
      await validation;
      const token = (await fs.readFile(fx.tokenFile, 'utf8')).trim();
      const url = 'http://127.0.0.1:' + port + '/mcp';
      const response = await fetch(url, { method: 'POST', headers: { authorization: 'Bearer ' + token,
        'content-type': 'application/json', accept: 'application/json, text/event-stream' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call',
          params: { name: 'generate_text', arguments: { text: 'Fiction.' } } }) });
      assert.equal((await response.json()).result.isError, true);
      assert.equal(calls, 0);
      assert.equal(fx.output.length, 0);
      finishCheck();
      if (failure) {
        await cleanup;
        assert.equal((await fetch(url, { method: 'POST' })).status, 503);
        let bindError;
        const freshIo = { ...fx.io, stat: fs.lstat, aclCheck: async () => {} };
        try { attempted = await launchOwnerBroker(fx.plan, { ...fx.options, io: freshIo, port }); }
        catch (error) { bindError = error; }
        assert.equal(bindError?.code, 'EADDRINUSE');
        finishCleanup();
        assert.match((await startup).error?.message, /FAKE_FINAL_ACL_FAILURE/);
        successor = await launchOwnerBroker(fx.plan, { ...fx.options, io: freshIo, port });
        assert.equal(await authStatus(successor, (await fs.readFile(fx.tokenFile, 'utf8')).trim()), 200);
      } else {
        server = (await startup).server;
        assert.equal(fx.output.length, 1);
        assert.equal(await authStatus(server, token), 200);
      }
    } finally {
      finishCheck(); finishCleanup();
      server ??= (await startup).server;
      await close(server); await close(attempted); await close(successor); await fx.cleanup();
    }
  });
}
