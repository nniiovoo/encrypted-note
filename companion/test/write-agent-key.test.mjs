import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { checkAgentKeyShape, writeAgentKey, readInput, KEY_REFUSED, ADMIN_KEY_REFUSED, FILE_REFUSED, ACL_APPEARED, USAGE }
  from '../isolation/write-agent-key.mjs';
import { readConfirmedCredential } from '../isolation/common.mjs';

const SCRIPT = fileURLToPath(new URL('../isolation/write-agent-key.mjs', import.meta.url));
const FAKE_KEYS = [
  'sk-FAKE_AGENT_KEY_FOR_TESTS_ONLY_0123456789abcdef',
  'sk-proj-FAKE_PROJECT_KEY_FOR_TESTS_ONLY_' + 'x'.repeat(120),
  'sk-svcacct-FAKE_SERVICE_ACCOUNT_KEY_FOR_TESTS_ONLY_0123456789',
];
// Values a Vault might leave on the clipboard, all fictional.
const NOT_KEYS = [
  '', 'FAKE_KEY_FOR_SETUP_ONLY_0123456789', 'sk-FAKE_TOO_SHORT',
  'fake fake fake fake fake fake fake fake fake fake fake fake',
  'sk-FAKE_AGENT_KEY_FOR_TESTS_ONLY_0123456789abcdef\n',
  ' sk-FAKE_AGENT_KEY_FOR_TESTS_ONLY_0123456789abcdef',
  'sk-FAKE_AGENT_KEY_FOR_TESTS ONLY_0123456789abcdef',
  'sk-FAKE_AGENT_KEY_FOR_TESTS_ONLY\r\n0123456789abcdef',
  'sk-FAKE_AGENT_KEY_FOR_TESTS_ONLY_0123456789abcdef"; echo FAKE',
  'sk-' + 'F'.repeat(510),
  '0x' + 'fa4e'.repeat(16),
  'OPENAI_AGENT_API_KEY=sk-FAKE_AGENT_KEY_FOR_TESTS_ONLY_0123456789abcdef',
  // Other providers' keys that also start with sk-.
  'sk-ant-api03-FAKE_ANTHROPIC_KEY_FOR_TESTS_ONLY_' + 'x'.repeat(60),
  'sk-or-v1-FAKE_OPENROUTER_KEY_FOR_TESTS_ONLY_' + 'f'.repeat(40),
];

async function scratch(t) {
  // Real path: /var and /tmp are symlinks, and protected-path checks refuse symlinked ancestors.
  const directory = await fs.mkdtemp(path.join(await fs.realpath(os.tmpdir()), 'enote-agent-key-'));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  return directory;
}

test('accepts only values shaped like an OpenAI API key, and never an Admin key', () => {
  for (const key of FAKE_KEYS) assert.equal(checkAgentKeyShape(key), null, key.slice(0, 12));
  for (const value of NOT_KEYS) assert.equal(checkAgentKeyShape(value), KEY_REFUSED, JSON.stringify(value.slice(0, 12)));
  assert.equal(checkAgentKeyShape('sk-admin-FAKE_ADMIN_KEY_FOR_TESTS_ONLY_0123456789'), ADMIN_KEY_REFUSED);
  for (const text of [KEY_REFUSED, ADMIN_KEY_REFUSED, FILE_REFUSED, ACL_APPEARED, USAGE]) assert.doesNotMatch(text, /FAKE/);
});

test('writes an owner-only file that Start accepts, and asks Time Machine to skip it', async t => {
  const directory = await scratch(t);
  const file = path.join(directory, 'agent-key.env');
  const excluded = [];
  for (const key of FAKE_KEYS) {
    assert.deepEqual(await writeAgentKey(file, Buffer.from(key), { exclude: async target => { excluded.push(target); return true; } }),
      { excluded: true });
    assert.equal((await fs.stat(file)).mode & 0o777, 0o600);
    assert.equal(await readConfirmedCredential(file, process.getuid()), key);
  }
  assert.deepEqual(excluded, FAKE_KEYS.map(() => file));
  // A failed exclusion still leaves the written file, and the owner is told.
  assert.deepEqual(await writeAgentKey(file, Buffer.from(FAKE_KEYS[0]), { exclude: async () => false }), { excluded: false });
  // An existing file with looser permissions is replaced by a fresh owner-only file.
  await fs.chmod(file, 0o644);
  await writeAgentKey(file, Buffer.from(FAKE_KEYS[1]), { exclude: async () => true });
  assert.equal((await fs.stat(file)).mode & 0o777, 0o600);
});

test('replacing a readable file never gives its existing readers the new key', async t => {
  const directory = await scratch(t);
  const file = path.join(directory, 'agent-key.env');
  const earlier = 'FAKE_EARLIER_CONTENT\n';
  await fs.writeFile(file, earlier, { mode: 0o644 });
  const reader = await fs.open(file, 'r');
  t.after(() => reader.close());
  const before = await reader.stat();
  await writeAgentKey(file, Buffer.from(FAKE_KEYS[0]), { exclude: async () => true });
  assert.equal(await reader.readFile('utf8'), earlier);
  assert.notEqual((await fs.stat(file)).ino, before.ino);
  assert.equal(await readConfirmedCredential(file, process.getuid()), FAKE_KEYS[0]);
  assert.deepEqual(await fs.readdir(directory), ['agent-key.env']);
});

test('refuses a value or a file before writing anything', async t => {
  const directory = await scratch(t);
  const file = path.join(directory, 'agent-key.env');
  const exclude = async () => { throw new Error('not reached'); };
  for (const value of [...NOT_KEYS, 'sk-admin-FAKE_ADMIN_KEY_FOR_TESTS_ONLY_0123456789']) {
    await assert.rejects(writeAgentKey(file, Buffer.from(value), { exclude }),
      error => [KEY_REFUSED, ADMIN_KEY_REFUSED].includes(error.message));
  }
  await assert.rejects(writeAgentKey(file, Buffer.from('sk-' + 'F'.repeat(5000)), { exclude }), { message: KEY_REFUSED });
  await assert.rejects(writeAgentKey(file, Buffer.from([0x73, 0x6b, 0x2d, 0xff, ...Buffer.from('F'.repeat(40))]), { exclude }),
    { message: KEY_REFUSED });
  await assert.rejects(fs.stat(file), { code: 'ENOENT' });
  const target = path.join(directory, 'elsewhere.env');
  await fs.writeFile(target, 'FAKE_UNRELATED_CONTENT\n');
  await fs.symlink(target, file);
  await assert.rejects(writeAgentKey(file, Buffer.from(FAKE_KEYS[0]), { exclude }), { message: FILE_REFUSED });
  assert.equal(await fs.readFile(target, 'utf8'), 'FAKE_UNRELATED_CONTENT\n');
  await fs.unlink(file);
  await fs.link(target, file);
  await assert.rejects(writeAgentKey(file, Buffer.from(FAKE_KEYS[0]), { exclude }), { message: FILE_REFUSED });
  assert.equal(await fs.readFile(target, 'utf8'), 'FAKE_UNRELATED_CONTENT\n');
  await assert.rejects(writeAgentKey(directory, Buffer.from(FAKE_KEYS[0]), { exclude }), { message: FILE_REFUSED });
});

test('refuses a file an ACL opens to another account, set on it or inherited from its folder', async t => {
  const directory = await scratch(t);
  const file = path.join(directory, 'agent-key.env');
  await fs.writeFile(file, 'FAKE_EARLIER_CONTENT\n', { mode: 0o600 });
  if (spawnSync('/bin/chmod', ['+a', 'user:nobody allow read', file]).status !== 0) {
    t.skip('chmod +a is not available here');
    return;
  }
  const exclude = async () => { throw new Error('not reached'); };
  await assert.rejects(writeAgentKey(file, Buffer.from(FAKE_KEYS[0]), { exclude }), { message: FILE_REFUSED });
  assert.equal(await fs.readFile(file, 'utf8'), 'FAKE_EARLIER_CONTENT\n');
  const folder = path.join(directory, 'inheriting');
  await fs.mkdir(folder, { mode: 0o700 });
  assert.equal(spawnSync('/bin/chmod', ['+a', 'user:nobody allow read,file_inherit', folder]).status, 0);
  const fresh = path.join(folder, 'agent-key.env');
  await assert.rejects(writeAgentKey(fresh, Buffer.from(FAKE_KEYS[0]), { exclude }), { message: FILE_REFUSED });
  await assert.rejects(fs.stat(fresh), { code: 'ENOENT' });
  assert.deepEqual(await fs.readdir(folder), []);
});

test('an ACL found after writing removes the temporary key and keeps the earlier file', async t => {
  const directory = await scratch(t);
  const file = path.join(directory, 'agent-key.env');
  await fs.writeFile(file, 'FAKE_EARLIER_CONTENT\n', { mode: 0o600 });
  let checks = 0;
  const aclCheck = async checked => {
    if (checked !== file && ++checks === 2) throw new Error('FAKE ACL allow entry');
  };
  await assert.rejects(writeAgentKey(file, Buffer.from(FAKE_KEYS[0]), { exclude: async () => true, aclCheck }),
    { message: ACL_APPEARED });
  assert.equal(checks, 2);
  assert.equal(await fs.readFile(file, 'utf8'), 'FAKE_EARLIER_CONTENT\n');
  assert.deepEqual(await fs.readdir(directory), ['agent-key.env']);
});

test('a destination changed while preparing the key is never overwritten', async t => {
  const directory = await scratch(t);
  const file = path.join(directory, 'agent-key.env');
  await fs.writeFile(file, 'FAKE_EARLIER_CONTENT\n', { mode: 0o600 });
  let checks = 0;
  const aclCheck = async checked => {
    if (checked !== file && ++checks === 2) {
      await fs.rename(file, path.join(directory, 'earlier.env'));
      await fs.writeFile(file, 'FAKE_REPLACEMENT_CONTENT\n', { mode: 0o600 });
    }
  };
  await assert.rejects(writeAgentKey(file, Buffer.from(FAKE_KEYS[0]), { exclude: async () => true, aclCheck }),
    { message: FILE_REFUSED });
  assert.equal(await fs.readFile(file, 'utf8'), 'FAKE_REPLACEMENT_CONTENT\n');
  assert.equal(await fs.readFile(path.join(directory, 'earlier.env'), 'utf8'), 'FAKE_EARLIER_CONTENT\n');
  assert.deepEqual((await fs.readdir(directory)).sort(), ['agent-key.env', 'earlier.env']);
});

test('unprotected or linked parent folders are refused before a key file is created', async t => {
  const directory = await scratch(t);
  const writable = path.join(directory, 'writable');
  await fs.mkdir(writable);
  await fs.chmod(writable, 0o777);
  const linked = path.join(directory, 'linked');
  await fs.symlink(writable, linked);
  for (const parent of [writable, linked]) {
    await assert.rejects(writeAgentKey(path.join(parent, 'agent-key.env'), Buffer.from(FAKE_KEYS[0]),
      { exclude: async () => true }), { message: FILE_REFUSED });
  }
  assert.deepEqual(await fs.readdir(writable), []);
});

test('input past the bound is refused as too long, even when the first read alone is key-shaped', async t => {
  const directory = await scratch(t);
  const file = path.join(directory, 'agent-key.env');
  const split = await readInput([Buffer.from(FAKE_KEYS[0]), Buffer.from('\nFAKE_OTHER_CLIPBOARD_TEXT ' + 'F'.repeat(8000))]);
  assert.equal(split.length, 4097);
  await assert.rejects(writeAgentKey(file, split, { exclude: async () => true }), { message: KEY_REFUSED });
  await assert.rejects(fs.stat(file), { code: 'ENOENT' });
  // Within the bound, the reads are joined as they came.
  assert.equal((await readInput([Buffer.from(FAKE_KEYS[0].slice(0, 10)), Buffer.from(FAKE_KEYS[0].slice(10))])).toString(),
    FAKE_KEYS[0]);
});

test('the command refuses a non-key from stdin and never prints the value', async t => {
  const directory = await scratch(t);
  const file = path.join(directory, 'agent-key.env');
  const marker = 'FAKE_VAULT_VALUE_ON_THE_CLIPBOARD';
  for (const [args, input, expected] of [
    [[file], marker, KEY_REFUSED],
    [[file], 'fake seed words ' + marker, KEY_REFUSED],
    [[file], 'sk-admin-' + marker + '_0123456789abcdef', ADMIN_KEY_REFUSED],
    [[], FAKE_KEYS[0], USAGE],
    [[file, marker], FAKE_KEYS[0], USAGE],
  ]) {
    const result = spawnSync(process.execPath, [SCRIPT, ...args], { input, encoding: 'utf8', timeout: 10_000 });
    assert.equal(result.status, 1);
    assert.equal(result.stderr, expected + '\n');
    assert.equal(result.stdout, '');
    assert.equal((result.stdout + result.stderr).includes(marker), false);
    assert.equal((result.stdout + result.stderr).includes(FAKE_KEYS[0]), false);
  }
  await assert.rejects(fs.stat(file), { code: 'ENOENT' });
});

test('the command still runs through a linked folder, instead of exiting with nothing', async t => {
  const directory = await scratch(t);
  const linked = path.join(directory, 'linked');
  await fs.symlink(path.dirname(SCRIPT), linked);
  const file = path.join(directory, 'agent-key.env');
  const result = spawnSync(process.execPath, [path.join(linked, path.basename(SCRIPT)), file],
    { input: 'FAKE_NOT_A_KEY', encoding: 'utf8', timeout: 10_000 });
  assert.equal(result.status, 1);
  assert.equal(result.stderr, KEY_REFUSED + '\n');
  await assert.rejects(fs.stat(file), { code: 'ENOENT' });
});
