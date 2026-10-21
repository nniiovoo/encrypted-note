import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { realpath } from 'node:fs/promises';
import { loadPlan, requireRole, assertProtectedPath, verifyCodeTree,
         readConfirmedCredential, readAgentToken } from './common.mjs';

export async function launchOwnerBroker(plan, { credentialFile, tokenFile, output = process.stderr } = {}) {
  requireRole(plan, 'owner');
  if (process.platform !== 'darwin') throw new Error('This launcher is for the reviewed Mac only.');
  const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
  await assertProtectedPath(await realpath(process.execPath), plan.ownerUid);
  for (const item of ['src', 'isolation', 'node_modules']) {
    await verifyCodeTree(path.join(root, item), plan.ownerUid);
  }
  for (const item of ['policy.json', 'package.json', 'package-lock.json']) {
    await assertProtectedPath(path.join(root, item), plan.ownerUid);
  }
  if (tokenFile !== plan.bearerTokenFile) throw new Error('Delegated token path differs from the reviewed plan.');
  await assertProtectedPath(tokenFile, plan.ownerUid, undefined, { tokenAgent: plan.agentShortName });
  // An independent owner preflight must already trust this launcher and its Node runtime.
  // Delay application/SDK imports until the remaining protected tree checks complete.
  const { createGenerator } = await import('../src/generate.mjs');
  const { startHttpServer } = await import('../src/http.mjs');
  let apiKey = await readConfirmedCredential(credentialFile, plan.ownerUid);
  const bearerToken = await readAgentToken(tokenFile, plan.ownerUid);
  if (apiKey === bearerToken) { apiKey = ''; throw new Error('Separate credentials are required.'); }
  const generate = createGenerator({ apiKey });
  apiKey = ''; // Best effort only; JavaScript strings cannot be guaranteed erased.
  const server = await startHttpServer({ generate, bearerToken, port: plan.brokerPort });
  const stop = () => { server.close(); server.closeAllConnections(); };
  // Supplemental lifetime, never a substitute for provider-enforced one-hour expiry.
  const lifetime = setTimeout(stop, plan.credentialExpirySeconds * 1000);
  for (const event of ['SIGINT', 'SIGTERM']) process.once(event, stop);
  server.once('close', () => {
    clearTimeout(lifetime);
    for (const event of ['SIGINT', 'SIGTERM']) process.removeListener(event, stop);
  });
  output.write('Owner broker is listening on reviewed loopback port 8789 for at most one hour.\n');
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
