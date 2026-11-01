# Private ChatGPT connection (optional)

Status: PREPARED ONLY. No tunnel, ChatGPT plugin, workspace setting, sign-in or live call has been made. A connection counts as verified only after the observed checks at the end of this page.

ChatGPT runs in OpenAI's cloud and cannot reach this computer's loopback port or stdio directly. The private route is OpenAI's [Secure MCP Tunnel](https://developers.openai.com/api/docs/guides/secure-mcp-tunnels): a local tunnel client connects outbound over HTTPS to OpenAI, collects MCP requests and returns the responses, without a public listener. The tunnel client must start the same **agent bridge** that local agents use, so every call still meets the owner broker's grant, token, limits and fixed destination.

## Before you connect

1. The local route works first: the owner broker (standalone launcher, or encrypted-note's Agent Companion helper) runs with a verified separate Standard agent account, and a local agent has made one fictional call through `isolation/agent-bridge.mjs`.
2. Choose a ChatGPT account or workspace for this connection that has no other connectors to your own data. Tool results go to the model and can be combined with anything else that conversation can reach.
3. Keep identities apart. The tunnel's runtime identity, the OpenAI inference Agent Key and any administrator credential are three different things. Never give the tunnel identity the inference key, and never give the inference key tunnel administration. Follow the official tunnel guide for the exact permissions and organization/workspace associations.

## Connect

1. In the **agent account**, install the tunnel client as the official guide describes and configure it to launch the stdio bridge with only the reviewed plan's path:

       command: /ABSOLUTE/PATH/TO/TRUSTED/node
       args:    /ABSOLUTE/PATH/TO/COMPANION/isolation/agent-bridge.mjs /ABSOLUTE/PATH/TO/REVIEWED/agent-plan.json

   No provider key, token value or credential path belongs in this configuration.
2. In ChatGPT, add the tunnel as a custom MCP server (Plugins → **+** → Add custom MCP server), following [connect ChatGPT](https://developers.openai.com/plugins/deploy/connect-chatgpt).
3. Confirm discovery lists exactly one tool, `generate_text`. The bridge lists it even while the helper is off.

## Verify before relying on it

Record each observation privately (never real keys, tokens, account names or paths in GitHub):

| Check | Expected observation |
| --- | --- |
| End-to-end call | A fictional text request returns bounded text. |
| Extra arguments | Requests adding a model, URL, key or other field are refused by the server. |
| A prompt asking to reveal a key | Is just text generation; no key or token appears anywhere. |
| Owner Stop or Vault lock (integrated helper) | The next call fails with a fixed error telling the agent to ask the owner to choose Start. It doesn't say why the helper stopped. |
| New Start | The next call works through the same bridge, with no restart of the tunnel client. |
| Grant expiry | Calls fail with a fixed error telling the agent to ask the owner to choose Start. |
| Revoked Agent Key | After revoking the key at OpenAI, a call fails without leaking provider details. |
| Disconnection | Removing the custom server in ChatGPT stops further calls. |

Until every row has been observed, report the ChatGPT connection as not verified. encrypted-note always shows it as "Not checked" because the app cannot observe this cloud route.
