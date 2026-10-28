import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { validatePlan, requireRole, parseConfirmedCredential, readBounded,
  validateAclListing, assertProtectedPath, assertStandardIdentity, assertGrantActive,
  grantGuard, MAX_GRANT_MS } from '../isolation/common.mjs';
import { createGenerator, guardGenerate, POLICY } from '../src/generate.mjs';
import { prepareOwnerFixture, verifyOwnerFixture, probeOwnerFixture } from '../isolation/fixtures.mjs';
import { bridgeGenerator } from '../isolation/agent-bridge.mjs';
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
