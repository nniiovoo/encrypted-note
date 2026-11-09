// ADR-0001 source hygiene, run by check.sh with the already-required Node 24+.
// Scope: index.html and fixture.html; JS/TS/HTML in src/ and isolation/; Rust in src-tauri/src/
// and crates/*/src/ (including inline tests); Swift, Objective-C, C and C++ in src-tauri/native/
// recursively. Dependencies, vendor/, src-tauri/gen/, build.rs files,
// configuration, docs, scripts and crates/*/tests/ are outside this check.
// src-tauri/companion/ is also outside: it holds the separately sandboxed Agent Companion
// helper's pins, entitlements and fictional verification payloads (ADR-0009), not Vault code.
// The helper's own Swift (src-tauri/native/companion/) is scanned like all native code: it
// supervises a process and has no network API of its own.
// companion/ is outside on purpose (ADR-0010): it is the broker that makes the one approved
// OpenAI request, and it runs only inside the helper's sandbox. In its place, a second check here
// fails if any file in src/, src-tauri/src/ or crates/ imports from it, so no Vault code reaches
// that network code. That check also needs a literal path, so a computed one gets past it.
// A third check keeps the broker narrow: in companion/src/ and companion/isolation/, only
// src/generate.mjs may import node:https (the one provider request) and only src/http.mjs
// node:http (the loopback server); no other Node networking module or browser network API may
// appear. companion/policy.json must match the SHA-256 pinned in companion.lock.json and name
// exactly the approved destination, which generate.mjs also refuses to load without.
// A fourth check keeps the agents' local route (ADR-0012) in its two places: Unix domain socket
// types, std::os::unix::net, getpeereid/peer credentials and libc bind/listen/accept may appear
// only in src-tauri/src/agent_route.rs (the app's listener) and crates/agent-keys-mcp/src/ (the
// agent's tool). Those files and src-tauri/src/core/agent.rs must not name `companion::` (the
// route shares nothing with the Agent Companion), and the tool's Cargo.toml may depend on no
// Vault crate.
// Patterns: browser fetch/sendBeacon calls (also fetch?.()), XMLHttpRequest, WebSocket,
// EventSource, WebTransport and RTCPeerConnection names; Rust std::net/tokio::net, socket types,
// libc socket/connect/getaddrinfo and NSURLSession/NSURLConnection names.
// Native URLSession/NSURLSession/URLRequest/NSURLConnection, Network NWConnection/NWListener and
// nw_connection/nw_listener names, importing or including Network or CFNetwork, socket() calls,
// getaddrinfo, CFStream/CFReadStream/CFWriteStream, and
// URLs built from a string (URL(string:), NSURL(string:), URLComponents(string:),
// URLWithString:, initWithString:); file URLs are allowed.
// Approved IPC stays distinct: src/api.ts invoke/listen and the isolation hook use
// Tauri's SDK. Its ipc:/http://ipc.localhost transport and tauri.localhost app URLs
// are not network API calls. URL literals (including navigation tests) are allowed.
// Matches in comments/strings also fail. No file/line suppressions: a harmless match
// needs a narrower rule and regression case.
// shortcut: literal text matching misses aliases, computed names, FFI and other egress
// paths; extend rules when reviewing a new entry point. This is no runtime egress
// sandbox and cannot contain compromised native code. CSP and cargo-deny are separate.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { join, posix } from "node:path";
import { fileURLToPath } from "node:url";

const browserApi = /\b(?:fetch|sendBeacon)(?=\s*(?:\?\.\s*)?\()|\b(?:XMLHttpRequest|WebSocket|EventSource|WebTransport|(?:webkit|moz)?RTCPeerConnection)\b/g;
const rustApi = /\b(?:std|tokio)\s*::\s*net\b|\b(?:TcpStream|TcpListener|UdpSocket)\b|\blibc\s*::\s*(?:socket|connect|getaddrinfo)\b|\blibc\s*::\s*\{[^;]*\b(?:socket|connect|getaddrinfo)\b|\blibc\s*::\s*\*|\bNSURL(?:Session|Connection)\b/g;
const nativeApi = /\b(?:URLSession|NSURLSession|URLRequest|NSURLConnection|NWConnection|NWListener|getaddrinfo)\b|\b(?:import|include)\s*[<"]?\s*(?:Network|CFNetwork)\b|\bimport\s+\w+\s+(?:Network|CFNetwork)\b|\bsocket\s*\(|\bCF(?:Read|Write)?Stream\w*|\bnw_(?:connection|listener)\w*|\b(?:NS)?URL(?:Components)?(?:\s*\.\s*init)?\s*\(\s*string\s*:|\bURLWithString\s*:|\[\s*NSURL\s+alloc\s*\]\s*initWithString\s*:/g;
const localSocketApi = /\b(?:UnixListener|UnixStream|UnixDatagram|getpeereid|LOCAL_PEERCRED|SO_PEERCRED|peer_cred)\b|\bstd\s*::\s*os\s*::\s*unix\s*::\s*net\b|\blibc\s*::\s*(?:bind|listen|accept)\b/g;
const localSocketPlace = /^(?:src-tauri\/src\/agent_route\.rs|crates\/agent-keys-mcp\/src\/.*\.rs)$/;
const keyRoutePlace = /^(?:src-tauri\/src\/agent_route\.rs|src-tauri\/src\/core\/agent\.rs|crates\/agent-keys-mcp\/src\/.*\.rs)$/;
const companionPathRef = /\bcompanion\s*::/g;
const vaultCrates = /^\s*(?:vault-format|notes|session|policy|guard|store|enote-reader|encrypted-note-app)\b/m;
const rootPage = /^(?:index|fixture)\.html$/;
const browserSource = /^(?:src|isolation)\/.*\.(?:[cm]?[jt]s|jsx|tsx|html)$/;
const rustSource = /^(?:src-tauri\/src|crates\/[^/]+\/src)\/.*\.rs$/;
const nativeSource = /^src-tauri\/native\/.*\.(?:swift|m|mm|c|cc|cpp|cxx|c\+\+|h|hh|hpp|hxx|inl)$/;
const vaultSource = /^(?:src\/.*\.(?:[cm]?[jt]s|jsx|tsx|html|css)|(?:src-tauri\/src|crates)\/.*\.rs)$/;
const importForm = /(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\s*\(\s*|\bnew\s+URL\s*\(\s*|\binclude(?:_str|_bytes)?!\s*\(\s*|#\[\s*path\s*=\s*)["'`]([^"'`\n]+)["'`]/g;
const companionPath = /^\/?(?:companion|encrypted-note-agent-companion)(?:\/|$)/;
const companionRuntime = /^companion\/(?:src|isolation)\/.*\.(?:[cm]?js)$/;
const nodeNetwork = /^(?:node:)?(?:https?|http2|net|tls|dgram|dns|undici)(?:\/|$)/;
const allowedNetwork = { "companion/src/generate.mjs": "node:https", "companion/src/http.mjs": "node:http" };
// ADR-0009: the one provider request. Only this destination may be named in companion/policy.json.
const approvedDestination = {
  providerProtocol: "https:", providerHost: "api.openai.com", providerPort: 443,
  providerMethod: "POST", providerPath: "/v1/responses",
};

// Resolved against the importing file, so src-tauri/companion/ (the helper's pins, not the broker)
// and a WebView file such as src/companion.ts are not confused with the top-level companion/.
function companionImports(path, source) {
  if (!vaultSource.test(path)) return [];
  return [...source.matchAll(importForm)].flatMap((match) => {
    const target = match[1].startsWith(".") ? posix.join(posix.dirname(path), match[1]) : match[1];
    if (!companionPath.test(target)) return [];
    const line = source.slice(0, match.index).split("\n").length;
    return [`${path}:${line}: Vault code imports the Agent Companion (${match[1]})`];
  });
}

function companionNetwork(path, source) {
  if (!companionRuntime.test(path)) return [];
  const found = [...source.matchAll(importForm)].filter((match) => nodeNetwork.test(match[1]) && match[1] !== allowedNetwork[path]);
  return [...found, ...source.matchAll(browserApi)].map((match) => {
    const line = source.slice(0, match.index).split("\n").length;
    return `${path}:${line}: networking outside the Agent Companion's one request and loopback server (${match[1] ?? match[0]})`;
  });
}

function policyProblems(policyText, pinnedSha256) {
  const problems = [];
  if (createHash("sha256").update(policyText).digest("hex") !== pinnedSha256) {
    problems.push("companion/policy.json differs from policySha256 in src-tauri/companion/companion.lock.json");
  }
  const policy = JSON.parse(policyText);
  for (const [key, value] of Object.entries(approvedDestination)) {
    if (policy[key] !== value) problems.push(`companion/policy.json: ${key} is not the approved destination`);
  }
  return problems;
}

// ADR-0012: local sockets only where the agents' route lives.
function localSockets(path, source) {
  if (!rustSource.test(path) || localSocketPlace.test(path)) return [];
  return [...source.matchAll(localSocketApi)].map((match) => {
    const line = source.slice(0, match.index).split("\n").length;
    return `${path}:${line}: local socket outside the agents' route (${match[0].replace(/\s+/g, " ")})`;
  });
}

// ADR-0012: the agents' route shares nothing with the Agent Companion.
function keyRouteCompanion(path, source) {
  if (!keyRoutePlace.test(path)) return [];
  return [...source.matchAll(companionPathRef)].map((match) => {
    const line = source.slice(0, match.index).split("\n").length;
    return `${path}:${line}: the agents' route uses the Agent Companion (${match[0]})`;
  });
}

// The agent's tool knows the route's protocol and nothing of the Vault.
function agentToolManifest(manifest) {
  return vaultCrates.test(manifest) ? ["crates/agent-keys-mcp/Cargo.toml: the agent's tool depends on a Vault crate"] : [];
}

function patternFor(path) {
  if (rootPage.test(path) || browserSource.test(path)) return browserApi;
  if (rustSource.test(path)) return rustApi;
  if (nativeSource.test(path)) return nativeApi;
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
let bytes = try Data(contentsOf: URL(fileURLWithPath: "/obviously-fake-vault"))
fakeBuffer.append(contentsOf: bytes)
`), []);
assert.deepEqual(matches("crates/reader/src/fake.rs", "let uid = unsafe { libc::getuid() };"), []);
assert.deepEqual(matches("crates/reader/src/fake.rs", "use libc::{c_int, getuid};"), []);
for (const api of ["fetch", "sendBeacon"]) {
  assert.equal(matches("src/fake.ts", `globalThis.${api} ("https://example.invalid/fake");`).length, 1);
}
for (const api of ["fetch", "sendBeacon"]) {
  assert.equal(matches("src/fake.ts", `globalThis.${api}?.("https://example.invalid/fake");`).length, 1);
  assert.equal(matches("src/fake.ts", `${api} ?.  ("https://example.invalid/fake");`).length, 1);
}
for (const path of ["src/fake.cts", "src/fake.mts", "isolation/nested/fake.mts"]) {
  assert.equal(matches(path, 'fetch("https://example.invalid/fake");').length, 1, path);
}
for (const api of ["XMLHttpRequest", "WebSocket", "EventSource", "WebTransport", "RTCPeerConnection", "webkitRTCPeerConnection", "mozRTCPeerConnection"]) {
  assert.equal(matches("isolation/fake.js", `new ${api}("https://example.invalid/fake");`).length, 1);
}
for (const api of ["std :: net", "tokio::net", "TcpStream", "TcpListener", "UdpSocket"]) {
  assert.equal(matches("crates/reader/src/fake.rs", `use ${api};`).length, 1);
}
for (const source of [
  "libc::socket(fake_domain, fake_type, 0)",
  "libc :: connect(fake_fd, fake_address, fake_length)",
  "libc::getaddrinfo(fake_host, fake_port, fake_hints, fake_result)",
  "use libc::{c_int, socket};",
  "use libc :: {\n  connect,\n};",
  "use libc::*;",
  "use libc::{c_int, fake::{socket}};",
  "let session = msg_send![class!(NSURLSession), sharedSession];",
  "let connection = msg_send![class!(NSURLConnection), alloc];",
]) {
  assert.equal(matches("src-tauri/src/fake.rs", source).length, 1, source);
}
for (const source of [
  'let task = URLSession.shared.dataTask(with: fakeURL)',
  'let request = URLRequest(url: fakeURL)',
  'NSURLConnection.sendAsynchronousRequest(fakeRequest, queue: fakeQueue)',
  'let connection = Network.NWConnection(host: "example.invalid", port: 443, using: .tcp)',
  'let listener = try NWListener(using: .tcp, on: 443)',
  'let session = NSURLSession.sharedSession()',
  'import Network',
  'import CFNetwork',
  'let descriptor = socket(AF_INET, SOCK_STREAM, 0)',
  'let found = getaddrinfo("example.invalid", nil, nil, &fakeResult)',
  'CFStreamCreatePairWithSocketToHost(nil, fakeHost, 443, &fakeRead, &fakeWrite)',
  'let connection = nw_connection_create(fakeEndpoint, fakeParameters)',
  'let page = try Data(contentsOf: URL(string: "https://example.invalid/fake")!)',
  'let page = URL (string: "https://example.invalid/fake")',
  'let page = URL.init(string: "https://example.invalid/fake")',
  'let page = NSURL(string: "https://example.invalid/fake")',
  'let parts = URLComponents(string: "https://example.invalid/fake")',
  'import class Network.NWEndpoint',
  'let listener = nw_listener_create(fakeParameters)',
]) {
  assert.equal(matches("src-tauri/native/nested/fake.swift", source).length, 1, source);
}
for (const [path, source] of [
  ["src-tauri/native/fake.m", "#import <Network/Network.h>"],
  ["src-tauri/native/fake.m", "#import <CFNetwork/CFNetwork.h>"],
  ["src-tauri/native/fake.m", "#include <Network/Network.h>"],
  ["src-tauri/native/fake.h", "#include <CFNetwork/CFNetwork.h>"],
  ["src-tauri/native/fake.cc", "int descriptor = socket(AF_INET, SOCK_STREAM, 0);"],
  ["src-tauri/native/fake.cpp", "int descriptor = socket(AF_INET, SOCK_STREAM, 0);"],
  ["src-tauri/native/fake.swift", "let stream = CFReadStreamCreateForHTTPRequest(nil, fakeRequest)"],
  ["src-tauri/native/fake.swift", "CFWriteStreamOpen(fakeStream)"],
  ["src-tauri/native/fake.m", "NSURLSession *session = fakeSession;"],
  ["src-tauri/native/fake.mm", 'NSURL *page = [NSURL URLWithString:@"https://example.invalid/fake"];'],
  ["src-tauri/native/fake.c", "int descriptor = socket(AF_INET, SOCK_STREAM, 0);"],
  ["src-tauri/native/fake.m", "#include<Network/Network.h>"],
  ["src-tauri/native/fake.m", '#import "CFNetwork/CFNetwork.h"'],
  ["src-tauri/native/fake.mm", 'NSURL *page = [[NSURL alloc] initWithString:@"https://example.invalid/fake"];'],
  ["src-tauri/native/fake.cxx", "int descriptor = socket(AF_INET, SOCK_STREAM, 0);"],
  ["src-tauri/native/fake.hh", "#include <Network/Network.h>"],
  ["src-tauri/native/fake.hpp", "#include <Network/Network.h>"],
]) {
  assert.equal(matches(path, source).length, 1, source);
}
for (const source of ["import Foundation", "import CryptoKit", "#include <stdio.h>", 'let file = URL(fileURLWithPath: "/fake")']) {
  assert.deepEqual(matches("src-tauri/native/fake.swift", source), [], source);
}
assert.deepEqual(matches("src-tauri/native/fake.swift", '// fake source\nlet session = Foundation.URLSession.shared'), [
  "src-tauri/native/fake.swift:2: disallowed networking entry point (URLSession)",
]);
assert.equal(matches("src-tauri/native/fake.swift", '// URLSession is forbidden even in a comment').length, 1);
assert.deepEqual(matches("src/fake.tsx", '// fake source\nfetch("https://example.invalid/fake");'), [
  "src/fake.tsx:2: disallowed networking entry point (fetch)",
]);
for (const path of ["index.html", "fixture.html", "src/fake.cts", "src/fake.mts", "src-tauri/native/fake.m", "src-tauri/native/fake.mm", "src-tauri/native/fake.c", "src/nested/fake.tsx", "isolation/index.html", "src-tauri/src/fake.rs", "crates/notes/src/fake.rs", "src-tauri/native/fake.swift", "src-tauri/native/nested/fake.swift"]) {
  assert.ok(patternFor(path), path);
}
for (const path of ["src-tauri/build.rs", "crates/notes/build.rs", "docs/fake.md", "vendor/fake/src/lib.rs", "crates/notes/tests/fake.rs", "src-tauri/gen/fake.rs", "scripts/fake.mjs", "src-tauri/gen/fake.swift", "docs/fake.swift", "src-tauri/native/fake.swift.backup", "companion/src/fake.mjs", "companion/isolation/fake.mjs", "src-tauri/companion/verification/fake.mjs"]) {
  assert.deepEqual(matches(path, "fetch(); std::net::TcpStream; URLSession.shared; Network.NWConnection();"), []);
}
for (const [path, source] of [
  ["src/fake.ts", 'import { generate } from "../companion/src/generate.mjs";'],
  ["src/nested/fake.tsx", 'export { generate } from "../../companion/src/generate.mjs";'],
  ["src/fake.ts", 'const later = await import("../companion/src/generate.mjs");'],
  ["src/fake.ts", 'import policy from "../companion/policy.json?raw";'],
  ["src/fake.ts", 'import "../companion/src/generate.mjs";'],
  ["src/fake.js", 'const later = require("../companion/src/generate.mjs");'],
  ["src/fake.ts", 'const page = new URL("../companion/policy.json", import.meta.url);'],
  ["src/fake.ts", 'import { generate } from "companion/src/generate.mjs";'],
  ["src/fake.ts", 'import { generate } from "/companion/src/generate.mjs";'],
  ["src/fake.ts", 'import { generate } from "encrypted-note-agent-companion";'],
  ["src-tauri/src/fake.rs", 'const POLICY: &str = include_str!("../../companion/policy.json");'],
  ["src-tauri/src/nested/fake.rs", 'const POLICY: &[u8] = include_bytes!("../../../companion/policy.json");'],
  ["src-tauri/src/fake.rs", 'include!("../../companion/fake.rs");'],
  ["crates/notes/src/fake.rs", '#[path = "../../../companion/fake.rs"] mod fake;'],
  ["crates/notes/tests/fake.rs", 'const POLICY: &str = include_str!("../../../companion/policy.json");'],
]) {
  assert.equal(companionImports(path, source).length, 1, source);
}
for (const [path, source] of [
  ["src-tauri/src/fake.rs", 'const PINS: &str = include_str!("../companion/companion.lock.json");'],
  ["src-tauri/src/fake.rs", "use crate::companion::OPERATION;"],
  ["src/fake.ts", 'import { startCompanion } from "./companion";'],
  ["src/nested/fake.ts", 'import { startCompanion } from "../companion/start";'],
  ["src/fake.ts", 'const note = "see companion/README.md";'],
  ["companion/src/fake.mjs", 'import { generate } from "../../companion/src/generate.mjs";'],
  ["scripts/fake.mjs", 'import { generate } from "../companion/src/generate.mjs";'],
]) {
  assert.deepEqual(companionImports(path, source), [], source);
}

for (const [path, source] of [
  ["companion/src/fake.mjs", "import https from 'node:https';"],
  ["companion/src/generate.mjs", "import http from 'node:http';"],
  ["companion/src/http.mjs", "import https from \"https\";"],
  ["companion/isolation/fake.mjs", "const net = await import('node:net');"],
  ["companion/isolation/fake.mjs", "const tls = require('tls');"],
  ["companion/src/fake.mjs", "import { request } from 'node:http2';"],
  ["companion/src/fake.mjs", "import { fetch } from 'undici';"],
  ["companion/src/fake.mjs", "import dns from 'node:dns';"],
  ["companion/src/fake.mjs", "import { lookup } from 'node:dns/promises';"],
  ["companion/src/fake.mjs", "await fetch('https://example.invalid/fake');"],
  ["companion/isolation/fake.mjs", "new WebSocket('wss://example.invalid/fake');"],
]) {
  assert.equal(companionNetwork(path, source).length, 1, `${path}: ${source}`);
}
for (const [path, source] of [
  ["companion/src/generate.mjs", "import https from 'node:https';"],
  ["companion/src/http.mjs", "import http from 'node:http';"],
  ["companion/isolation/fake.mjs", "import { pathToFileURL } from 'node:url';"],
  ["companion/isolation/fake.mjs", "import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';"],
  ["companion/test/fake.test.mjs", "import http from 'node:http'; await fetch('http://127.0.0.1:1/fake');"],
]) {
  assert.deepEqual(companionNetwork(path, source), [], `${path}: ${source}`);
}
for (const [path, source] of [
  ["src-tauri/src/agent_route.rs", "use std::os::unix::net::{UnixListener, UnixStream};"],
  ["src-tauri/src/agent_route.rs", "let rc = unsafe { libc::getpeereid(fake_fd, &mut fake_uid, &mut fake_gid) };"],
  ["crates/agent-keys-mcp/src/lib.rs", "let stream = UnixStream::connect(fake_path);"],
  ["crates/agent-keys-mcp/src/peer.rs", "libc::getpeereid(fake_fd, &mut fake_uid, &mut fake_gid)"],
  ["crates/agent-keys-mcp/tests/fake.rs", "let fake = UnixListener::bind(fake_path);"],
  ["src-tauri/tests/fake.rs", "let fake = UnixStream::connect(fake_path);"],
  ["src-tauri/src/fake.rs", "// A Unix socket is not mentioned by name here."],
]) {
  assert.deepEqual(localSockets(path, source), [], `${path}: ${source}`);
}
for (const [path, source] of [
  ["src-tauri/src/commands.rs", "let fake = UnixListener::bind(fake_path);"],
  ["src-tauri/src/core/agent.rs", "let fake = UnixStream::connect(fake_path);"],
  ["src-tauri/src/fake.rs", "use std :: os :: unix :: net;"],
  ["crates/notes/src/fake.rs", "libc::getpeereid(fake_fd, &mut fake_uid, &mut fake_gid)"],
  ["crates/reader/src/fake.rs", "let fake = libc::bind(fake_fd, fake_address, fake_length);"],
  ["crates/reader/src/fake.rs", "let fake = libc :: accept(fake_fd, fake_address, fake_length);"],
  ["src-tauri/src/fake.rs", "let fake = fake_stream.peer_cred();"],
  ["src-tauri/src/fake.rs", "let fake = UnixDatagram::unbound();"],
  ["crates/agent-keys-mcp-copy/src/lib.rs", "let fake = UnixStream::connect(fake_path);"],
]) {
  assert.equal(localSockets(path, source).length, 1, `${path}: ${source}`);
}
for (const [path, source] of [
  ["src-tauri/src/agent_route.rs", "use crate::companion::Companion;"],
  ["src-tauri/src/agent_route.rs", "crate :: companion :: stop_on_exit(fake_app);"],
  ["src-tauri/src/core/agent.rs", "use super::super::companion::CompanionView;"],
  ["crates/agent-keys-mcp/src/lib.rs", "use encrypted_note_app::companion::Companion;"],
]) {
  assert.equal(keyRouteCompanion(path, source).length, 1, `${path}: ${source}`);
}
for (const [path, source] of [
  ["src-tauri/src/agent_route.rs", "// It shares nothing with the Agent Companion."],
  ["src-tauri/src/lib.rs", "companion::stop_on_exit(app);"],
  ["src-tauri/src/commands.rs", "use crate::companion::{Companion, CompanionView};"],
]) {
  assert.deepEqual(keyRouteCompanion(path, source), [], `${path}: ${source}`);
}
assert.deepEqual(agentToolManifest('[dependencies]\nserde_json.workspace = true\nlibc = "=0.2.189"\n'), []);
for (const crate of ["notes", "vault-format", "store", "session", "encrypted-note-app"]) {
  assert.equal(agentToolManifest(`[dependencies]\nserde_json.workspace = true\n${crate}.workspace = true\n`).length, 1, crate);
}
const fakePolicy = JSON.stringify({ model: "fake-model", ...approvedDestination });
const fakePin = createHash("sha256").update(fakePolicy).digest("hex");
assert.deepEqual(policyProblems(fakePolicy, fakePin), []);
assert.equal(policyProblems(fakePolicy, "0".repeat(64)).length, 1);
for (const change of [{ providerHost: "unallowed.invalid" }, { providerPort: 8443 }, { providerMethod: "GET" }, { providerPath: undefined }]) {
  const changed = JSON.stringify({ ...JSON.parse(fakePolicy), ...change });
  assert.deepEqual(policyProblems(changed, createHash("sha256").update(changed).digest("hex")).length, 1, JSON.stringify(change));
}

const root = fileURLToPath(new URL("../", import.meta.url));
const paths = ["index.html", "fixture.html"];
for (const directory of ["src", "isolation", "src-tauri/src", "src-tauri/native", "crates"]) {
  for (const relative of readdirSync(join(root, directory), { recursive: true })) {
    const path = `${directory}/${relative.replaceAll("\\", "/")}`;
    if (patternFor(path)) paths.push(path);
  }
}
const vaultPaths = [];
for (const directory of ["src", "src-tauri/src", "crates"]) {
  for (const relative of readdirSync(join(root, directory), { recursive: true })) {
    const path = `${directory}/${relative.replaceAll("\\", "/")}`;
    if (vaultSource.test(path)) vaultPaths.push(path);
  }
}
const companionPaths = [];
for (const directory of ["companion/src", "companion/isolation"]) {
  for (const relative of readdirSync(join(root, directory), { recursive: true })) {
    const path = `${directory}/${relative.replaceAll("\\", "/")}`;
    if (companionRuntime.test(path)) companionPaths.push(path);
  }
}
const violations = paths.sort().flatMap((path) => matches(path, readFileSync(join(root, path), "utf8")));
const imports = vaultPaths.sort().flatMap((path) => companionImports(path, readFileSync(join(root, path), "utf8")));
const routeViolations = [
  ...paths.sort().flatMap((path) => localSockets(path, readFileSync(join(root, path), "utf8"))),
  ...vaultPaths.sort().flatMap((path) => keyRouteCompanion(path, readFileSync(join(root, path), "utf8"))),
  ...agentToolManifest(readFileSync(join(root, "crates/agent-keys-mcp/Cargo.toml"), "utf8")),
];
const companionViolations = [
  ...companionPaths.sort().flatMap((path) => companionNetwork(path, readFileSync(join(root, path), "utf8"))),
  ...policyProblems(readFileSync(join(root, "companion/policy.json"), "utf8"),
    JSON.parse(readFileSync(join(root, "src-tauri/companion/companion.lock.json"), "utf8")).companion.policySha256),
];
if (violations.length) {
  console.error(violations.join("\n"));
  console.error("Remove the entry point (ADR-0001). For a harmless match, narrow the rule with a fake regression case; do not exempt a file or line.");
  process.exitCode = 1;
} else {
  console.log(`Networking source hygiene passed (${paths.length} files; fake pass/fail self-check passed).`);
}
if (imports.length) {
  console.error(imports.join("\n"));
  console.error("Vault code must not import companion/ (ADR-0010). Do not exempt a file or line.");
  process.exitCode = 1;
} else {
  console.log(`No Vault source imports companion/ (${vaultPaths.length} files; fake pass/fail self-check passed).`);
}
if (routeViolations.length) {
  console.error(routeViolations.join("\n"));
  console.error("Local sockets belong only to the agents' route (ADR-0012), which shares nothing with the Agent Companion. Do not exempt a file or line.");
  process.exitCode = 1;
} else {
  console.log(`Agents' local route stays in its place (${paths.length} files; fake pass/fail self-check passed).`);
}
if (companionViolations.length) {
  console.error(companionViolations.join("\n"));
  console.error("The Agent Companion makes one fixed request and serves only loopback (ADR-0009). Do not widen it, and do not change the pin to match an unreviewed policy.");
  process.exitCode = 1;
} else {
  console.log(`Agent Companion networking and policy pin passed (${companionPaths.length} files; fake pass/fail self-check passed).`);
}
