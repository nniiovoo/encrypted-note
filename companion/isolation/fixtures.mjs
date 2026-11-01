import fs from 'node:fs/promises';
import { constants } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';
import { loadPlan, requireRole, assertProtectedPath } from './common.mjs';
const LABEL = 'FAKE-ENOTE-ISOLATION-ONLY';
const CONTENTS = Object.freeze({
  'owner-only-fake.txt': LABEL + '-OWNER\n',
  'broker-fake.mjs': '// ' + LABEL + '-BROKER\n',
  'policy-fake.json': JSON.stringify({ label: LABEL, fake: true }) + '\n',
  'dependency-fake.mjs': '// ' + LABEL + '-DEPENDENCY\n',
});
export function validateFixture(manifest) {
  if (manifest?.version !== 1 || manifest.label !== LABEL ||
      !Number.isInteger(manifest.ownerUid) || !Number.isInteger(manifest.agentUid) ||
      manifest.ownerUid === manifest.agentUid || !path.isAbsolute(manifest.root ?? '') ||
      path.resolve(manifest.root) !== manifest.root ||
      !/^enote-isolation-[A-Za-z0-9]+$/.test(path.basename(manifest.root))) {
    throw new Error('Invalid fictional fixture manifest.');
  }
  return manifest;
}
export async function prepareOwnerFixture(plan, parent, { protect = assertProtectedPath } = {}) {
  requireRole(plan, 'owner');
  await protect(parent, plan.ownerUid);
  const root = await fs.mkdtemp(path.join(parent, 'enote-isolation-'));
  await fs.chmod(root, 0o700); // Only this newly created directory.
  for (const [name, value] of Object.entries(CONTENTS)) {
    await fs.writeFile(path.join(root, name), value, { mode: 0o600, flag: 'wx' });
  }
  const manifest = { version: 1, label: LABEL, ownerUid: plan.ownerUid, agentUid: plan.agentUid, root };
  await fs.writeFile(path.join(root, 'owner-record.json'), JSON.stringify(manifest, null, 2) + '\n',
    { mode: 0o600, flag: 'wx' });
  return manifest;
}
export async function verifyOwnerFixture(manifest) {
  validateFixture(manifest);
  if (process.getuid() !== manifest.ownerUid) throw new Error('Owner verification requires the owner UID.');
  const directory = await fs.lstat(manifest.root);
  if (!directory.isDirectory() || directory.isSymbolicLink() || directory.uid !== manifest.ownerUid ||
      (directory.mode & 0o777) !== 0o700) throw new Error('Fictional fixture protection changed.');
  for (const [name, expected] of Object.entries(CONTENTS)) {
    const handle = await fs.open(path.join(manifest.root, name), constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
      const info = await handle.stat();
      if (!info.isFile() || info.uid !== manifest.ownerUid || info.nlink !== 1 ||
          (info.mode & 0o777) !== 0o600 || info.size !== Buffer.byteLength(expected)) {
        throw new Error('Fictional owner fixture changed.');
      }
      const actual = await handle.readFile();
      const equal = createHash('sha256').update(actual).digest('hex') ===
        createHash('sha256').update(expected).digest('hex');
      actual.fill(0);
      if (!equal) throw new Error('Fictional owner fixture changed.');
    } finally { await handle.close(); }
  }
  return { ownerFixturesUnchanged: true };
}
export async function probeOwnerFixture(manifest, io = fs) {
  validateFixture(manifest);
  const denied = async operation => {
    try { await operation(); }
    catch (error) { if (['EACCES', 'EPERM'].includes(error.code)) return; throw new Error('Fixture denial was not established.'); }
    throw new Error('Agent accessed a protected fictional fixture; stop before credentials.');
  };
  const result = {};
  await denied(async () => { const data = await io.readFile(path.join(manifest.root, 'owner-only-fake.txt')); data.fill(0); });
  result.ownerReadDenied = true;
  await denied(() => io.readdir(manifest.root)); result.ownerListingDenied = true;
  for (const name of Object.keys(CONTENTS)) {
    await denied(async () => {
      const handle = await io.open(path.join(manifest.root, name), 'a');
      try { await handle.write('FAKE-AGENT-WRITE-ATTEMPT\n'); } finally { await handle.close(); }
    });
  }
  result.ownerAndCodeWriteDenied = true;
  await denied(() => io.writeFile(path.join(manifest.root, 'new-fake-child.txt'), LABEL, { flag: 'wx', mode: 0o600 }));
  result.ownerCreateDenied = true;
  await denied(() => io.rename(path.join(manifest.root, 'owner-only-fake.txt'),
    path.join(manifest.root, 'owner-renamed-fake.txt'))); result.ownerRenameDenied = true;
  await denied(() => io.unlink(path.join(manifest.root, 'owner-only-fake.txt'))); result.ownerUnlinkDenied = true;
  return result;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.argv[2] === 'prepare' && process.argv.length === 5) {
      const plan = await loadPlan(process.argv[3]);
      const manifest = await prepareOwnerFixture(plan, process.argv[4]);
      // Nonsecret paths/UIDs, for the owner's private operational record only.
      process.stdout.write(JSON.stringify(manifest) + '\n');
    } else if (process.argv[2] === 'verify-owner' && process.argv.length === 4) {
      const manifest = JSON.parse(await fs.readFile(process.argv[3], 'utf8'));
      process.stdout.write(JSON.stringify(await verifyOwnerFixture(manifest)) + '\n');
    } else throw new Error('Unknown operation.');
  } catch {
    process.stderr.write('Fictional fixture operation stopped; preserve the private record and review the boundary.\n');
    process.exitCode = 1;
  }
}
