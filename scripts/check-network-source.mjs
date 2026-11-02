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
import { readdirSync, readFileSync } from "node:fs";
import { join, posix } from "node:path";
import { fileURLToPath } from "node:url";

const browserApi = /\b(?:fetch|sendBeacon)(?=\s*(?:\?\.\s*)?\()|\b(?:XMLHttpRequest|WebSocket|EventSource|WebTransport|(?:webkit|moz)?RTCPeerConnection)\b/g;
const rustApi = /\b(?:std|tokio)\s*::\s*net\b|\b(?:TcpStream|TcpListener|UdpSocket)\b|\blibc\s*::\s*(?:socket|connect|getaddrinfo)\b|\blibc\s*::\s*\{[^;]*\b(?:socket|connect|getaddrinfo)\b|\blibc\s*::\s*\*|\bNSURL(?:Session|Connection)\b/g;
const nativeApi = /\b(?:URLSession|NSURLSession|URLRequest|NSURLConnection|NWConnection|NWListener|getaddrinfo)\b|\b(?:import|include)\s*[<"]?\s*(?:Network|CFNetwork)\b|\bimport\s+\w+\s+(?:Network|CFNetwork)\b|\bsocket\s*\(|\bCF(?:Read|Write)?Stream\w*|\bnw_(?:connection|listener)\w*|\b(?:NS)?URL(?:Components)?(?:\s*\.\s*init)?\s*\(\s*string\s*:|\bURLWithString\s*:|\[\s*NSURL\s+alloc\s*\]\s*initWithString\s*:/g;
const rootPage = /^(?:index|fixture)\.html$/;
const browserSource = /^(?:src|isolation)\/.*\.(?:[cm]?[jt]s|jsx|tsx|html)$/;
const rustSource = /^(?:src-tauri\/src|crates\/[^/]+\/src)\/.*\.rs$/;
const nativeSource = /^src-tauri\/native\/.*\.(?:swift|m|mm|c|cc|cpp|cxx|c\+\+|h|hh|hpp|hxx|inl)$/;
const vaultSource = /^(?:src\/.*\.(?:[cm]?[jt]s|jsx|tsx|html|css)|(?:src-tauri\/src|crates)\/.*\.rs)$/;
const importForm = /(?:\bfrom\s*|\bimport\s*\(?\s*|\brequire\s*\(\s*|\bnew\s+URL\s*\(\s*|\binclude(?:_str|_bytes)?!\s*\(\s*|#\[\s*path\s*=\s*)["'`]([^"'`\n]+)["'`]/g;
const companionPath = /^\/?(?:companion|encrypted-note-agent-companion)(?:\/|$)/;

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
const violations = paths.sort().flatMap((path) => matches(path, readFileSync(join(root, path), "utf8")));
const imports = vaultPaths.sort().flatMap((path) => companionImports(path, readFileSync(join(root, path), "utf8")));
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
