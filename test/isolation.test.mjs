import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { validatePlan, requireRole, parseConfirmedCredential, readBounded,
  validateAclListing, assertProtectedPath, assertStandardIdentity } from '../isolation/common.mjs';
import { prepareOwnerFixture, verifyOwnerFixture, probeOwnerFixture } from '../isolation/fixtures.mjs';
import { bridgeGenerator } from '../isolation/agent-bridge.mjs';
const ownerUid = process.getuid();
const plan = {
  version: 1, ownerUid, agentUid: ownerUid + 1, agentShortName: 'enote-agent',
  credentialExpirySeconds: 3600, projectHardSpendLimitCents: 100,
  projectHardSpendCurrency: 'USD', projectHardSpendInterval: 'month',
  brokerHost: '127.0.0.1', brokerPort: 8789, providerModel: 'gpt-6-luna',
  providerScope: 'api.responses.write', bearerTokenFile: '/Users/Shared/FAKE-TOKEN-DIRECTORY/token',
};
test('isolation plan rejects missing identity, looser trial controls and unreviewed destinations', () => {
  validatePlan(plan);
  for (const changed of [
    { ownerUid: null }, { agentUid: ownerUid }, { agentUid: 0 }, { bearerTokenFile: null },
    { bearerTokenFile: '/Users/Shared/../unreviewed' }, { credentialExpirySeconds: 7200 },
    { projectHardSpendLimitCents: 200 }, { brokerHost: '0.0.0.0' }, { brokerPort: 443 },
    { providerModel: 'other' }, { providerScope: 'api.all.read' }, { unexpected: true },
  ]) assert.throws(() => validatePlan({ ...plan, ...changed }));
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
