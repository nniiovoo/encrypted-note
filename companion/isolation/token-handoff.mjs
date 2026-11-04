// Launch-lifetime authority in the owner-prepared, agent-search-only handoff directory.
// The owner-only directory is the boundary: it excludes agent renames, not other owner software.
import path from 'node:path';
import { constants } from 'node:fs';
import { open, lstat, unlink } from 'node:fs/promises';
import { assertProtectedPath, checkAcl, run } from './common.mjs';

export const tokenIo = {
  stat: lstat,
  aclCheck: checkAcl,
  addReadAcl: (file, agent) => run('/bin/chmod', ['+a', 'user:' + agent + ' allow read', file], { maxBuffer: 1024, timeout: 5000 }),
};
const publications = new WeakMap();
const unavailable = () => { throw new Error('Delegated token handoff is unavailable.'); };
const matches = (info, lease) => info.isFile() && !info.isSymbolicLink() &&
  info.dev === lease.dev && info.ino === lease.ino;

export async function checkHandoffDirectory(plan, { stat = lstat, aclCheck = checkAcl } = {}) {
  const dir = path.dirname(plan.bearerTokenFile);
  const info = await stat(dir);
  if (!info.isDirectory() || info.isSymbolicLink() || info.uid !== plan.ownerUid || (info.mode & 0o777) !== 0o700) unavailable();
  await assertProtectedPath(dir, plan.ownerUid, stat, {
    tokenAgent: plan.agentShortName,
    aclCheck: (file, permitted) => aclCheck(file, file === dir ? 'user:' + plan.agentShortName + ' allow search' : permitted),
  });
}

// Call only after successfully binding this launch's listener. Retain the open handle until
// cleanup, so an unlinked publication's inode cannot be recycled into a successor's lease.
export async function writeAgentToken(plan, token, io = tokenIo) {
  const file = plan.bearerTokenFile;
  try {
    const stale = await io.stat(file);
    if (!stale.isFile() || stale.isSymbolicLink() || stale.uid !== plan.ownerUid || stale.nlink !== 1) unavailable();
    await unlink(file);
  } catch (error) { if (error?.code !== 'ENOENT') throw error; }
  const handle = await open(file, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600);
  let lease;
  try {
    const info = await handle.stat();
    lease = Object.freeze({ file, dev: info.dev, ino: info.ino });
    publications.set(lease, handle);
    await handle.writeFile(token + '\n');
    await handle.sync();
    await io.addReadAcl(file, plan.agentShortName);
    await assertProtectedPath(file, plan.ownerUid, io.stat, { aclCheck: io.aclCheck, tokenAgent: plan.agentShortName });
    if (!matches(await lstat(file), lease)) unavailable();
    return lease;
  } catch (error) {
    if (lease) await removeAgentToken(lease);
    else await handle.close();
    throw error;
  }
}

// A pathname is never cleanup authority. Only a lease issued by this module can remove the
// exact publication it owns. There is no same-owner malicious rename protection in Node's unlink.
export async function removeAgentToken(lease) {
  const handle = lease && publications.get(lease);
  if (!handle) return;
  publications.delete(lease);
  try {
    if (matches(await lstat(lease.file), lease)) await unlink(lease.file);
  } catch { /* gone, replaced, or no longer removable */ }
  finally { await handle.close(); }
}
