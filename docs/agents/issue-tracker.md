# Issue tracker: Local Markdown

New issues and PRDs are local markdown files under `.scratch/<feature>/`, which git ignores, so a fresh clone or worktree has none. Issues the owner has published are on GitHub (`gh issue list`). Read both; post to GitHub only when the owner asks.

## Conventions

- One feature per directory: `.scratch/<feature-slug>/`
- The PRD is `.scratch/<feature-slug>/PRD.md`
- Implementation issues are `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, numbered from `01`
- Triage state is recorded as a `Status:` line near the top of each issue file (see `triage-labels.md` for the role strings)
- Comments and conversation history append to the bottom of the file under a `## Comments` heading
- Issue files describe the work, never real secret values. Use obviously fake placeholders in examples.

## When a skill says "publish to the issue tracker"

Create a new file under `.scratch/<feature-slug>/` (creating the directory if needed).

## When a skill says "fetch the relevant ticket"

Read the file at the referenced path, or run `gh issue view <number>` for a published issue. The user will normally pass the path or the issue number directly.
