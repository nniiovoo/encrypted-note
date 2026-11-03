// Secret scan of tracked files, run by check.sh with the already-required Node 24+. No
// dependencies. It fails on:
// - PEM private-key blocks (RSA, EC, OPENSSH, PGP and so on) that aren't marked as fake;
// - known live key shapes: OpenAI sk-/sk-proj-, Anthropic sk-ant-, GitHub ghp_/gho_/ghs_/ghu_/
//   ghr_/github_pat_, AWS AKIA/ASIA, Slack xox?-, Stripe sk_live_/rk_live_, Google AIza;
// - 12 or more BIP39 English words in a row, as in a Seed Phrase, also as a list, a numbered list,
//   a table or comment lines, or split into strings joined by a + at a line end. The word list is
//   the one the app itself uses: the bip39 crate pinned in Cargo.lock, read from Cargo's source cache.
// A value is allowed when it says it's a placeholder (FAKE, EXAMPLE, PLACEHOLDER, DUMMY,
// REDACTED, or eight or more X's). The only allowed Seed Phrases are the published BIP39 test
// vectors listed below. No file/line suppressions: use an obviously fake value instead.
// Findings print as file:line and the kind of value, never the value itself.
// shortcut: it reads only tracked files as they are on disk now, not history, staged-only
// content or untracked files. Passwords, raw Private Keys and secrets with no known shape get
// past it, so AGENTS.md's rule still applies.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, lstatSync, readdirSync, readFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const placeholder = /FAKE|EXAMPLE|PLACEHOLDER|DUMMY|REDACTED|X{8}/i;
const mixed = (value) => /[a-z]/.test(value) && /[A-Z]/.test(value) && /[0-9]/.test(value);
const keyShapes = [
  ["Anthropic API key", /\bsk-ant-[A-Za-z0-9]+-[A-Za-z0-9_-]{32,}/g, mixed],
  ["OpenAI API key", /\bsk-(?!ant-)(?:[a-z]+-)?[A-Za-z0-9_-]{32,}/g, mixed],
  ["GitHub token", /\b(?:ghp|gho|ghs|ghu|ghr)_[A-Za-z0-9]{36,}\b|\bgithub_pat_[A-Za-z0-9_]{60,}/g],
  ["AWS access key ID", /\b(?:AKIA|ASIA)[A-Z0-9]{16}\b/g],
  ["Slack token", /\bxox[abposr]-[A-Za-z0-9-]{10,}/g],
  ["Stripe live key", /\b(?:sk|rk)_live_[A-Za-z0-9]{20,}/g],
  ["Google API key", /\bAIza[0-9A-Za-z_-]{35}/g],
];
const pemBegin = /-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----/g;
const pemEnd = /-----END [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----/;
// Upper case only here: base64 key text would match a case-blind word now and then.
const pemPlaceholder = /FAKE|EXAMPLE|PLACEHOLDER|DUMMY|REDACTED/;
// Published BIP39 test vectors (the BIP's own English vectors and the bip39 crate's tests).
const testVectors = new Set([
  `${"abandon ".repeat(11)}about`,
  `${"abandon ".repeat(17)}agent`,
  `${"abandon ".repeat(23)}art`,
  "legal winner thank year wave sausage worth useful legal winner thank yellow",
  "letter advice cage absurd amount doctor acoustic avoid letter advice cage above",
  `${"zoo ".repeat(11)}wrong`,
  `${"zoo ".repeat(17)}when`,
  `${"zoo ".repeat(23)}vote`,
]);
// Words may be split only by spaces, line breaks (also written as \n or a Rust \ continuation) and
// the quotes, commas and brackets of a list.
const wordGap = /^[\s"'`,[\]\\]*$/;
// Before that, these are blanked out: list numbers (1. or 1)); a table's | and its number-only
// cells; the -, *, >, # and // or /* that start a list, quote, heading or comment line; and a + or
// */ that ends a line. Other digits stay, so a word list numbered like the EFF one ("11111", a tab,
// then the word on each line) doesn't count.
const blank = (match) => " ".repeat(match.length);
function unlabel(text) {
  return text.split("\n").map((line) => {
    const plain = line
      .replace(/^[ \t]*(?:(?:\d{1,2}[.)]|[-*>#]+)(?:\s+|$)|\/[/*]+!?[ \t]*)+/, blank)
      .replace(/(?<!\S)\d{1,2}[.)](?=\s|$)/g, blank)
      .replace(/[ \t]*(?:\+|\*\/)[ \t\r]*$/, blank);
    return /^[ \t]*\|/.test(plain) ? plain.replace(/(?<=\|)[ \t]*\d{1,2}[.)]?[ \t]*(?=\|)/g, blank).replace(/\|/g, " ") : plain;
  }).join("\n");
}

function lineOf(source, index) {
  return source.slice(0, index).split("\n").length;
}

function keyFindings(path, source) {
  const found = [];
  for (const [kind, pattern, shaped = () => true] of keyShapes) {
    for (const match of source.matchAll(pattern)) {
      if (!placeholder.test(match[0]) && shaped(match[0])) found.push(`${path}:${lineOf(source, match.index)}: ${kind}`);
    }
  }
  for (const match of source.matchAll(pemBegin)) {
    const rest = source.slice(match.index, match.index + 16384);
    const end = rest.match(pemEnd);
    if (!pemPlaceholder.test(end ? rest.slice(0, end.index + end[0].length) : rest)) {
      found.push(`${path}:${lineOf(source, match.index)}: PEM private-key block`);
    }
  }
  return found;
}

function seedFindings(path, source, words) {
  const found = [];
  let run = [];
  const vectors = [...testVectors].map((vector) => vector.split(" "));
  // Test vectors are taken out first, so two of them side by side in a list still pass.
  const flush = () => {
    let rest = [];
    const report = () => {
      if (rest.length >= 12) found.push(`${path}:${lineOf(source, rest[0].index)}: ${rest.length} BIP39 words in a row (Seed Phrase)`);
      rest = [];
    };
    for (let i = 0; i < run.length; ) {
      const vector = vectors.find((v) => v.every((word, j) => run[i + j]?.text === word));
      if (vector) {
        report();
        i += vector.length;
      } else {
        rest.push(run[i]);
        i += 1;
      }
    }
    report();
    run = [];
  };
  // Same length as the source, so match indexes still give the right line.
  const text = unlabel(source.replace(/\\[nrt]/g, "  "));
  for (const match of text.matchAll(/[A-Za-z]+/g)) {
    const word = match[0].toLowerCase();
    const previous = run.at(-1);
    if (previous && !wordGap.test(text.slice(previous.index + previous.text.length, match.index))) flush();
    if (words.has(word)) run.push({ text: word, index: match.index });
    else flush();
  }
  flush();
  return found;
}

function scan(path, source, words) {
  return [...keyFindings(path, source), ...seedFindings(path, source, words)];
}

function bip39Words() {
  const lock = readFileSync(join(root, "Cargo.lock"), "utf8");
  const version = lock.match(/\[\[package\]\]\nname = "bip39"\nversion = "([^"]+)"/)?.[1];
  assert.ok(version, "Cargo.lock has no bip39 package.");
  const registry = join(process.env.CARGO_HOME ?? join(homedir(), ".cargo"), "registry", "src");
  const sources = existsSync(registry) ? readdirSync(registry) : [];
  const file = sources.map((index) => join(registry, index, `bip39-${version}`, "src", "language", "english.rs")).find(existsSync);
  if (!file) {
    console.error(`The BIP39 word list (bip39 ${version}) is not in Cargo's source cache: run 'cargo fetch --locked', then try again.`);
    process.exit(1);
  }
  const words = new Set(readFileSync(file, "utf8").match(/"[a-z]+"/g)?.map((word) => word.slice(1, -1)));
  assert.equal(words.size, 2048, `${file} is not the 2048-word BIP39 English list.`);
  return words;
}

const words = bip39Words();
const list = [...words];
// One test's prefix: two words, then the list's own first 21 words (crates/notes/tests/seed.rs).
testVectors.add(`zoo wrong ${list.slice(0, 21).join(" ")}`);

// Fake values only, assembled at run time so this file never holds a real-shaped one.
const fakeBody = (length) => "Fa1".repeat(length).slice(0, length);
for (const [kind, value] of [
  ["OpenAI API key", "sk-" + "proj-" + fakeBody(40)],
  ["OpenAI API key", "sk-" + fakeBody(48)],
  ["Anthropic API key", "sk-" + "ant-api03-" + fakeBody(40)],
  ["GitHub token", "gh" + "p_" + fakeBody(36)],
  ["GitHub token", "gh" + "s_" + fakeBody(36)],
  ["GitHub token", "github" + "_pat_" + fakeBody(70)],
  ["AWS access key ID", "AK" + "IA" + "Q".repeat(16)],
  ["AWS access key ID", "AS" + "IA" + "Q".repeat(16)],
  ["Slack token", "xo" + "xb-" + "1234567890-abcdefghij"],
  ["Stripe live key", "sk" + "_live_" + fakeBody(24)],
  ["Stripe live key", "rk" + "_live_" + fakeBody(24)],
  ["Google API key", "AI" + "za" + fakeBody(35)],
  ["PEM private-key block", "-----BEGIN " + "RSA PRIVATE KEY-----\nTUlJ\n-----END " + "RSA PRIVATE KEY-----"],
  ["PEM private-key block", "-----BEGIN " + "OPENSSH PRIVATE KEY-----\nb3Bl"],
]) {
  assert.deepEqual(scan("fake.txt", `fake first line\nconst value = "${value}";\n`, words), [`fake.txt:2: ${kind}`], kind);
}
for (const value of [
  "sk-" + "FAKE-0000",
  "sk-" + "proj-FAKE" + fakeBody(40),
  "sk-" + "ant-api03-EXAMPLE" + fakeBody(40),
  "AK" + "IAIOSFODNN7EXAMPLE",
  "gh" + "p_" + "X".repeat(36),
  "-----BEGIN " + "FAKE PRIVATE KEY-----\nZmFrZQ==\n-----END " + "FAKE PRIVATE KEY-----",
  "-----BEGIN " + "PRIVATE KEY-----\nPLACEHOLDER\n-----END " + "PRIVATE KEY-----",
  "sk-this-is-a-long-kebab-case-css-class-name",
  "task-allow allow-dyld-environment-variables",
]) {
  assert.deepEqual(scan("fake.txt", value, words), [], value);
}
const fakePhrase = Array.from({ length: 12 }, (_, i) => list[(i * 97 + 5) % list.length]);
assert.deepEqual(scan("fake.rs", `// fake\nlet phrase = "${fakePhrase.join(" ")}";`, words), ["fake.rs:2: 12 BIP39 words in a row (Seed Phrase)"]);
assert.deepEqual(scan("fake.ts", `const phrase = [\n  "${fakePhrase.join('",\n  "')}",\n];`, words), ["fake.ts:2: 12 BIP39 words in a row (Seed Phrase)"]);
assert.deepEqual(scan("fake.rs", `let phrase = "${fakePhrase.slice(0, 6).join(" ")} \\\n    ${fakePhrase.slice(6).join(" ")}";`, words).length, 1);
assert.deepEqual(scan("fake.rs", `let phrase = "${fakePhrase.slice(0, 11).join(" ")}";`, words), []);
assert.deepEqual(scan("fake.rs", `${fakePhrase.slice(0, 6).join(" ")}; ${fakePhrase.slice(6).join(" ")}`, words), []);
// The same fake phrase as wallets and code usually lay it out: each must be found once, on line 2.
const [firstHalf, secondHalf] = [fakePhrase.slice(0, 6).join(" "), fakePhrase.slice(6).join(" ")];
for (const [layout, value] of [
  ["numbered list", fakePhrase.map((word, i) => `${i + 1}. ${word}`).join("\n")],
  ["numbered on one line", fakePhrase.map((word, i) => `${i + 1}) ${word}`).join("  ")],
  ["bullet list", fakePhrase.map((word) => `- ${word}`).join("\n")],
  ["quote", `> ${firstHalf}\n> ${secondHalf}`],
  ["table", `| ${fakePhrase.slice(0, 6).map((word, i) => `${i + 1}. ${word}`).join(" | ")} |\n| ${fakePhrase.slice(6).join(" | ")} |`],
  ["numbered table", fakePhrase.map((word, i) => `| ${i + 1} | ${word} |`).join("\n")],
  ["line comments", `// ${firstHalf}\n// ${secondHalf}`],
  ["block comment", `/* ${firstHalf}\n * ${secondHalf} */`],
  ["joined strings", `const phrase = "${firstHalf} " +\n  "${secondHalf}";`],
]) {
  assert.deepEqual(scan("fake.md", `fake first line\n${value}`, words), ["fake.md:2: 12 BIP39 words in a row (Seed Phrase)"], layout);
}
assert.deepEqual(scan("fake.txt", fakePhrase.map((word, i) => `${11111 + i}\t${word}`).join("\n"), words), []);
assert.deepEqual(scan("fake.ts", `const phrase = "${firstHalf} " + other + "${secondHalf}";`, words), []);
for (const vector of testVectors) assert.deepEqual(scan("fake.rs", `"${vector}"`, words), [], vector);
assert.deepEqual(scan("fake.rs", `["${[...testVectors].slice(0, 2).join('", "')}"]`, words), []);
assert.deepEqual(scan("fake.rs", `"${[...testVectors][0]} ${fakePhrase.join(" ")}"`, words).length, 1);
assert.deepEqual(scan("fake.txt", "-----BEGIN " + "EC PRIVATE KEY-----\nTUlJ\n-----END " + "EC PRIVATE KEY-----\nsk-FAKE-0000", words).length, 1);

const paths = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8", maxBuffer: 64 << 20 }).split("\0").filter(Boolean);
const findings = [];
let scanned = 0;
for (const path of paths.sort()) {
  // Regular files only: a tracked symlink could point anywhere, such as a Vault folder.
  if (!lstatSync(join(root, path), { throwIfNoEntry: false })?.isFile()) continue;
  const bytes = readFileSync(join(root, path));
  if (bytes.includes(0)) continue;
  scanned += 1;
  findings.push(...scan(path, bytes.toString("utf8"), words));
}
if (findings.length) {
  console.error(findings.join("\n"));
  console.error("Remove the value and use an obviously fake placeholder (AGENTS.md). If it was ever real, cancel it at its service: a commit can't be fully taken back.");
  process.exitCode = 1;
} else {
  console.log(`Secret scan passed (${scanned} tracked files; fake pass/fail self-check passed).`);
}
