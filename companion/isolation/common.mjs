import { lstat, realpath, open, readFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { performance } from 'node:perf_hooks';
export const run = promisify(execFile);
export function validatePlan(plan) {
  const keys = ['version', 'ownerUid', 'agentUid', 'agentShortName', 'credentialExpirySeconds',
    'projectHardSpendLimitCents', 'projectHardSpendCurrency', 'projectHardSpendInterval',
    'brokerHost', 'brokerPort', 'providerModel', 'providerScope', 'bearerTokenFile', 'grantExpiresAt'];
  if (plan?.version === 1) {
    throw new Error('Version-1 isolation plans are no longer accepted. Prepare a new reviewed version-2 plan.');
  }
  if (!plan || Object.getPrototypeOf(plan) !== Object.prototype ||
      Object.keys(plan).some(key => !keys.includes(key)) || !Object.hasOwn(plan, 'grantExpiresAt') ||
      (plan.grantExpiresAt !== null && !isCanonicalUtc(plan.grantExpiresAt)) ||
      plan.version !== 2 || !Number.isSafeInteger(plan.ownerUid) || plan.ownerUid < 501 ||
      // From 2^31 up are system accounts such as nobody (4294967294), never a login account.
      !Number.isSafeInteger(plan.agentUid) || plan.agentUid < 501 || plan.agentUid >= 2147483648 ||
      plan.ownerUid === plan.agentUid ||
      !/^[a-z][a-z0-9_-]{0,30}$/.test(plan.agentShortName ?? '') ||
      plan.credentialExpirySeconds !== 3600 || plan.projectHardSpendLimitCents !== 100 ||
      plan.projectHardSpendCurrency !== 'USD' || plan.projectHardSpendInterval !== 'month' ||
      plan.brokerHost !== '127.0.0.1' || plan.brokerPort !== 8789 ||
      plan.providerModel !== 'gpt-6-luna' || plan.providerScope !== 'api.responses.write' ||
      typeof plan.bearerTokenFile !== 'string' || !path.isAbsolute(plan.bearerTokenFile) ||
      path.resolve(plan.bearerTokenFile) !== plan.bearerTokenFile) {
    throw new Error('The reviewed nonsecret isolation plan is incomplete or changed.');
  }
  return plan;
}
// A grant is one owner-approved hour at most, ending at an absolute UTC instant.
export const MAX_GRANT_MS = 3_600_000;
export const GRANT_INACTIVE = 'The approved agent grant is missing, expired or too long. Prepare a new reviewed grant.';
export function isCanonicalUtc(value) {
  if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?Z$/.test(value)) return false;
  const parsed = Date.parse(value);
  if (!Number.isFinite(parsed)) return false;
  const canonical = new Date(parsed).toISOString();
  return canonical === value || canonical.replace('.000Z', 'Z') === value;
}
// Checked before any credential read, before the broker opens and before each provider admission.
export function assertGrantActive(plan, nowMs = Date.now()) {
  const expires = isCanonicalUtc(plan?.grantExpiresAt) ? Date.parse(plan.grantExpiresAt) : NaN;
  if (!Number.isFinite(nowMs) || !Number.isFinite(expires) || nowMs >= expires || expires - nowMs > MAX_GRANT_MS) {
    throw new Error(GRANT_INACTIVE);
  }
  return expires;
}
// Admission guard: the earlier of the absolute deadline and one hour of monotonic time since start.
// A backwards wall-clock change cannot add time; a forward change ends the grant early.
export function grantGuard(expiresAtMs, { now = Date.now, mono = () => performance.now() } = {}) {
  const started = mono();
  return () => now() < expiresAtMs && mono() - started < MAX_GRANT_MS;
}
export function requireRole(plan, role, uid = process.getuid()) {
  validatePlan(plan);
  const expected = role === 'owner' ? plan.ownerUid : plan.agentUid;
  if (uid === 0 || uid !== expected) throw new Error('Wrong execution identity; no fixture or credential was accessed.');
}
export async function loadPlan(file) {
  const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const info = await handle.stat();
    if (!info.isFile() || info.size > 8192) throw new Error('Invalid nonsecret isolation plan.');
    return validatePlan(JSON.parse(await readBounded(handle, 8192, info)));
  } finally { await handle.close(); }
}
export async function assertProtectedPath(file, ownerUid, stat = lstat, { aclCheck = checkAcl, tokenAgent } = {}) {
  const target = path.resolve(file);
  let current = target;
  while (true) {
    const info = await stat(current);
    const shared = tokenAgent && current === '/Users/Shared' && info.uid === 0 &&
      info.isDirectory() && (info.mode & 0o1777) === 0o1777;
    if (info.isSymbolicLink() || ![0, ownerUid].includes(info.uid) ||
        ((info.mode & 0o022) !== 0 && !shared)) throw new Error('A controlling code or runtime path is not protected.');
    const permitted = tokenAgent && current === target ? 'user:' + tokenAgent + ' allow read' :
      tokenAgent && current === path.dirname(target) ? 'user:' + tokenAgent + ' allow search' : null;
    await aclCheck(current, permitted);
    const parent = path.dirname(current);
    if (parent === current) break;
    current = parent;
  }
}
export async function assertStandardIdentity(plan, execute = run, uid = process.getuid()) {
  requireRole(plan, 'agent', uid);
  const groups = await execute('/usr/bin/id', ['-Gn'], { maxBuffer: 8192 });
  if (groups.stdout.trim().split(/\s+/).some(group => ['admin', 'wheel', 'operator'].includes(group))) {
    throw new Error('The agent account has a privileged group.');
  }
  const actual = await execute('/usr/bin/id', ['-un'], { maxBuffer: 8192 });
  if (actual.stdout.trim() !== plan.agentShortName) throw new Error('The agent account does not match the plan.');
  let hasSudo = false;
  try { await execute('/usr/bin/sudo', ['-n', '-l'], { maxBuffer: 8192, timeout: 5000 }); hasSudo = true; }
  catch (error) {
    if (error.killed || !Number.isInteger(error.code)) throw new Error('Could not check noninteractive sudo authorization.');
  }
  if (hasSudo) throw new Error('Agent sudo authorization is present; stop before credentials.');
}
export async function readAgentToken(file, uid = process.getuid()) {
  if (!path.isAbsolute(file ?? '')) throw new Error('A private agent-token path is required.');
  const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const info = await handle.stat();
    if (!info.isFile() || info.uid !== uid || (info.mode & 0o077) !== 0 || info.nlink !== 1 ||
        info.size < 32 || info.size > 257) throw new Error('The delegated token file is not private.');
    const token = (await readBounded(handle, 257, info)).trimEnd();
    if (!/^[A-Za-z0-9_-]{32,256}$/.test(token)) throw new Error('Invalid delegated token.');
    return token;
  } finally { await handle.close(); }
}
// A process per entry takes longer over the real dependency tree than the helper's readiness
// check is allowed, so ACLs are listed in batches.
const ACL_BATCH_SIZE = 128;
export async function verifyCodeTree(directory, ownerUid, { ancestorsChecked = false, aclBatch = checkAclBatch } = {}) {
  // Callers that verified the directory and its ancestors themselves pass ancestorsChecked.
  if (!ancestorsChecked) await assertProtectedPath(directory, ownerUid);
  const { readdir } = await import('node:fs/promises');
  const pending = [];
  const flush = async () => { if (pending.length > 0) await aclBatch(pending.splice(0)); };
  const walk = async current => {
    for (const entry of await readdir(current, { withFileTypes: true })) {
      // npm command shims are not part of this runtime and are never executed.
      if (entry.name === '.bin' && path.basename(current) === 'node_modules') continue;
      const child = path.join(current, entry.name);
      const info = await lstat(child);
      if (info.isSymbolicLink() || ![0, ownerUid].includes(info.uid) || (info.mode & 0o022) !== 0) {
        throw new Error('A code or dependency path is not protected.');
      }
      pending.push(child);
      if (pending.length >= ACL_BATCH_SIZE) await flush();
      if (entry.isDirectory()) await walk(child);
      else if (!entry.isFile()) throw new Error('Unexpected runtime file type.');
    }
  };
  await walk(directory);
  await flush();
}

export function parseConfirmedCredential(contents) {
  if (Buffer.byteLength(contents, 'utf8') > 2048) throw new Error('Invalid confirmed credential file.');
  const match = /^OPENAI_AGENT_API_KEY=(?:"([A-Za-z0-9_-]{16,512})"|'([A-Za-z0-9_-]{16,512})'|([A-Za-z0-9_-]{16,512}))\r?\n?$/.exec(contents);
  if (!match) throw new Error('The confirmed file must contain only the separate Agent Key assignment.');
  return match[1] ?? match[2] ?? match[3];
}
export async function readConfirmedCredential(file, ownerUid) {
  await assertProtectedPath(file, ownerUid);
  const handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const info = await handle.stat();
    if (!info.isFile() || info.uid !== ownerUid || (info.mode & 0o777) !== 0o600 ||
        info.nlink !== 1 || info.size > 2048) throw new Error('The confirmed credential file is not owner-only 0600.');
    return parseConfirmedCredential(await readBounded(handle, 2048, info));
  } finally { await handle.close(); }
}

export async function readBounded(handle, limit, before) {
  const buffer = Buffer.alloc(limit + 1);
  try {
    const { bytesRead } = await handle.read(buffer, 0, buffer.length, 0);
    const after = await handle.stat();
    if (bytesRead > limit || after.size !== before.size || after.mtimeMs !== before.mtimeMs || bytesRead !== after.size) throw new Error('The protected file changed or exceeds its bound.');
    return new TextDecoder('utf-8', { fatal: true }).decode(buffer.subarray(0, bytesRead));
  } finally { buffer.fill(0); }
}
export function validateAclListing(listing, permitted = null) {
  for (const line of listing.split('\n').slice(1)) {
    if (!line.trim()) continue;
    const entry = line.trim().replace(/^\d+:\s*/, '');
    if (/\ballow\b/.test(entry) && entry !== permitted) throw new Error('Unexpected ACL allow grant.');
  }
}
export async function checkAcl(file, permitted = null) {
  const { stdout } = await run('/bin/ls', ['-lde', file], { maxBuffer: 8192, timeout: 5000 });
  validateAclListing(stdout, permitted);
}
// `ls -lde` prints each file on a line starting with its mode, then its ACL entries on indented
// lines. A file name can hold any text, including a newline, so only indented lines are read: a
// name that looks like an entry can only cause a refusal, never hide a real one.
export function validateAclBatchListing(listing) {
  for (const line of listing.split('\n')) {
    if (/^\s/.test(line) && /\ballow\b/.test(line)) throw new Error('Unexpected ACL allow grant.');
  }
}
async function checkAclBatch(files) {
  // A missing file, a timeout or too much output rejects here, so the check fails closed.
  const { stdout } = await run('/bin/ls', ['-lde', ...files], { maxBuffer: 1024 * 1024, timeout: 5000 });
  validateAclBatchListing(stdout);
}
