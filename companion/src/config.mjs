export function readRuntimeConfig(env, { requireHttpToken = false } = {}) {
  const apiKey = env.OPENAI_AGENT_API_KEY;
  if (typeof apiKey !== 'string' || apiKey.length < 16 || apiKey.length > 512 ||
      /[\s\x00-\x1f\x7f]/.test(apiKey)) throw new Error('A separate Agent Key is required.');
  const bearerToken = env.AGENT_COMPANION_BEARER_TOKEN;
  if (requireHttpToken &&
      (typeof bearerToken !== 'string' || !/^[A-Za-z0-9_-]{32,256}$/.test(bearerToken) ||
       bearerToken === apiKey)) throw new Error('A separate HTTP authorization token is required.');
  return { apiKey, bearerToken };
}
