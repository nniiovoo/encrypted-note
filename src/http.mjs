import http from 'node:http';
import { timingSafeEqual } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import { StreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/streamableHttp.js';
import { createGenerator, POLICY } from './generate.mjs';
import { createMcpServer } from './mcp.mjs';
import { readRuntimeConfig } from './config.mjs';
import { CancelledNotificationSchema } from '@modelcontextprotocol/sdk/types.js';
import { CompanionError } from './generate.mjs';

function reply(response, status) {
  response.writeHead(status, {
    'content-type': 'application/json',
    'cache-control': 'no-store',
    ...(status === 401 ? { 'www-authenticate': 'Bearer realm="agent-text-companion"' } : {}),
  });
  response.end(JSON.stringify({ error: 'Request rejected.' }));
}
function authorized(header, token) {
  if (typeof header !== 'string' || !/^Bearer [A-Za-z0-9_-]{32,256}$/.test(header)) return false;
  const actual = Buffer.from(header.slice(7));
  const expected = Buffer.from(token);
  return actual.length === expected.length && timingSafeEqual(actual, expected);
}
async function readBody(request) {
  const chunks = [];
  let size = 0;
  const length = request.headers['content-length'];
  if (length && (!/^\d+$/.test(length) || Number(length) > POLICY.maxMcpMessageBytes)) {
    throw new Error('body');
  }
  for await (const chunk of request) {
    size += chunk.length;
    if (size > POLICY.maxMcpMessageBytes) throw new Error('body');
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}
export async function startHttpServer({ generate, bearerToken, port = 8789 }) {
  if (typeof bearerToken !== 'string' || !/^[A-Za-z0-9_-]{32,256}$/.test(bearerToken) ||
      !Number.isInteger(port) || port < 0 || port > 65535) {
    throw new Error('Invalid HTTP configuration.');
  }
  // Stateless HTTP transports still share cancellation state for the one scoped client.
  const activeCalls = new Map();
  const server = http.createServer(async (request, response) => {
    let mcp;
    let transport;
    try {
      const actualPort = server.address().port;
      const hosts = ['127.0.0.1:' + actualPort, 'localhost:' + actualPort];
      const origins = hosts.map(host => 'http://' + host);
      const hostCount = request.rawHeaders.filter((_, i) => i % 2 === 0)
        .filter(name => name.toLowerCase() === 'host').length;
      if (hostCount !== 1 || !hosts.includes(request.headers.host)) { reply(response, 403); return; }
      if (request.headers.origin !== undefined && !origins.includes(request.headers.origin)) {
        reply(response, 403); return;
      }
      if (request.url !== '/mcp') { reply(response, 404); return; }
      if (!authorized(request.headers.authorization, bearerToken)) { reply(response, 401); return; }
      if (request.method !== 'POST') { reply(response, 405); return; }
      if (!/^application\/json(?:;|$)/i.test(request.headers['content-type'] ?? '')) {
        reply(response, 415); return;
      }
      const body = await readBody(request);
      // A single bounded JSON-RPC message; batch calls are deliberately unsupported.
      if (!body || typeof body !== 'object' || Array.isArray(body)) { reply(response, 400); return; }
      if (body.method === 'notifications/cancelled') {
        const cancelled = CancelledNotificationSchema.safeParse(body);
        if (!cancelled.success || Object.hasOwn(body, 'id')) { reply(response, 400); return; }
        activeCalls.get(cancelled.data.params.requestId)?.abort();
        response.writeHead(202, { 'cache-control': 'no-store' }); response.end(); return;
      }
      const scopedGenerate = async (args, { signal } = {}) => {
        const id = body.id;
        if (activeCalls.has(id)) throw new CompanionError('busy');
        const controller = new AbortController();
        const cancel = () => controller.abort();
        signal?.addEventListener('abort', cancel, { once: true });
        if (signal?.aborted) cancel();
        activeCalls.set(id, controller);
        try { return await generate(args, { signal: controller.signal }); }
        finally {
          signal?.removeEventListener('abort', cancel);
          if (activeCalls.get(id) === controller) activeCalls.delete(id);
        }
      };
      mcp = createMcpServer(scopedGenerate);
      transport = new StreamableHTTPServerTransport({
        sessionIdGenerator: undefined, enableJsonResponse: true,
      });
      response.once('close', () => { void transport.close(); void mcp.close(); });
      await mcp.connect(transport);
      await transport.handleRequest(request, response, body);
    } catch {
      if (!response.headersSent && !response.destroyed) reply(response, 400);
      else response.end();
    }
  });
  server.requestTimeout = 5000;
  server.headersTimeout = 5000;
  server.keepAliveTimeout = 1000;
  server.maxHeadersCount = 32;
  server.maxConnections = 16;
  server.on('clientError', (_error, socket) => {
    socket.end('HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n');
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, '127.0.0.1', resolve);
  });
  return server;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const { apiKey, bearerToken } = readRuntimeConfig(process.env, { requireHttpToken: true });
    const server = await startHttpServer({ generate: createGenerator({ apiKey }), bearerToken });
    process.stderr.write('Companion HTTP transport is listening on loopback port 8789.\n');
    for (const event of ['SIGINT', 'SIGTERM']) process.once(event, () => server.close());
  } catch {
    process.stderr.write('Companion could not start. Check the separate Agent Key and HTTP authorization configuration.\n');
    process.exitCode = 1;
  }
}
