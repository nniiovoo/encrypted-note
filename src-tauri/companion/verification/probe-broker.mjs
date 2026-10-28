// Fictional-data isolation probe, installed in place of integrated-broker.mjs in a TEST helper.
// It reports through the helper's fixed protocol only: "ready" means every boundary held;
// a distinct error code names the first boundary that did not. It never prints file contents.
import fs from 'node:fs';
import net from 'node:net';
const canary = process.env.HOME.replace(/\/Library\/Containers\/[^/]+\/Data$/, '') +
  '/Library/Containers/' + 'APP_ID/Data/Library/Application Support/APP_ID/Vault.noindex/fake-vault.canary';
const container = canary.slice(0, canary.indexOf('/Data/') + 5);
const say = line => process.stdout.write(JSON.stringify({ v: 1, ...line }) + '\n');
if (process.argv[2] === '--check') {
  say({ event: 'check', plan: true, credential: true, handoff: true, code: true, agent: true });
  process.exit(0);
}
const failed = code => { say({ event: 'error', code }); process.exit(1); };
try { fs.readFileSync(canary); failed('plan_invalid'); } catch (e) { if (e.code !== 'EPERM' && e.code !== 'EACCES') failed('plan_missing'); }
try { fs.readdirSync(container); failed('wrong_identity'); } catch (e) { if (e.code !== 'EPERM' && e.code !== 'EACCES') failed('code_unprotected'); }
try { fs.writeFileSync(canary + '.probe', 'FAKE'); failed('credential_invalid'); } catch {}
const server = net.createServer();
server.once('error', () => failed('port_unavailable'));
server.listen(0, '127.0.0.1', () => {
  server.close();
  say({ event: 'ready', expiresAt: Math.floor(Date.now() / 1000) + 3600 });
  process.stdin.resume();
  process.stdin.on('data', () => { say({ event: 'stopped', reason: 'owner' }); process.exit(0); });
  process.stdin.on('end', () => { say({ event: 'stopped', reason: 'parent' }); process.exit(0); });
});
