# The Agent Companion's source lives in this repository; the sandbox stays the boundary

Status: Accepted by the owner, 2026-11-01, including publishing the companion under this repository's MIT license with its history. Amends "One pinned source" in [ADR-0009](0009-optional-agent-companion-helper.md). The rest of ADR-0009 stands.

The companion began as a standalone project ([ADR-0008](0008-agent-gateway-awaits-a-concrete-api.md)) and stayed in its own private repository, pinned here by commit. No decision ever compared one repository with two. The options ADR-0009 weighed were about process isolation, and that is what protects the **Vault**: the helper's XPC sandbox, not where its source is kept.

Two repositories cost a lot once the helper shipped in the app. On 2026-10-31 alone there were three companion pull requests, each with its own pin bump and re-verification; companion CI that couldn't start because of a billing limit; a public app whose helper couldn't be built without private access; and a readiness bug (one process per file against a 10-second limit) that sat between the two test suites.

## Decision

- **One repository, one folder.** The companion's source is `companion/`, with its own `package.json` and lockfile. The Vault's `package.json`, `package-lock.json` and Cargo workspace don't include it.
- **What "One pinned source" becomes.** `src-tauri/companion/companion.lock.json` keeps the `policy.json` SHA-256 and the Node.js release hashes, and `scripts/package-companion.sh` still verifies them. The repository and commit fields and `--companion-source` are gone. The script bundles `companion/` as committed at the current commit, refuses if that folder has uncommitted changes, and records the commit in the helper's `EnoteCompanionCommit` value. Nothing reads that value; it says what a helper was built from.
- **The policy hash stays a separate step.** Changing what the broker may do means changing `policy.json` and its pin together, or packaging refuses. So that change always shows in the pin as well as in the folder.
- **The runtime boundary doesn't move.** The helper stays a separately sandboxed XPC service, and the Vault app's entitlements don't change. This adds no route from an agent into the Vault and no network code to the Vault core or WebView.
- **The Vault scan leaves `companion/` out on purpose.** Calling OpenAI is the broker's job, so `scripts/check-network-source.mjs` skips it and says so. In its place, that script fails if any file in `src/`, `src-tauri/src/` or `crates/` imports from `companion/`.
- **One test run.** `scripts/check.sh` and CI run the companion's tests on Mac, the only place the helper exists, one file at a time because they share port 8789.
- **The history came along.** The source was merged with its original commit SHAs, after a scan of the full history found no real secrets, only `FAKE_` test markers. The commits this app used to pin stay findable here.

## Consequences

- Anyone can now read and build the helper. The code is public under MIT, and that can't be taken back for copies already made.
- A change that crosses the helper and the broker is one pull request, tested and verified together, with no pin to bump.
- **What the commit pin gave up:** bumping it was a deliberate step, naming exactly what would ship. Now anything merged into `companion/` ships in the next packaged build, so review of that folder is the control. The policy hash above still covers the broker's limits.
- Packaging needs a git checkout, because it reads committed files. It still fetches npm packages and Node.js, verified as before, but no longer fetches the companion with git. The resulting app makes no such fetches.
- Tests for the companion don't run on Windows, where the helper is unavailable.
- **Reading old history:** `git blame` reaches the original commits, but `git log --follow` stops at the import. Use the old path: `git log 1c983a7 -- src/generate.mjs`. In imported commit messages, `#N` means a pull request in the old repository, though GitHub links it to this one.
- The old repository stays where it is, private and no longer updated, until the owner decides otherwise. Four commits that existed only on its side branches stayed there; each was squash-merged into its main, so their changes are here.

## Considered Options

- **Keep two repositories:** rejected for the costs above. The split protected nothing that the sandbox doesn't.
- **Make the companion public but keep it separate:** fixes access and CI billing, but keeps the pin bumps and the split test suites.
- **A git submodule or a vendored copy with a pin:** keeps the pin bumps, and a submodule makes a fresh clone harder.
- **An npm workspace:** rejected. It would change the Vault's lockfile and install the companion's dependencies with every Vault `npm ci`.
- **A clean copy without history:** also safe to publish, but loses `git blame` and the reasons recorded in commit messages, and the old pins would point only at a private repository. The scan made this unnecessary.
