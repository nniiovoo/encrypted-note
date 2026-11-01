import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const run = promisify(execFile);
const root = fileURLToPath(new URL('../', import.meta.url));
const guide = await readFile(new URL('../docs/keyless-code.md', import.meta.url), 'utf8');
const example = guide.match(/```js\n([\s\S]*?)\n```/)?.[1];
assert.ok(example, 'The documented example must exist.');

test('documented keyless client works and closes with a fictional stdio provider', async () => {
  const parent = path.join(root, '.scratch');
  await mkdir(parent, { recursive: true });
  const directory = await mkdtemp(path.join(parent, 'keyless-example-'));
  try {
    const source = example
      .replace("'/ABSOLUTE/PATH/TO/TRUSTED/node'", JSON.stringify(process.execPath))
      .replace("'/ABSOLUTE/PATH/TO/COMPANION/isolation/agent-bridge.mjs'",
        JSON.stringify(path.join(root, 'fixtures/fake-stdio.mjs')))
      .replace("'/ABSOLUTE/PATH/TO/REVIEWED/agent-plan.json'",
        JSON.stringify(path.join(directory, 'fictional-nonsecret-plan.json')));
    const script = path.join(directory, 'example.mjs');
    await writeFile(script, source);
    await run(process.execPath, ['--check', script], { env: {}, timeout: 10_000 });
    const result = await run(process.execPath, [script], { env: {}, timeout: 10_000 });
    assert.equal(result.stdout, 'Fictional stdio output.\n');
    assert.equal(result.stderr, '');

    await writeFile(script, source.replace('Fictional example: write a short greeting.', ' '));
    await assert.rejects(run(process.execPath, [script], { env: {}, timeout: 10_000 }), error => {
      assert.equal(error.code, 1);
      assert.equal(error.killed, false);
      assert.equal(error.stdout, '');
      assert.equal(error.stderr, 'Text generation did not finish. Check the local broker setup.\n');
      return true;
    });
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
