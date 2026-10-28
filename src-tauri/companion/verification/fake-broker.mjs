// Fictional supervision payload for a TEST helper. MODE is replaced per scenario by
// scripts/verify-companion.sh. It never handles a real key.
const MODE = 'MODE_VALUE';
const say = line => process.stdout.write(JSON.stringify({ v: 1, ...line }) + '\n');
if (process.argv[2] === '--check') {
  say({ event: 'check', plan: true, credential: true, handoff: true, code: true, agent: true });
  process.exit(0);
}
const now = Math.floor(Date.now() / 1000);
if (MODE === 'markers') {
  process.stderr.write('FAKE_OPENAI_AGENT_KEY_MARKER sk-FAKE-0000 Bearer FAKE_TOKEN_MARKER\n');
  process.stdout.write('FAKE_PROVIDER_ERROR_MARKER sk-FAKE-0000\n');
  setTimeout(() => process.exit(0), 2000);
} else if (MODE === 'oversized') {
  process.stdout.write('{"v":1,"event":"ready","expiresAt":' + (now + 60) + ',"pad":"' + 'x'.repeat(8192) + '"}\n');
  setTimeout(() => process.exit(0), 2000);
} else {
  say({ event: 'ready', expiresAt: MODE === 'expire' ? now + 2 : now + 3600 });
  process.stdin.resume();
  if (MODE === 'normal') {
    process.stdin.on('data', () => { say({ event: 'stopped', reason: 'owner' }); process.exit(0); });
    process.stdin.on('end', () => { say({ event: 'stopped', reason: 'parent' }); process.exit(0); });
  } else if (MODE === 'crash') {
    setTimeout(() => process.exit(3), 1000);
  } else if (MODE === 'expire') {
    setTimeout(() => { say({ event: 'stopped', reason: 'expired' }); process.exit(0); }, 2000);
  } else if (MODE === 'ignore-stop') {
    process.stdin.on('data', () => {}); process.stdin.on('end', () => {});
    process.on('SIGTERM', () => {}); // only SIGKILL ends it
    setInterval(() => {}, 1000);
  }
}
