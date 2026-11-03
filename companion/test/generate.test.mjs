import test from 'node:test';
import assert from 'node:assert/strict';
import https from 'node:https';
import { EventEmitter } from 'node:events';
import { Readable } from 'node:stream';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { createGenerator, makeProviderBody, sendToOpenAI, POLICY } from '../src/generate.mjs';
import { readRuntimeConfig } from '../src/config.mjs';

const KEY = 'FAKE_OPENAI_AGENT_KEY_FOR_TESTS_ONLY';
function success(text = 'Fictional generated text.') {
  return { status: 200, json: { model: POLICY.model, status: 'completed',
    output: [{ type: 'message', role: 'assistant', content: [{ type: 'output_text', text }] }] } };
}
test('constructs one fixed text-only provider operation', async () => {
  let received;
  const generate = createGenerator({ apiKey: KEY, transport: async args => { received = args; return success(); } });
  assert.deepEqual(await generate({ text: 'Write a fictional greeting.' }), { text: 'Fictional generated text.' });
  assert.deepEqual(received.body, makeProviderBody('Write a fictional greeting.'));
  assert.deepEqual(Object.keys(received).sort(), ['apiKey', 'body', 'signal']);
  assert.equal(received.body.model, 'gpt-6-luna');
  assert.equal(received.body.max_output_tokens, 512);
  assert.equal(received.body.store, false);
  assert.equal(received.body.stream, false);
  assert.equal(received.body.background, false);
  assert.deepEqual(received.body.tools, []);
  assert.equal(received.body.tool_choice, 'none');
  assert.equal(received.body.reasoning.effort, 'none');
  assert.equal(received.body.service_tier, 'default');
});
test('rejects extra model, tools, files, URLs, headers, history and policy overrides before transport', async () => {
  let calls = 0;
  const generate = createGenerator({ apiKey: KEY, transport: async () => { calls++; return success(); } });
  for (const field of ['model', 'tools', 'file', 'url', 'headers', 'previous_response_id',
                       'conversation', 'store', 'max_output_tokens', 'OPENAI_BASE_URL']) {
    await assert.rejects(generate({ text: 'Fictional input.', [field]: 'https://unallowed.invalid/' }),
      error => error.code === 'invalid_input');
  }
  assert.equal(calls, 0);
});
test('bounds UTF-8 input, rejects malformed strings and accepts the exact byte boundary', async () => {
  let calls = 0;
  const generate = createGenerator({ apiKey: KEY, transport: async () => { calls++; return success(); } });
  for (const args of [null, [], {}, { text: '' }, { text: '  \n' }, { text: 4 },
                      { text: '\ud800' }, { text: 'a'.repeat(4097) }, { text: '😀'.repeat(1025) }]) {
    await assert.rejects(generate(args), error => error.code === 'invalid_input');
  }
  await generate({ text: '😀'.repeat(1024) });
  assert.equal(calls, 1);
});
test('failed requests consume rate admission and invalid inputs do not', async () => {
  let instant = 0;
  let calls = 0;
  const generate = createGenerator({ apiKey: KEY, now: () => instant,
    transport: async () => { calls++; return { status: 429 }; } });
  for (let i = 0; i < 20; i++) await assert.rejects(generate({ text: '' }), { code: 'invalid_input' });
  for (let i = 0; i < 6; i++) await assert.rejects(generate({ text: 'Fiction.' }), { code: 'provider_limit' });
  await assert.rejects(generate({ text: 'Fiction.' }), { code: 'usage_limit' });
  assert.equal(calls, 6);
  instant = 60_000;
  await assert.rejects(generate({ text: 'Fiction.' }), { code: 'provider_limit' });
  assert.equal(calls, 7);
});
test('enforces hourly and total-process admission', async () => {
  let instant = 0;
  const generate = createGenerator({ apiKey: KEY, now: () => instant, transport: async () => success() });
  for (let i = 0; i < 60; i++) {
    await generate({ text: 'Fiction.' }); instant += 11_000;
  }
  await assert.rejects(generate({ text: 'Fiction.' }), { code: 'usage_limit' });
  instant = 3_600_000;
  for (let i = 0; i < 40; i++) {
    await generate({ text: 'Fiction.' }); instant += 11_000;
  }
  instant += 86_400_000;
  await assert.rejects(generate({ text: 'Fiction.' }), { code: 'usage_limit' });
});
test('permits only one concurrent provider request', async () => {
  let complete;
  const pending = new Promise(resolve => { complete = resolve; });
  const generate = createGenerator({ apiKey: KEY, transport: async () => pending });
  const first = generate({ text: 'Fiction.' });
  await assert.rejects(generate({ text: 'More fiction.' }), { code: 'busy' });
  complete(success());
  await first;
});
test('times out and aborts without retrying', async () => {
  let calls = 0;
  let signal;
  const generate = createGenerator({ apiKey: KEY, timeoutMs: 15,
    transport: async args => { calls++; signal = args.signal; return new Promise(() => {}); } });
  await assert.rejects(generate({ text: 'Fiction.' }), { code: 'timeout' });
  assert.equal(calls, 1);
  assert.equal(signal.aborted, true);
});
test('cancellation aborts the admitted provider request', async () => {
  const controller = new AbortController();
  let signal;
  const generate = createGenerator({ apiKey: KEY,
    transport: async args => { signal = args.signal; return new Promise(() => {}); } });
  const request = generate({ text: 'Fiction.' }, { signal: controller.signal });
  await Promise.resolve();
  controller.abort();
  await assert.rejects(request, { code: 'cancelled' });
  assert.equal(signal.aborted, true);
});
test('never forwards provider error bodies, headers, keys or thrown messages', async () => {
  const marker = 'FAKE_PRIVATE_ERROR_MARKER';
  for (const status of [301, 307, 400, 401, 403, 429, 500]) {
    const generate = createGenerator({ apiKey: KEY, transport: async () => ({
      status, json: { error: { message: marker + KEY } }, headers: { secret: marker },
    }) });
    await assert.rejects(generate({ text: 'Fiction.' }), error =>
      !error.message.includes(marker) && !error.message.includes(KEY));
  }
  const throwing = createGenerator({ apiKey: KEY, transport: async () => { throw new Error(marker + KEY); } });
  await assert.rejects(throwing({ text: 'Fiction.' }), error =>
    error.code === 'provider_unavailable' && !error.message.includes(marker) && !error.message.includes(KEY));
});
test('each provider status gets its own fixed error and next step', async () => {
  for (const [status, code] of [[400, 'provider_rejected'], [404, 'provider_rejected'], [401, 'provider_auth'],
    [403, 'provider_auth'], [429, 'provider_limit'], [500, 'provider_unavailable'], [503, 'provider_unavailable'],
    [307, 'provider_unavailable']]) {
    const generate = createGenerator({ apiKey: KEY, transport: async () => ({ status }) });
    await assert.rejects(generate({ text: 'Fiction.' }), { code });
  }
});
test('a model refusal says so and returns none of its wording', async () => {
  const wording = 'FAKE_PRIVATE_REFUSAL_WORDING';
  const refusal = { type: 'refusal', refusal: wording };
  for (const content of [[refusal], [{ type: 'output_text', text: 'A fictional start' }, refusal]]) {
    const generate = createGenerator({ apiKey: KEY, transport: async () => ({ status: 200, json: { ...success().json,
      output: [{ type: 'message', role: 'assistant', content }] } }) });
    await assert.rejects(generate({ text: 'Fiction.' }), error =>
      error.code === 'model_refusal' && /Do not send the same text again\./.test(error.message) &&
      !error.message.includes(wording) && !error.message.includes('fictional start'));
  }
  // Only the fixed model's completed reply counts as a refusal.
  for (const json of [{ model: 'other-model' }, { status: 'failed' }]) {
    const generate = createGenerator({ apiKey: KEY, transport: async () => ({ status: 200, json: { ...success().json,
      ...json, output: [{ type: 'message', role: 'assistant', content: [refusal] }] } }) });
    await assert.rejects(generate({ text: 'Fiction.' }), { code: 'invalid_response' });
  }
});
test('rejects unexpected model, tools, incomplete output, oversized text and exact-key echoes', async () => {
  const responses = [
    { status: 200, json: { ...success().json, model: 'other-model' } },
    { status: 200, json: { ...success().json, status: 'incomplete' } },
    { status: 200, json: { ...success().json, output: [{ type: 'function_call' }] } },
    success('a'.repeat(POLICY.maxOutputBytes + 1)),
    success(KEY),
    success('\ud800'),
  ];
  for (const result of responses) {
    const generate = createGenerator({ apiKey: KEY, transport: async () => result });
    await assert.rejects(generate({ text: 'Fiction.' }), { code: 'invalid_response' });
  }
});
test('an answer cut off at the output limit says so and returns none of it', async () => {
  const cutOff = { ...success('A fictional answer that stops mid').json,
    status: 'incomplete', incomplete_details: { reason: 'max_output_tokens' } };
  const generate = createGenerator({ apiKey: KEY, transport: async () => ({ status: 200, json: cutOff }) });
  await assert.rejects(generate({ text: 'Fiction.' }), error =>
    error.code === 'output_limit' && error.message.includes('longer than ' + POLICY.maxOutputTokens + ' tokens') &&
    !error.message.includes('stops mid'));
  // The limit can be reached before any visible text, leaving no output at all.
  const empty = createGenerator({ apiKey: KEY, transport: async () => ({ status: 200, json: { ...cutOff, output: [] } }) });
  await assert.rejects(empty({ text: 'Fiction.' }), { code: 'output_limit' });
  for (const json of [
    { ...cutOff, incomplete_details: { reason: 'content_filter' } },
    { ...cutOff, incomplete_details: undefined },
    { ...cutOff, model: 'other-model' },
  ]) {
    const other = createGenerator({ apiKey: KEY, transport: async () => ({ status: 200, json }) });
    await assert.rejects(other({ text: 'Fiction.' }), { code: 'invalid_response' });
  }
});
test('HTTPS transport fixes destination and does not follow a redirect', async t => {
  let options;
  let calls = 0;
  t.mock.method(https, 'request', (args, callback) => {
    options = args; calls++;
    const request = new EventEmitter();
    request.end = () => {
      const response = Readable.from([Buffer.from('FAKE_ERROR_BODY')]);
      response.statusCode = 307;
      response.headers = { location: 'https://unallowed.invalid/', 'content-type': 'application/json' };
      callback(response);
    };
    return request;
  });
  const result = await sendToOpenAI({ apiKey: KEY, body: makeProviderBody('Fiction.') });
  assert.deepEqual(result, { status: 307 });
  assert.equal(calls, 1);
  assert.equal(options.protocol, 'https:');
  assert.equal(options.hostname, 'api.openai.com');
  assert.equal(options.port, 443);
  assert.equal(options.path, '/v1/responses');
  assert.equal(options.method, 'POST');
  assert.equal(options.rejectUnauthorized, true);
  // The destination comes from policy.json, so its pinned SHA-256 covers it.
  assert.deepEqual([options.protocol, options.hostname, options.port, options.method, options.path],
    [POLICY.providerProtocol, POLICY.providerHost, POLICY.providerPort, POLICY.providerMethod, POLICY.providerPath]);
});
test('the broker refuses to load a policy that names another destination', async t => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'enote-policy-'));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  await fs.mkdir(path.join(directory, 'src'));
  await fs.copyFile(new URL('../src/generate.mjs', import.meta.url), path.join(directory, 'src', 'generate.mjs'));
  const load = async changes => {
    await fs.writeFile(path.join(directory, 'policy.json'), JSON.stringify({ ...POLICY, ...changes }));
    return spawnSync(process.execPath, [path.join(directory, 'src', 'generate.mjs')], { encoding: 'utf8', timeout: 10_000 });
  };
  assert.equal((await load({})).status, 0);
  for (const changes of [{ providerHost: 'unallowed.invalid' }, { providerPort: 8443 }, { providerProtocol: 'http:' },
    { providerMethod: 'GET' }, { providerPath: '/v1/files' }, { providerPath: undefined }]) {
    const result = await load(changes);
    assert.notEqual(result.status, 0, JSON.stringify(changes));
    assert.match(result.stderr, /The companion policy names an unapproved provider destination\./);
  }
});
test('configuration accepts only the separate Agent Key and distinct HTTP authorization', () => {
  assert.throws(() => readRuntimeConfig({ OPENAI_API_KEY: KEY }));
  assert.throws(() => readRuntimeConfig({ OPENAI_AGENT_API_KEY: KEY + '\r\n' }));
  assert.throws(() => readRuntimeConfig({ OPENAI_AGENT_API_KEY: KEY,
    AGENT_COMPANION_BEARER_TOKEN: KEY }, { requireHttpToken: true }));
  const token = 'FAKE_HTTP_TOKEN_FOR_TESTS_ONLY_1234567890';
  const config = readRuntimeConfig({ OPENAI_AGENT_API_KEY: KEY, AGENT_COMPANION_BEARER_TOKEN: token },
    { requireHttpToken: true });
  assert.equal(config.apiKey, KEY);
  assert.equal(config.bearerToken, token);
});
