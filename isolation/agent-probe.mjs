import fs from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { loadPlan, assertStandardIdentity } from './common.mjs';
import { validateFixture, probeOwnerFixture } from './fixtures.mjs';

export async function runAgentProbe(plan, manifest, workspace) {
  await assertStandardIdentity(plan);
  validateFixture(manifest);
  if (manifest.agentUid !== plan.agentUid || manifest.ownerUid !== plan.ownerUid) throw new Error('Fixture identity mismatch.');
  const info = await fs.lstat(workspace);
  if (!info.isDirectory() || info.isSymbolicLink() || info.uid !== plan.agentUid ||
      (info.mode & 0o077) !== 0) throw new Error('Use a new private agent workspace.');
  const positive = await fs.mkdtemp(path.join(workspace, 'enote-agent-positive-'));
  const file = path.join(positive, 'agent-only-fake.txt');
  const link = path.join(positive, 'owner-only-fake-link');
  try {
    await fs.writeFile(file, 'FAKE-AGENT-POSITIVE-CONTROL\n', { mode: 0o600, flag: 'wx' });
    if ((await fs.readFile(file, 'utf8')) !== 'FAKE-AGENT-POSITIVE-CONTROL\n') throw new Error('Positive control failed.');
    const result = await probeOwnerFixture(manifest);
    await fs.symlink(path.join(manifest.root, 'owner-only-fake.txt'), link);
    let denied = false;
    try { const value = await fs.readFile(link); value.fill(0); }
    catch (error) { if (['EACCES', 'EPERM'].includes(error.code)) denied = true; }
    if (!denied) throw new Error('Symlink path was not denied.');
    return { actualStandardIdentityChecked: true, ownWorkspaceControlPassed: true,
      ...result, ownerSymlinkReadDenied: true };
  } finally {
    await fs.unlink(link).catch(error => { if (error.code !== 'ENOENT') throw error; });
    await fs.unlink(file);
    await fs.rmdir(positive);
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.argv.length !== 5) throw new Error('Reviewed plan, fixture and workspace required.');
    const plan = await loadPlan(process.argv[2]);
    const manifest = JSON.parse(await fs.readFile(process.argv[3], 'utf8'));
    const result = await runAgentProbe(plan, manifest, process.argv[4]);
    process.stdout.write(JSON.stringify(result) + '\n');
  } catch {
    process.stderr.write('Isolation probe failed or remains unverified. Do not use a real Agent Key.\n');
    process.exitCode = 1;
  }
}
