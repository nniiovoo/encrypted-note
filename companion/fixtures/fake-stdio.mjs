import { startStdio } from '../src/stdio.mjs';
import { createGenerator, POLICY } from '../src/generate.mjs';
await startStdio({ generate: createGenerator({ apiKey: 'FAKE_OPENAI_AGENT_KEY_FOR_TESTS_ONLY', transport: async () => ({ status: 200, json: { model: POLICY.model, status: 'completed', output: [{ type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'Fictional stdio output.' }] }] } }) }) });
