import https from 'node:https';
import { performance } from 'node:perf_hooks';
import policy from '../policy.json' with { type: 'json' };

export const POLICY = Object.freeze(policy);
const PUBLIC_ERRORS = Object.freeze({
  invalid_input: 'Provide one nonempty text string of at most 4096 UTF-8 bytes.',
  busy: 'The companion is processing another request.',
  usage_limit: 'The companion request limit has been reached.',
  timeout: 'The provider request timed out. It was not retried and may still be billed.',
  cancelled: 'The request was cancelled and was not retried.',
  provider_auth: 'The separate Agent Key was rejected.',
  provider_limit: 'The provider rejected the request because of a limit.',
  provider_unavailable: 'The provider request could not be completed.',
  invalid_response: 'The provider returned an unsupported response.',
  grant_expired: 'The approved agent grant has ended. Ask the owner for a new grant.',
});
export class CompanionError extends Error {
  constructor(code) {
    super(PUBLIC_ERRORS[code] ?? PUBLIC_ERRORS.provider_unavailable);
    this.code = Object.hasOwn(PUBLIC_ERRORS, code) ? code : 'provider_unavailable';
  }
}
// Checks the owner's grant before validation, admission or any provider work.
export function guardGenerate(generate, isActive) {
  return async (args, options) => {
    if (!isActive()) throw new CompanionError('grant_expired');
    return generate(args, options);
  };
}
export function validateInput(args) {
  if (!args || Object.getPrototypeOf(args) !== Object.prototype ||
      Object.keys(args).length !== 1 || !Object.hasOwn(args, 'text') ||
      typeof args.text !== 'string' || !args.text.isWellFormed() ||
      !args.text.trim() || Buffer.byteLength(args.text, 'utf8') > POLICY.maxInputBytes) {
    throw new CompanionError('invalid_input');
  }
  return args.text;
}
export function makeProviderBody(text) {
  return {
    model: POLICY.model,
    input: [{ role: 'user', content: [{ type: 'input_text', text }] }],
    max_output_tokens: POLICY.maxOutputTokens,
    store: false, stream: false, background: false,
    tools: [], tool_choice: 'none',
    reasoning: { effort: 'none' },
    service_tier: 'default',
  };
}
export function sendToOpenAI({ apiKey, body, signal }) {
  return new Promise((resolve, reject) => {
    const bytes = Buffer.from(JSON.stringify(body), 'utf8');
    const request = https.request({
      protocol: 'https:', hostname: 'api.openai.com', port: 443,
      method: 'POST', path: '/v1/responses',
      agent: false, rejectUnauthorized: true, signal,
      headers: {
        authorization: 'Bearer ' + apiKey,
        'content-type': 'application/json',
        'content-length': bytes.length,
      },
    }, response => {
      const chunks = [];
      let size = 0;
      response.on('data', chunk => {
        size += chunk.length;
        if (size > POLICY.maxProviderResponseBytes) {
          response.destroy(new CompanionError('invalid_response'));
          return;
        }
        chunks.push(chunk);
      });
      response.once('error', () => reject(new CompanionError('provider_unavailable')));
      response.once('end', () => {
        const status = response.statusCode ?? 0;
        // Never parse or return a provider error body, redirect or header.
        if (status !== 200) { resolve({ status }); return; }
        if (!/^application\/json(?:;|$)/i.test(response.headers['content-type'] ?? '')) {
          reject(new CompanionError('invalid_response')); return;
        }
        try { resolve({ status, json: JSON.parse(Buffer.concat(chunks).toString('utf8')) }); }
        catch { reject(new CompanionError('invalid_response')); }
      });
    });
    request.once('error', () => reject(new CompanionError('provider_unavailable')));
    request.end(bytes);
  });
}
function extractText(result, apiKey) {
  if (result?.status !== 200) {
    if (result?.status === 401 || result?.status === 403) throw new CompanionError('provider_auth');
    if (result?.status === 429) throw new CompanionError('provider_limit');
    throw new CompanionError('provider_unavailable');
  }
  const json = result.json;
  if (json?.model !== POLICY.model || json?.status !== 'completed' ||
      !Array.isArray(json.output) || !json.output.length || json.output.length > 8) {
    throw new CompanionError('invalid_response');
  }
  const texts = [];
  for (const item of json.output) {
    if (item?.type !== 'message' || item.role !== 'assistant' ||
        !Array.isArray(item.content) || !item.content.length || item.content.length > 8) {
      throw new CompanionError('invalid_response');
    }
    for (const content of item.content) {
      if (content?.type !== 'output_text' || typeof content.text !== 'string' ||
          !content.text.isWellFormed()) throw new CompanionError('invalid_response');
      texts.push(content.text);
    }
  }
  const text = texts.join('\n');
  if (!text.length || Buffer.byteLength(text, 'utf8') > POLICY.maxOutputBytes ||
      text.includes(apiKey)) throw new CompanionError('invalid_response');
  return { text };
}
export function createGenerator({ apiKey, transport = sendToOpenAI,
                                  now = () => performance.now(),
                                  timeoutMs = POLICY.upstreamTimeoutMs }) {
  if (typeof apiKey !== 'string' || apiKey.length < 16 || apiKey.length > 512 ||
      /[\s\x00-\x1f\x7f]/.test(apiKey)) throw new Error('Invalid Agent Key configuration.');
  let active = false;
  let admitted = 0;
  let recent = [];
  return async function generate(args, { signal } = {}) {
    const text = validateInput(args);
    if (signal?.aborted) throw new CompanionError('cancelled');
    if (active) throw new CompanionError('busy');
    const instant = now();
    recent = recent.filter(time => instant - time < 3_600_000);
    if (admitted >= POLICY.maxRequestsPerProcess ||
        recent.length >= POLICY.maxRequestsPerHour ||
        recent.filter(time => instant - time < 60_000).length >= POLICY.maxRequestsPerMinute) {
      throw new CompanionError('usage_limit');
    }
    // Admission consumes local budget even if the provider rejects, times out or disconnects.
    admitted += 1;
    recent.push(instant);
    active = true;
    const controller = new AbortController();
    let timer;
    let cancel;
    const deadline = new Promise((_, reject) => {
      timer = setTimeout(() => {
        reject(new CompanionError('timeout'));
        controller.abort();
      }, timeoutMs);
      cancel = () => {
        reject(new CompanionError('cancelled'));
        controller.abort();
      };
      signal?.addEventListener('abort', cancel, { once: true });
      if (signal?.aborted) cancel();
    });
    try {
      const result = await Promise.race([
        Promise.resolve().then(() => transport({
          apiKey, body: makeProviderBody(text), signal: controller.signal,
        })).catch(() => { throw new CompanionError('provider_unavailable'); }),
        deadline,
      ]);
      return extractText(result, apiKey);
    } finally {
      clearTimeout(timer);
      signal?.removeEventListener('abort', cancel);
      active = false;
    }
  };
}
