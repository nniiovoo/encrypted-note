import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, mkdir, writeFile, readFile, lstat, readlink, symlink, open, rm } from 'node:fs/promises';
import { constants } from 'node:fs';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const execute = promisify(execFile);
const script = fileURLToPath(new URL('./build-reader.sh', import.meta.url));
const fictionalBinary = 'FAKE_READER_BYTES_FOR_PUBLICATION_TEST_ONLY\n';

async function fixture(run) {
  const root = await mkdtemp(path.join(tmpdir(), 'enote-reader-publication-'));
  const bin = path.join(root, 'bin');
  const out = path.join(root, 'out');
  const scratch = path.join(root, 'tmp');
  try {
    await Promise.all([bin, out, scratch].map(folder => mkdir(folder)));
    await writeFile(path.join(bin, 'git'), `#!${process.execPath}
import fs from 'node:fs';
const args = process.argv.slice(2);
while (args[0] === '-C' || args[0] === '-c') args.splice(0, 2);
if (args[0] === 'init') fs.mkdirSync(args.at(-1), { recursive: true });
else if (args[0] === 'rev-parse') process.stdout.write('FAKE_REVIEWED_COMMIT_ONLY\\n');
else if (!['fetch', 'checkout', 'merge-base'].includes(args[0])) process.exit(2);
`, { mode: 0o755 });
    await writeFile(path.join(bin, 'cargo'), `#!${process.execPath}
import fs from 'node:fs';
import path from 'node:path';
const release = path.join(process.env.CARGO_TARGET_DIR, 'release');
fs.mkdirSync(release, { recursive: true });
fs.writeFileSync(path.join(release, 'enote-reader'), ${JSON.stringify(fictionalBinary)});
const output = path.join(process.env.ENOTE_REVIEW_OUT, 'enote-reader');
if (process.env.ENOTE_REVIEW_LATE === 'file') fs.writeFileSync(output, 'FAKE_EXISTING_READER_ONLY\\n');
if (process.env.ENOTE_REVIEW_LATE === 'link') fs.symlinkSync(path.join(process.env.ENOTE_REVIEW_OUT, 'fictional-target'), output);
if (process.env.ENOTE_REVIEW_LATE === 'fifo') fs.renameSync(path.join(process.env.ENOTE_REVIEW_OUT, 'fictional-target'), output);
if (process.env.ENOTE_REVIEW_LATE === 'fifo-link') fs.symlinkSync(path.join(process.env.ENOTE_REVIEW_OUT, 'fictional-target'), output);
`, { mode: 0o755 });
    await writeFile(path.join(bin, 'shasum'), `#!${process.execPath}
import fs from 'node:fs';
import { createHash } from 'node:crypto';
const file = process.argv.at(-1);
// A broken publication must fail its assertion, rather than hanging while hashing a FIFO.
process.stdout.write(fs.lstatSync(file).isFile() ? createHash('sha256').update(fs.readFileSync(file)).digest('hex') : 'FAKE_NONREGULAR_HASH');
`, { mode: 0o755 });
    const build = (late = '') => execute('bash', [script, '--out', out], {
      env: { ...process.env, PATH: bin + path.delimiter + process.env.PATH,
        TMPDIR: scratch, ENOTE_REVIEW_OUT: out, ENOTE_REVIEW_LATE: late },
    });
    await run({ out, build });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

// The owner script is Bash-only; its Windows/Git Bash build remains unverified.
const options = { skip: process.platform === 'win32' };

test('Reader publication saves a fresh executable and reports its actual hash', options, async () => {
  await fixture(async ({ out, build }) => {
    const { stdout } = await build();
    const saved = path.join(out, 'enote-reader');
    assert.equal(await readFile(saved, 'utf8'), fictionalBinary);
    assert.equal((await lstat(saved)).mode & 0o777, 0o755);
    assert.ok(stdout.includes(createHash('sha256').update(fictionalBinary).digest('hex')));
  });
});

test('Reader publication refuses an existing file or dangling link for either binary name', options, async () => {
  for (const name of ['enote-reader', 'enote-reader.exe']) {
    for (const kind of ['file', 'link']) {
      await fixture(async ({ out, build }) => {
        const saved = path.join(out, name);
        const target = path.join(out, 'fictional-target');
        if (kind === 'file') await writeFile(saved, 'FAKE_EXISTING_READER_ONLY\n');
        else await symlink(target, saved);
        await assert.rejects(build());
        if (kind === 'file') assert.equal(await readFile(saved, 'utf8'), 'FAKE_EXISTING_READER_ONLY\n');
        else {
          assert.equal(await readlink(saved), target);
          await assert.rejects(lstat(target), { code: 'ENOENT' });
        }
      });
    }
  }
});

test('Reader publication refuses a file or dangling link created while compiling', options, async () => {
  for (const late of ['file', 'link']) {
    await fixture(async ({ out, build }) => {
      await assert.rejects(build(late));
      const saved = path.join(out, 'enote-reader');
      if (late === 'file') assert.equal(await readFile(saved, 'utf8'), 'FAKE_EXISTING_READER_ONLY\n');
      else {
        assert.equal(await readlink(saved), path.join(out, 'fictional-target'));
        await assert.rejects(lstat(path.join(out, 'fictional-target')), { code: 'ENOENT' });
      }
    });
  }
});

test('Reader publication refuses a FIFO or link to one created while compiling', options, async () => {
  for (const late of ['fifo', 'fifo-link']) {
    await fixture(async ({ out, build }) => {
      const target = path.join(out, 'fictional-target');
      await execute('mkfifo', ['-m', '600', target]);
      // Keep a nonblocking reader open so a faulty writer is observable without hanging.
      const reader = await open(target, constants.O_RDONLY | constants.O_NONBLOCK);
      try {
        await assert.rejects(build(late));
        const node = late === 'fifo' ? path.join(out, 'enote-reader') : target;
        assert.equal((await lstat(node)).mode & 0o777, 0o600);
        assert.equal((await reader.read(Buffer.alloc(128), 0, 128, null)).bytesRead, 0);
      } finally {
        await reader.close();
      }
    });
  }
});
