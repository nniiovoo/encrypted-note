// Owner only: writes the NEW Agent Key file that Settings → Agent Companion → Set up… reads.
//
//   pbpaste | node write-agent-key.mjs ~/enote-agent-key.env
//
// The key comes in on stdin, never as an argument, so it stays out of shell history and the
// process list. A value that doesn't look like an OpenAI API key is refused before anything is
// written, so a password or Seed Phrase left on the clipboard can't land in the file. Anthropic
// (sk-ant-) and OpenRouter (sk-or-) keys are refused too, but an OpenAI key copied from the Vault
// would still pass. No message ever includes the value.
import { lstat, open, rename, unlink } from 'node:fs/promises';
import { constants, realpathSync } from 'node:fs';
import { randomUUID } from 'node:crypto';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { run, parseConfirmedCredential, checkAcl, assertProtectedPath } from './common.mjs';

const MAX_INPUT_BYTES = 4096;
export const KEY_REFUSED = "That doesn't look like an OpenAI API key (one line starting with sk-, no spaces), " +
  'so nothing was written. Copy the new Agent Key at OpenAI, then run this again.';
export const ADMIN_KEY_REFUSED = 'That is an OpenAI Admin key, so nothing was written. ' +
  'Create a new project key with only the api.responses.write permission.';
export const FILE_REFUSED = 'The key file must be new or a plain file of your own, with no ACL that lets another account in. ' +
  'Nothing was written.';
export const ACL_APPEARED = 'An ACL that lets another account in appeared on the key file while it was written, ' +
  'so the key was taken out again. Treat it as seen: revoke it at OpenAI, make a new one, and run this again.';
export const USAGE = 'Pipe the key in, for example: pbpaste | node write-agent-key.mjs ~/enote-agent-key.env. Nothing was written.';

// Project, service-account and older user keys: sk-, then letters, digits, - and _ only.
// Anthropic and OpenRouter keys start with sk- as well, so they are refused by name.
export function checkAgentKeyShape(value) {
  if (typeof value !== 'string' || !/^sk-[A-Za-z0-9_-]{37,509}$/.test(value)) return KEY_REFUSED;
  if (/^sk-(?:ant|or)-/.test(value)) return KEY_REFUSED;
  if (value.startsWith('sk-admin-')) return ADMIN_KEY_REFUSED;
  return null;
}

// Best effort: a failure leaves the file in place and only changes what the owner is told.
export async function excludeFromBackup(file) {
  if (process.platform !== 'darwin') return false;
  try { await run('/usr/bin/tmutil', ['addexclusion', file], { maxBuffer: 1024, timeout: 10_000 }); return true; }
  catch { return false; }
}

export async function writeAgentKey(file, input, { exclude = excludeFromBackup, aclCheck = checkAcl } = {}) {
  const value = Buffer.isBuffer(input) && input.length <= MAX_INPUT_BYTES
    ? new TextDecoder('utf-8', { fatal: false }).decode(input) : '';
  const refused = checkAgentKeyShape(value);
  if (refused) throw new Error(refused);
  const contents = Buffer.from('OPENAI_AGENT_API_KEY=' + value + '\n', 'utf8');
  try {
    parseConfirmedCredential(contents.toString('utf8')); // the same check Start makes
    file = path.resolve(file);
    const parent = path.dirname(file);
    const protectParent = () => assertProtectedPath(parent, process.getuid())
      .catch(() => { throw new Error(FILE_REFUSED); });
    const inspectDestination = async () => {
      let info;
      try { info = await lstat(file); }
      catch (error) { if (error.code === 'ENOENT') return null; throw new Error(FILE_REFUSED); }
      if (!info.isFile() || info.uid !== process.getuid() || info.nlink !== 1) throw new Error(FILE_REFUSED);
      await aclCheck(file).catch(() => { throw new Error(FILE_REFUSED); });
      return info;
    };
    await protectParent();
    const before = await inspectDestination();
    const temporary = path.join(parent, '.enote-agent-key-' + randomUUID());
    const handle = await open(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600)
      .catch(() => { throw new Error(FILE_REFUSED); });
    let published = false;
    try {
      const info = await handle.stat();
      if (!info.isFile() || info.uid !== process.getuid() || info.nlink !== 1) throw new Error(FILE_REFUSED);
      // An allow entry, set on the file or inherited from its folder, lets another account read it
      // whatever its mode.
      await aclCheck(temporary).catch(() => { throw new Error(FILE_REFUSED); });
      await handle.chmod(0o600);
      await handle.writeFile(contents);
      await handle.sync();
      // Checked again before success is reported; if one appeared, the key is taken out again.
      await aclCheck(temporary).catch(async () => {
        await handle.truncate(0);
        await handle.sync();
        throw new Error(ACL_APPEARED);
      });
      await protectParent();
      const after = await inspectDestination();
      if (before?.dev !== after?.dev || before?.ino !== after?.ino ||
          before?.mtimeMs !== after?.mtimeMs || before?.ctimeMs !== after?.ctimeMs) throw new Error(FILE_REFUSED);
      // A fresh inode keeps an earlier reader's open descriptor away from the new key.
      await rename(temporary, file);
      published = true;
    } finally {
      if (!published) {
        await handle.truncate(0).catch(() => {});
        await handle.sync().catch(() => {});
      }
      try { await handle.close(); }
      finally { if (!published) await unlink(temporary).catch(() => {}); }
    }
  } finally { contents.fill(0); }
  return { excluded: await exclude(file) };
}

// Input past the bound is refused as too long, however the reads split it, so a key-shaped first
// part is never judged on its own.
export async function readInput(stream) {
  const chunks = [];
  let size = 0;
  for await (const chunk of stream) {
    size += chunk.length;
    if (size <= MAX_INPUT_BYTES) chunks.push(chunk);
  }
  return size > MAX_INPUT_BYTES ? Buffer.alloc(MAX_INPUT_BYTES + 1) : Buffer.concat(chunks);
}

// Node resolves links for this module's own URL, so a linked path must count as this file too.
if (process.argv[1] && import.meta.url === pathToFileURL(realpathSync(process.argv[1])).href) {
  let input = Buffer.alloc(0);
  try {
    if (process.argv.length !== 3 || process.stdin.isTTY) {
      throw new Error(USAGE);
    }
    const file = path.resolve(process.argv[2]);
    input = await readInput(process.stdin);
    const { excluded } = await writeAgentKey(file, input);
    process.stdout.write('Wrote ' + file + ', readable only by your Mac account. ' + (excluded
      ? 'Asked Time Machine to leave it out.'
      : 'Time Machine was not told to leave it out, so delete it once the agent\'s first call has returned text.') +
      ' Now copy something else, so the key leaves the clipboard.\n');
  } catch (error) {
    // Only this file's fixed texts are printed; anything else could carry part of the input.
    const known = [KEY_REFUSED, ADMIN_KEY_REFUSED, FILE_REFUSED, ACL_APPEARED, USAGE].includes(error?.message);
    process.stderr.write((known ? error.message : 'The Agent Key file could not be written. Delete it if it exists, then run this again.') + '\n');
    process.exitCode = 1;
  } finally { input.fill(0); }
}
