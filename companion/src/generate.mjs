import https from 'node:https';
import { performance } from 'node:perf_hooks';
import policy from '../policy.json' with { type: 'json' };

export const POLICY = Object.freeze(policy);
// The one approved destination (ADR-0009). policy.json, whose pinned SHA-256 the helper checks
// before every Start, must name exactly this, or the broker refuses to load.
const DESTINATION = Object.freeze({
  providerProtocol: 'https:', providerHost: 'api.openai.com', providerPort: 443,
  providerMethod: 'POST', providerPath: '/v1/responses',
});
for (const [key, value] of Object.entries(DESTINATION)) {
  if (POLICY[key] !== value) throw new Error('The companion policy names an unapproved provider destination.');
}
const PUBLIC_ERRORS = Object.freeze({
  invalid_input: 'Provide one nonempty text string of at most 4096 UTF-8 bytes.',
  busy: 'The Agent Companion is working on another request. Wait for it to finish, then try again.',
  usage_limit: 'The Agent Companion request limit has been reached. Wait a minute, then try once more. If it fails again, tell the owner.',
  timeout: 'The provider request timed out. It was not retried and may still be billed. Try once more at most. If it fails again, tell the owner.',
  cancelled: 'The request was cancelled and was not retried. Send it again only if it is still needed.',
  provider_auth: 'The separate Agent Key was rejected. Tell the owner. Do not retry.',
  provider_limit: 'The provider rejected the request because of a limit. Tell the owner. Do not retry.',
  provider_rejected: 'The provider did not accept the fixed request, and retrying will not change that. Tell the owner. Do not retry.',
  provider_unavailable: 'The provider request could not be completed. Try once more at most. If it fails again, tell the owner.',
  invalid_response: 'The provider returned an unsupported response. Try once more at most. If it fails again, tell the owner.',
  model_refusal: 'The model declined this text, so no answer was returned. Do not send the same text again. Change the request or tell the owner.',
  output_limit: 'The answer was longer than 512 tokens, so it was cut off and not returned. It may still be billed. Ask for a shorter answer or split the request. The same text will likely fail again.',
  grant_expired: "The owner's grant has ended. Ask the owner to choose Start for up to one hour in encrypted-note. Do not look for keys or tokens.",
  // Never says why the Agent Companion is off, so it carries nothing about the Vault being locked.
  helper_off: "The Agent Companion isn't running. Ask the owner to choose Start for up to one hour in encrypted-note. Do not look for keys or tokens.",
});
export class CompanionError extends Error {
  constructor(code) {
    super(PUBLIC_ERRORS[code] ?? PUBLIC_ERRORS.provider_unavailable);
    this.code = Object.hasOwn(PUBLIC_ERRORS, code) ? code : 'provider_unavailable';
  }
}
// Only an exact fixed text crosses the bridge; anything else becomes the generic error.
export function errorFromPublicText(text) {
  return new CompanionError(Object.keys(PUBLIC_ERRORS).find(code => PUBLIC_ERRORS[code] === text));
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
      protocol: POLICY.providerProtocol, hostname: POLICY.providerHost, port: POLICY.providerPort,
      method: POLICY.providerMethod, path: POLICY.providerPath,
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
    // A malformed request or an unknown model fails the same way every time. A 400 can also be
    // OpenAI refusing this particular text, which sending it again would not change either.
    if (result?.status === 400 || result?.status === 404) throw new CompanionError('provider_rejected');
    throw new CompanionError('provider_unavailable');
  }
  const json = result.json;
  // A cut-off answer is never returned in part; the agent is told to ask for less instead.
  if (json?.model === POLICY.model && json?.status === 'incomplete' &&
      json?.incomplete_details?.reason === 'max_output_tokens') throw new CompanionError('output_limit');
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
    // A refusal is reported by its fixed text; its wording never reaches the agent.
    if (item.content.some(content => content?.type === 'refusal')) throw new CompanionError('model_refusal');
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
