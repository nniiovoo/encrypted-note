import { pathToFileURL } from 'node:url';
import { Transform } from 'node:stream';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { createGenerator, POLICY } from './generate.mjs';
import { createMcpServer } from './mcp.mjs';
import { readRuntimeConfig } from './config.mjs';

export function boundedInput(input) {
  let lineBytes = 0;
  const bounded = new Transform({
    transform(chunk, _encoding, callback) {
      for (const byte of chunk) {
        if (byte === 10) lineBytes = 0;
        else if (++lineBytes > POLICY.maxMcpMessageBytes) {
          callback(new Error('MCP message too large.')); return;
        }
      }
      callback(null, chunk);
    },
  });
  input.pipe(bounded);
  return bounded;
}
export async function startStdio({ generate, input = process.stdin, output = process.stdout }) {
  const server = createMcpServer(generate);
  const limited = boundedInput(input);
  const transport = new StdioServerTransport(limited, output);
  let closing = false;
  const shutdown = () => {
    if (closing) return;
    closing = true;
    input.unpipe(limited);
    input.pause();
    void server.close();
  };
  for (const event of ['error', 'end', 'close']) limited.once(event, shutdown);
  input.once('error', shutdown);
  input.once('close', shutdown);
  output.once('error', shutdown);
  output.once('close', shutdown);
  transport.onerror = () => {};
  await server.connect(transport);
  return server;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const { apiKey } = readRuntimeConfig(process.env);
    await startStdio({ generate: createGenerator({ apiKey }) });
  } catch {
    process.stderr.write('Companion could not start. Check the separate Agent Key configuration.\n');
    process.exitCode = 1;
  }
}
