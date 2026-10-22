// ADR-0001 source hygiene, run by check.sh with the already-required Node 24+.
// Scope: index.html; JS/TS/HTML in src/ and isolation/; Rust in src-tauri/src/ and
// crates/*/src/ (including inline tests); Swift in src-tauri/native/ recursively.
// Dependencies, vendor/, src-tauri/gen/,
// configuration, docs, scripts and crates/*/tests/ are outside this check.
// Patterns: browser fetch/sendBeacon calls, XMLHttpRequest, WebSocket, EventSource,
// WebTransport and RTCPeerConnection names; Rust std::net/tokio::net and socket types.
// Swift URLSession/URLRequest/NSURLConnection and Network NWConnection/NWListener names.
// Approved IPC stays distinct: src/api.ts invoke/listen and the isolation hook use
// Tauri's SDK. Its ipc:/http://ipc.localhost transport and tauri.localhost app URLs
// are not network API calls. URL literals (including navigation tests) are allowed.
// Matches in comments/strings also fail. No file/line suppressions: a harmless match
// needs a narrower rule and regression case.
// shortcut: literal text matching misses aliases, computed names, FFI and other egress
// paths; extend rules when reviewing a new entry point. This is no runtime egress
// sandbox and cannot contain compromised native code. CSP and cargo-deny are separate.
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const browserApi = /\b(?:fetch|sendBeacon)(?=\s*\()|\b(?:XMLHttpRequest|WebSocket|EventSource|WebTransport|(?:webkit|moz)?RTCPeerConnection)\b/g;
const rustApi = /\b(?:std|tokio)\s*::\s*net\b|\b(?:TcpStream|TcpListener|UdpSocket)\b/g;
const swiftApi = /\b(?:URLSession|URLRequest|NSURLConnection|NWConnection|NWListener)\b/g;
const browserSource = /^(?:src|isolation)\/.*\.(?:[cm]?js|jsx|ts|tsx|html)$/;
const rustSource = /^(?:src-tauri\/src|crates\/[^/]+\/src)\/.*\.rs$/;
const swiftSource = /^src-tauri\/native\/.*\.swift$/;

function patternFor(path) {
  if (path === "index.html" || browserSource.test(path)) return browserApi;
  if (rustSource.test(path)) return rustApi;
  if (swiftSource.test(path)) return swiftApi;
  return null;
}

function matches(path, source) {
  const pattern = patternFor(path);
  if (!pattern) return [];
  return [...source.matchAll(pattern)].map((match) => {
    const line = source.slice(0, match.index).split("\n").length;
    return `${path}:${line}: disallowed networking entry point (${match[0].replace(/\s+/g, " ")})`;
  });
}

// Fake source only: never execute these snippets or make network requests.
assert.deepEqual(matches("src/api.ts", `
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
invoke("app_status"); listen("status-changed", () => {});
const transport = "ipc: http://ipc.localhost";
const ownPage = "http://tauri.localhost/";
`), []);
assert.deepEqual(matches("src/Home.tsx", "// For the answer to the fetch below, which arrives later."), []);
assert.deepEqual(matches("src/fake.ts", "fetch\n(\"https://example.invalid/fake\");").length, 1);
assert.deepEqual(matches("isolation/index.js", "window.__TAURI_ISOLATION_HOOK__ = payload => payload;"), []);
assert.deepEqual(matches("src-tauri/src/lib.rs", 'use std::sync::mpsc; let url = "https://example.invalid/fake";'), []);
assert.deepEqual(matches("src-tauri/native/fake.swift", `
import Foundation
import CryptoKit
let file = URL(fileURLWithPath: "/obviously-fake-vault")
let documentation = "https://example.invalid/fake"
struct URLSessionMetadata {}
let NWConnectionCount = 0
`), []);
for (const api of ["fetch", "sendBeacon"]) {
  assert.equal(matches("src/fake.ts", `globalThis.${api} ("https://example.invalid/fake");`).length, 1);
}
for (const api of ["XMLHttpRequest", "WebSocket", "EventSource", "WebTransport", "RTCPeerConnection", "webkitRTCPeerConnection", "mozRTCPeerConnection"]) {
  assert.equal(matches("isolation/fake.js", `new ${api}("https://example.invalid/fake");`).length, 1);
}
for (const api of ["std :: net", "tokio::net", "TcpStream", "TcpListener", "UdpSocket"]) {
  assert.equal(matches("crates/reader/src/fake.rs", `use ${api};`).length, 1);
}
for (const source of [
  'let task = URLSession.shared.dataTask(with: fakeURL)',
  'let request = URLRequest(url: fakeURL)',
  'NSURLConnection.sendAsynchronousRequest(fakeRequest, queue: fakeQueue)',
  'let connection = Network.NWConnection(host: "example.invalid", port: 443, using: .tcp)',
  'let listener = try NWListener(using: .tcp, on: 443)',
]) {
  assert.equal(matches("src-tauri/native/nested/fake.swift", source).length, 1);
}
assert.deepEqual(matches("src-tauri/native/fake.swift", '// fake source\nlet session = Foundation.URLSession.shared'), [
  "src-tauri/native/fake.swift:2: disallowed networking entry point (URLSession)",
]);
assert.equal(matches("src-tauri/native/fake.swift", '// URLSession is forbidden even in a comment').length, 1);
assert.deepEqual(matches("src/fake.tsx", '// fake source\nfetch("https://example.invalid/fake");'), [
  "src/fake.tsx:2: disallowed networking entry point (fetch)",
]);
for (const path of ["index.html", "src/nested/fake.tsx", "isolation/index.html", "src-tauri/src/fake.rs", "crates/notes/src/fake.rs", "src-tauri/native/fake.swift", "src-tauri/native/nested/fake.swift"]) {
  assert.ok(patternFor(path), path);
}
for (const path of ["docs/fake.md", "vendor/fake/src/lib.rs", "crates/notes/tests/fake.rs", "src-tauri/gen/fake.rs", "scripts/fake.mjs", "src-tauri/gen/fake.swift", "docs/fake.swift", "src-tauri/native/fake.swift.backup"]) {
  assert.deepEqual(matches(path, "fetch(); std::net::TcpStream; URLSession.shared; Network.NWConnection();"), []);
}

const root = fileURLToPath(new URL("../", import.meta.url));
const paths = ["index.html"];
for (const directory of ["src", "isolation", "src-tauri/src", "src-tauri/native", "crates"]) {
  for (const relative of readdirSync(join(root, directory), { recursive: true })) {
    const path = `${directory}/${relative.replaceAll("\\", "/")}`;
    if (patternFor(path)) paths.push(path);
  }
}
const violations = paths.sort().flatMap((path) => matches(path, readFileSync(join(root, path), "utf8")));
if (violations.length) {
  console.error(violations.join("\n"));
  console.error("Remove the entry point (ADR-0001). For a harmless match, narrow the rule with a fake regression case; do not exempt a file or line.");
  process.exitCode = 1;
} else {
  console.log(`Networking source hygiene passed (${paths.length} files; fake pass/fail self-check passed).`);
}
