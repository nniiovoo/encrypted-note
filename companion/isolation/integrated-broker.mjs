// Owner broker supervised by encrypted-note's Agent Companion helper (a separate XPC service with
// its own macOS sandbox). It runs the same fixed generate_text broker as owner-launcher.mjs, with:
// - fixed data paths inside the helper's own container (HOME), never chosen by a caller;
// - a fresh owner grant of at most one hour for each owner Start, enforced at the broker;
// - a fresh delegated agent token for each Start, removed again on every stop;
// - a bounded owner-control protocol on stdin/stdout, separate from the agent MCP channel.
// Protocol output is fixed codes and numbers only. Nothing from the plan, credential, token,
// provider or MCP traffic is ever written to stdout or stderr.
import path from 'node:path';
import { constants } from 'node:fs';
import { open, lstat, realpath } from 'node:fs/promises';
import { randomBytes } from 'node:crypto';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { performance } from 'node:perf_hooks';
import { checkHandoffDirectory, writeAgentToken, removeAgentToken, tokenIo } from './token-handoff.mjs';
import {
  loadPlan, requireRole, assertProtectedPath, verifyCodeTree, readConfirmedCredential,
  assertGrantActive, grantGuard, MAX_GRANT_MS, checkAcl, run,
} from './common.mjs';

export const PROTOCOL_VERSION = 1;
export const MAX_CONTROL_LINE_BYTES = 128;
export const ERROR_CODES = Object.freeze([
  'plan_missing', 'plan_invalid', 'wrong_identity', 'code_unprotected', 'agent_unverified',
  'credential_missing', 'credential_invalid', 'handoff_unavailable', 'port_unavailable',
  'grant_inactive', 'control_invalid', 'internal',
]);
export const STOP_REASONS = Object.freeze(['owner', 'expired', 'parent', 'error']);
const ADMIN_GID = 80;

class ControlError extends Error {
  constructor(code) { super(code); this.code = ERROR_CODES.includes(code) ? code : 'internal'; }
}
const fail = code => { throw new ControlError(code); };

export function dataPaths(home) {
  if (typeof home !== 'string' || !path.isAbsolute(home) || path.resolve(home) !== home) fail('plan_missing');
  const dir = path.join(home, 'Library', 'Application Support', 'encrypted-note-companion');
  return { dir, plan: path.join(dir, 'plan.json'), credential: path.join(dir, 'credential.env') };
}

// Every protocol line is built here from fixed fields; free text never reaches the helper.
export function encodeEvent(event) {
  let line;
  if (event.event === 'ready' && Number.isSafeInteger(event.expiresAt) && event.expiresAt > 0) {
    line = { v: PROTOCOL_VERSION, event: 'ready', expiresAt: event.expiresAt };
  } else if (event.event === 'stopped' && STOP_REASONS.includes(event.reason)) {
    line = { v: PROTOCOL_VERSION, event: 'stopped', reason: event.reason };
  } else if (event.event === 'error') {
    line = { v: PROTOCOL_VERSION, event: 'error', code: ERROR_CODES.includes(event.code) ? event.code : 'internal' };
  } else if (event.event === 'check') {
    line = { v: PROTOCOL_VERSION, event: 'check' };
    for (const key of ['plan', 'credential', 'handoff', 'code', 'agent']) line[key] = event[key] === true;
  } else {
    line = { v: PROTOCOL_VERSION, event: 'error', code: 'internal' };
  }
  return JSON.stringify(line) + '\n';
}

// The helper may send exactly one command. Anything else is a protocol violation.
export function parseControlLine(line) {
  if (typeof line !== 'string' || Buffer.byteLength(line) > MAX_CONTROL_LINE_BYTES) fail('control_invalid');
  let value;
  try { value = JSON.parse(line); } catch { fail('control_invalid'); }
  if (!value || Object.getPrototypeOf(value) !== Object.prototype || Object.keys(value).length !== 2 ||
      value.v !== PROTOCOL_VERSION || value.op !== 'stop') fail('control_invalid');
  return 'stop';
}

// Group write is accepted only for the admin group on code/runtime ancestors (for example
// /Applications) and only after the agent account is confirmed not to be an administrator.
export async function assertAgentNotAdmin(plan, execute = run) {
  let result;
  try {
    result = await execute('/usr/bin/dsmemberutil',
      ['checkmembership', '-U', plan.agentShortName, '-G', 'admin'], { maxBuffer: 1024, timeout: 5000 });
  } catch { fail('agent_unverified'); }
  if (result.stdout.trim() !== 'user is not a member of the group') fail('agent_unverified');
}
export async function assertProtectedCodePath(file, ownerUid, { stat = lstat, aclCheck = checkAcl } = {}) {
  const adminStat = async current => {
    const info = await stat(current);
    if (!info.isSymbolicLink() && info.gid === ADMIN_GID && (info.mode & 0o002) === 0 && [0, ownerUid].includes(info.uid)) {
      // Present admin-group write as clear; the caller has already confirmed the agent is not an admin.
      return { uid: info.uid, mode: info.mode & ~0o020, isSymbolicLink: () => false, isDirectory: () => info.isDirectory() };
    }
    return info;
  };
  await assertProtectedPath(file, ownerUid, adminStat, { aclCheck });
}

async function verifyCode(ownerUid) {
  const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
  await assertProtectedCodePath(await realpath(process.execPath), ownerUid);
  for (const item of ['src', 'isolation', 'node_modules']) {
    const directory = path.join(root, item);
    await assertProtectedCodePath(directory, ownerUid);
    await verifyCodeTree(directory, ownerUid, { ancestorsChecked: true });
  }
  for (const item of ['policy.json', 'package.json', 'package-lock.json']) {
    await assertProtectedCodePath(path.join(root, item), ownerUid);
  }
}

// Keep the existing token helpers importable from this entry point.
export { checkHandoffDirectory, writeAgentToken } from './token-handoff.mjs';
const defaultIo = { ...tokenIo, verifyCode, assertAgentNotAdmin };

// Metadata-only readiness. The credential's contents are not read here.
export async function checkReadiness({ home, uid = process.getuid(), io = defaultIo } = {}) {
  const result = { plan: false, credential: false, handoff: false, code: false, agent: false };
  let paths; let plan;
  try { paths = dataPaths(home); plan = await loadPlan(paths.plan); requireRole(plan, 'owner', uid); result.plan = true; }
  catch { return result; }
  try {
    await assertProtectedPath(paths.credential, plan.ownerUid, io.stat, { aclCheck: io.aclCheck });
    const handle = await open(paths.credential, constants.O_RDONLY | constants.O_NOFOLLOW);
    try {
      const info = await handle.stat();
      result.credential = info.isFile() && info.uid === plan.ownerUid && (info.mode & 0o777) === 0o600 &&
        info.nlink === 1 && info.size >= 37 && info.size <= 2048;
    } finally { await handle.close(); }
  } catch { /* stays false */ }
  try { await checkHandoffDirectory(plan, io); result.handoff = true; } catch { /* stays false */ }
  try { await io.assertAgentNotAdmin(plan); result.agent = true; } catch { /* stays false */ }
  try { if (result.agent) { await io.verifyCode(plan.ownerUid); result.code = true; } } catch { /* stays false */ }
  return result;
}

export async function startIntegratedBroker({
  home, uid = process.getuid(), now = Date.now, mono = () => performance.now(),
  transport, io = defaultIo, input = process.stdin, emit = line => process.stdout.write(line),
  createToken = () => randomBytes(32).toString('base64url'), signals = process,
  port, // tests only; the production entry point always uses the reviewed plan's port
  setTimer = setTimeout, clearTimer = clearTimeout,
} = {}) {
  const paths = dataPaths(home);
  let plan;
  try { plan = await loadPlan(paths.plan); }
  catch (error) { fail(error?.code === 'ENOENT' ? 'plan_missing' : 'plan_invalid'); }
  try { requireRole(plan, 'owner', uid); } catch { fail('wrong_identity'); }
  // Each owner Start is one new grant ending one hour from now, never later.
  const grant = { ...plan, grantExpiresAt: new Date(now() + MAX_GRANT_MS).toISOString() };
  let expiresAt;
  try { expiresAt = assertGrantActive(grant, now()); } catch { fail('grant_inactive'); }
  try { await io.assertAgentNotAdmin(plan); } catch { fail('agent_unverified'); }
  try { await io.verifyCode(plan.ownerUid); } catch { fail('code_unprotected'); }
  try { await checkHandoffDirectory(plan, io); } catch { fail('handoff_unavailable'); }
  // Delay SDK imports until the protected-tree checks pass, as owner-launcher.mjs does.
  const { createGenerator, guardGenerate, CompanionError } = await import('../src/generate.mjs');
  const { startHttpServer, stopHttpServer } = await import('../src/http.mjs');
  let ready = false;
  const isActive = grantGuard(expiresAt, { now, mono });
  if (!isActive()) fail('grant_inactive');
  let apiKey;
  try { apiKey = await readConfirmedCredential(paths.credential, plan.ownerUid); }
  catch (error) { fail(error?.code === 'ENOENT' ? 'credential_missing' : 'credential_invalid'); }
  const token = createToken();
  if (!/^[A-Za-z0-9_-]{32,256}$/.test(token) || token === apiKey) { apiKey = ''; fail('internal'); }
  let generate;
  try {
    const generator = createGenerator({ apiKey, ...(transport ? { transport } : {}) });
    generate = guardGenerate(async (args, options) => {
      if (!ready) throw new CompanionError('helper_off');
      return generator(args, options);
    }, isActive);
  } catch { apiKey = ''; fail('credential_invalid'); }
  apiKey = ''; // Best effort only; JavaScript strings cannot be guaranteed erased.
  if (!isActive()) fail('grant_inactive');
  let publication;
  let server;
  try { server = await startHttpServer({ generate, bearerToken: token, port: port ?? plan.brokerPort,
    beforeClose: () => { ready = false; return removeAgentToken(publication); } }); }
  catch { fail('port_unavailable'); }
  const closed = new Promise(resolve => server.once('close', resolve));
  const closeServer = () => { stopHttpServer(server); return closed; };
  try {
    publication = await writeAgentToken(plan, token, io, {
      cleanupFailure: async lease => { publication = lease; await closeServer(); },
    });
  }
  catch { await closeServer(); fail('handoff_unavailable'); }
  if (!isActive()) { await closeServer(); fail('grant_inactive'); }

  let stopped = false;
  let resolveDone;
  const done = new Promise(resolve => { resolveDone = resolve; });
  let buffered = '';
  const onData = chunk => {
    buffered += chunk.toString('utf8');
    if (Buffer.byteLength(buffered) > MAX_CONTROL_LINE_BYTES * 4) { void stop('error'); return; }
    let index;
    while ((index = buffered.indexOf('\n')) >= 0) {
      const line = buffered.slice(0, index); buffered = buffered.slice(index + 1);
      try { parseControlLine(line); void stop('owner'); } catch { void stop('error'); }
    }
  };
  const onEnd = () => { void stop('parent'); };
  const onSignal = () => { void stop('owner'); };
  const lifetime = setTimer(() => { void stop('expired'); },
    Math.max(0, Math.min(expiresAt - now(), MAX_GRANT_MS)));
  async function stop(reason) {
    if (stopped) return done;
    stopped = true;
    clearTimer(lifetime);
    input.off?.('data', onData); input.off?.('end', onEnd); input.off?.('close', onEnd);
    // Close the control pipe, or it keeps this process alive after its final event, and the
    // helper learns that a run has ended only when the process exits. pause() isn't enough:
    // called from the data handler, the stream starts reading again once the handler returns.
    input.destroy?.();
    for (const event of ['SIGTERM', 'SIGINT']) signals.removeListener?.(event, onSignal);
    await closeServer();
    emit(encodeEvent({ event: 'stopped', reason }));
    resolveDone(reason);
    return done;
  }
  input.on('data', onData); input.once('end', onEnd); input.once('close', onEnd);
  for (const event of ['SIGTERM', 'SIGINT']) signals.once?.(event, onSignal);
  input.resume?.();
  ready = true;
  emit(encodeEvent({ event: 'ready', expiresAt: Math.floor(expiresAt / 1000) }));
  return { stop, done, server, expiresAt };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const emit = line => process.stdout.write(line);
  try {
    if (process.argv.length === 3 && process.argv[2] === '--check') {
      emit(encodeEvent({ event: 'check', ...(await checkReadiness({ home: process.env.HOME })) }));
    } else if (process.argv.length === 2) {
      const { done } = await startIntegratedBroker({ home: process.env.HOME });
      await done;
      process.exitCode = 0;
    } else {
      fail('internal');
    }
  } catch (error) {
    emit(encodeEvent({ event: 'error', code: error instanceof ControlError ? error.code : 'internal' }));
    process.exitCode = 1;
  }
}
