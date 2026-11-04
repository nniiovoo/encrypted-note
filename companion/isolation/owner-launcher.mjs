import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { realpath } from 'node:fs/promises';
import { randomBytes } from 'node:crypto';
import { tokenIo, checkHandoffDirectory, writeAgentToken, removeAgentToken } from './token-handoff.mjs';
import { loadPlan, requireRole, assertProtectedPath, verifyCodeTree,
         readConfirmedCredential, assertGrantActive, grantGuard, GRANT_INACTIVE,
         MAX_GRANT_MS } from './common.mjs';

async function verifyCode(ownerUid) {
  const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
  await assertProtectedPath(await realpath(process.execPath), ownerUid);
  for (const item of ['src', 'isolation', 'node_modules']) {
    await verifyCodeTree(path.join(root, item), ownerUid);
  }
  for (const item of ['policy.json', 'package.json', 'package-lock.json']) {
    await assertProtectedPath(path.join(root, item), ownerUid);
  }
}
const defaultIo = { ...tokenIo, verifyCode };

export async function launchOwnerBroker(plan, {
  credentialFile, tokenFile, output = process.stderr, io = defaultIo, transport,
  port, // tests only; the production entry point always uses the reviewed plan's port
} = {}) {
  requireRole(plan, 'owner');
  assertGrantActive(plan);
  if (process.platform !== 'darwin') throw new Error('This launcher is for the reviewed Mac only.');
  await io.verifyCode(plan.ownerUid);
  if (tokenFile !== plan.bearerTokenFile) throw new Error('Delegated token path differs from the reviewed plan.');
  await checkHandoffDirectory(plan, io);
  // An independent owner preflight must already trust this launcher and its Node runtime.
  // Delay application/SDK imports until the remaining protected tree checks complete.
  const { createGenerator, guardGenerate } = await import('../src/generate.mjs');
  const { startHttpServer, stopHttpServer } = await import('../src/http.mjs');
  // Grant again immediately before the credential read and before the broker opens.
  const expiresAt = assertGrantActive(plan);
  let apiKey = await readConfirmedCredential(credentialFile, plan.ownerUid);
  const bearerToken = randomBytes(32).toString('base64url');
  if (apiKey === bearerToken) { apiKey = ''; throw new Error('Separate credentials are required.'); }
  const isActive = grantGuard(expiresAt);
  const generate = guardGenerate(createGenerator({ apiKey, ...(transport ? { transport } : {}) }), isActive);
  apiKey = ''; // Best effort only; JavaScript strings cannot be guaranteed erased.
  assertGrantActive(plan);
  if (!isActive()) throw new Error(GRANT_INACTIVE);
  const server = await startHttpServer({ generate, bearerToken, port: port ?? plan.brokerPort });
  let publication;
  try {
    publication = await writeAgentToken(plan, bearerToken, io);
    if (!isActive()) throw new Error(GRANT_INACTIVE);
  } catch (error) {
    stopHttpServer(server);
    await removeAgentToken(publication);
    throw error;
  }
  const stop = () => stopHttpServer(server);
  // Stop at the earlier of the absolute grant deadline and one hour from now. Supplemental
  // to, never a substitute for, provider-enforced key expiry.
  const lifetime = setTimeout(stop, Math.max(0, Math.min(expiresAt - Date.now(), MAX_GRANT_MS)));
  for (const event of ['SIGINT', 'SIGTERM']) process.once(event, stop);
  server.once('close', () => {
    clearTimeout(lifetime);
    for (const event of ['SIGINT', 'SIGTERM']) process.removeListener(event, stop);
    void removeAgentToken(publication);
  });
  output.write('Owner broker is listening on reviewed loopback port 8789 until its grant ends (at most one hour).\n');
  return server;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.argv.length !== 5) throw new Error('Reviewed paths required.');
    const plan = await loadPlan(process.argv[2]);
    await assertProtectedPath(process.argv[2], plan.ownerUid);
    await launchOwnerBroker(plan, { credentialFile: process.argv[3], tokenFile: process.argv[4] });
  } catch {
    process.stderr.write('Owner broker did not start. Resolve the reviewed identity, protected code or confirmed credential-file check.\n');
    process.exitCode = 1;
  }
}
