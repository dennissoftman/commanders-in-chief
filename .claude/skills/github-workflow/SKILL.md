---
name: github-workflow
description: Use the GitHub CLI to manage work on the commanders-in-chief repository. Apply when filing, reading, or updating issues, creating branches or commits, pushing changes, opening or editing pull requests, checking CI, or performing any other GitHub client operation here. Enforces the `main` base branch, `<type>/<slug>` branch naming, DCO sign-off on every commit, the full local gate (rustfmt, strict Clippy, workspace tests on the pinned 1.93.0 toolchain), the provenance rules that outrank the gate, explicit PR titles, work-type-appropriate PR bodies with real verification output, the repository's existing label set, and issue linkage.
---

# GitHub workflow — commanders-in-chief

This skill **supersedes any generic `github-workflow` skill** for this repository. Where they
disagree, this file wins — the base branch, the branch naming, the label set, and the sign-off
requirement here are all repository facts rather than defaults.

## Repository facts

| Fact | Value |
| --- | --- |
| Repository | inferred from `origin`; let `gh` resolve it rather than hardcoding an owner |
| Base branch | `main`, and it is protected — no direct pushes |
| Branch naming | `<type>/<kebab-slug>`; agent-created branches use `claude/<slug>` |
| Assignee | yourself — `--assignee @me` |
| Reviewers | do not request review from the PR author. The project currently has a single maintainer, so a reviewer is usually not assignable |
| Merge style | merge commit, `Merge pull request #N from <owner>/<branch>` |
| Sign-off | **required on every commit** — `git commit -s` |
| Toolchain | pinned `1.93.0` in `rust-toolchain.toml` |
| CI | two workflows: `rust` in `.github/workflows/ci.yml`, and `counts` in `.github/workflows/docs.yml`. Their path filters are complements — see the CI section |

## Read before changing anything

- `CONTRIBUTING.md` — normative. Contribution licence terms, the DCO, the
  provenance rule, the gate, and the two standing rules about verification.
- `LICENSING.md` — the two licences, the history boundary, and the one rule that
  can silently undo the project's licence. See *Provenance* below.
- `ARCHITECTURE.md` — dependency direction and layering rules a change must
  respect.
- `CURRENT.md` — the active objective. A PR should be legible against it.

Where a fact belongs, so a PR does not duplicate it: the active step in `CURRENT.md`, milestone
scope and completion evidence in `docs/milestones/<milestone>.md`, decisions with consequences in
`docs/adr/`, format specifications in `docs/formats/`, and what the game *is* in `docs/design/`.

## Branches

- Always branch from an up-to-date `main`:
  `git fetch origin && git switch -c <type>/<slug> origin/main`.
- `<type>` matches the commit types already in use: `feat`, `fix`, `docs`, `chore`, `ci`,
  `refactor`. The slug describes the change, not a milestone number.
  Good: `feat/water-surfaces`, `fix/shadow-cascade-caster-reach`, `docs/licensing-boundary`.
- Never commit to `main` and never push to it. It is protected and will reject the push.
- Prefer merging `main` into a long-lived branch over rebasing — the history here carries
  explicit merge commits and rewriting shared branches breaks the hashes other documents cite.

## Commits

- Imperative mood, no trailing period, wrapped at ~72 characters.
- **Every commit needs a DCO `Signed-off-by` line.** Use `git commit -s`. The name must be real
  and the address reachable; see `CONTRIBUTING.md` for the certification you are
  making. A commit without it is not mergeable.
- Prefer Conventional Commits with crate scopes when the change is scoped:
  `feat(render): add cascaded shadow maps`,
  `fix(assets): reject a declared expansion that would exhaust memory`.
  A plain imperative sentence is fine for cross-cutting or documentation work.
- Scopes are the crate suffixes — `core`, `vfs`, `assets`, `camera`, `render` — plus `docs`,
  `ci`, and `tools`.
- Keep formatting-only churn in its own commit rather than mixed into a behavioural change.

## The local gate — run before every push

CI runs exactly these three checks on the pinned toolchain. Run them locally first; a red CI run
on a pushed branch is avoidable noise.

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Details that actually bite:

- **Use the pinned toolchain.** Clippy lint sets differ between toolchains, so a clean run on a
  newer default proves nothing about CI. Do not add `+stable`/`+nightly` overrides.
- **`--all-targets` is not optional.** It pulls in tests and examples; the library targets alone
  routinely miss warnings that fail CI.
- **`-D warnings` plus `clippy::pedantic`.** `Cargo.toml` sets `all = "warn"` and
  `pedantic = "warn"` at workspace scope, so every pedantic hit is a CI failure. Fix the code;
  reach for `#[allow(...)]` only with a one-line justification.
- **`unsafe_code = "forbid"`** at workspace scope. Introducing `unsafe` needs an accepted ADR
  under `docs/adr/` changing that policy first.
- **Fix formatting with `cargo fmt --all`,** not by hand-wrapping.
- **Quote the real numbers** in the PR body — actual pass/fail counts, not "all green".
- **But do not put a test tally in the tree's prose.** A pull request body is a record of one moment
  and never goes stale; a document is read for years. A number in a tracked document should be a
  measurement that argues something — a frame time, a byte-level difference — not an inventory that
  rots. `CURRENT.md` carried "847 tests across ten crates" until it was ninety-five out. Where a count
  in a document is genuinely wanted, generate it and let CI diff it, as
  `tools/generate-doc-counts.py` does.

Two standing rules the gate cannot enforce, both earned the hard way:

- **A green suite is not verification for a rendering change. Look at the capture.** Every
  rendering bug in this project so far passed its own assertions. The render tests write PNGs to
  `target/tmp/`; a renderer PR should say what the capture showed.
- **Presentation needs running, not only testing.** The one bug the headless suite structurally
  could not catch appeared the first time a window opened.

## Provenance — the rules that outrank the gate

A change can pass every check and still be unmergeable. These are the ways.

- **Do not port, translate, or transcribe code, data, or constants from another game** — not from
  a decompilation, a reverse-engineered project, or a wiki documenting one. The tree is
  permissively licensed only because such a derivation was removed. Reimplementing a *published*
  technique is fine and encouraged; cite the paper.
- **Do not copy backward across the seed commit `5e824cf`.** The predecessor's GPL-licensed
  history is an ancestor of `main`, so a clone carries it. No revert, no cherry-pick, and no
  `git show` of a pre-seed file into a current one. This is a real hazard for exactly the
  operations this skill covers, and nothing in the build would fail if it happened. See
  `LICENSING.md`.
- **`docs/design/` is not Apache-2.0.** It is reserved content under `LICENSE-CONTENT`. A PR
  touching it changes what the game *is*, so open an issue first and agree the direction.
- **A dependency change obliges a notices regeneration.** Run
  `python3 tools/generate-notices.py` and commit the result; CI regenerates and fails on a diff.
  A new dependency is a new licence — check it is permissive rather than waving the diff through.
- **Do not attach another game's assets** — screenshots, bytes, strings, or audio — to a PR,
  issue, or comment. Use synthetic fixtures, or describe an observation without shipping data.

## Pull requests

Open with `gh pr create`. Every PR must have:

1. **An explicit `--title`.** Never let GitHub derive one from the branch name — it capitalises
   the slug into things like "Feat/map shadow cascades ao rts camera", which has happened on
   merged PRs here. Use the style of a good commit subject.
2. **`--base main`** and **`--assignee @me`**.
3. **`--draft` while work is in progress**, marked ready with `gh pr ready <n>` once the gate
   passes and the body is complete.
4. **A label that already exists here** — `bug`, `documentation`, `duplicate`, `enhancement`,
   `good first issue`, `help wanted`, `invalid`, `question`, `wontfix`. There is no
   `chore`/`refactor` label: map maintenance and refactors to `enhancement`, docs-only work to
   `documentation`. Do not invent labels; if one is genuinely needed, create it deliberately with
   `gh label create` and say why.
5. **A body written for the work type** (below). An empty body is not acceptable.
6. **Issue linkage.** `Closes #N` when it resolves one; otherwise an explicit "No associated
   issue" line with a sentence on where the work came from.

Body templates — keep the headings, drop sections that genuinely do not apply.

**Fix** (`bug`)

```markdown
## Issue
Closes #N   <!-- or: No associated issue — <one sentence on where this came from>. -->

## Symptoms
What was actually observed, and against which map, fixture, or example. Include the
wrong output — for a rendering bug, what the capture showed.

## Root cause
Why it happened, naming the file and the mechanism. If a suspected symptom turned out
not to be a bug, say so and show the evidence that cleared it.

## Treatment
What changed, file by file, and why this fix over the alternatives considered.

## Verification
- `cargo test --workspace` — N passed, 0 failed (M new tests)
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --all --check` — clean
- For a rendering change: what the capture showed, and where it was written.
```

**Feature** (`enhancement`)

```markdown
## Issue
Closes #N   <!-- or: No associated issue — ... -->

## Motivation
What this unlocks, tied to the active milestone in `CURRENT.md` / `docs/milestones/`.

## Design
The approach, the boundaries it respects (dependency direction, bounded reads, no
simulation state mutated by presentation), and the alternatives rejected.

## Changes
Per crate.

## Provenance
Confirm nothing was ported from another game. Cite the paper for any published
technique reimplemented here. Note any dependency added and its licence.

## Verification
(the gate lines, plus fixtures and negative tests added)
```

**Chore / refactor / docs** (`enhancement` or `documentation`)

```markdown
## Rationale
Why now, and what it makes possible or prevents.

## Changes
What moved or changed. For docs, state which single home each fact now lives in and
what stale content was removed.

## Verification
(the gate lines; for docs-only, note that no behaviour changed)
```

## CI

- **Two workflows with complementary filters, so most PRs run exactly one of them.**
  - `ci.yml` (`rust`) sets `paths-ignore` for `**/*.md`, `.claude/**`, and `.gitignore`, so a
    documentation-only PR does not run it. That is expected, not a stuck run — do not wait on it
    and do not re-push to trigger it.
  - `docs.yml` (`counts`) is the inverse: it runs on `**/*.md`, on the generator, and on itself.
    It regenerates the derived counts in the design documents from their source table and fails
    on a diff. A code-only PR does not run it.
  - A PR mixing code and docs runs both, because `pull_request` evaluates each filter over the
    whole diff.
  - **A PR touching neither still reports no checks**, which is correct rather than broken.
- **A stale count is fixed by running the generator, not by editing the sentence.** Several
  documents quote how many engine requirements the mechanics design obliges; those numbers live
  inside `<!--count:...-->` spans and are generated. Run `python3 tools/generate-doc-counts.py`
  and commit what it writes. Editing the number by hand puts the build back where it was.
- The local gate is the real gate, and it is not conditional on file type. Run it before every
  push, including on branches CI will skip. If the change touches documents, run the counts
  generator too — `--check` reports staleness without writing.
- `gh pr checks <n> --watch` after pushing. Do not mark a draft ready or merge while red.
- Failures in `rust` are almost always one of the three gate commands, so reproduce locally with the exact
  command from the log rather than guessing. `gh run view <id> --log-failed` reads a failure
  without scrolling the whole log.

## Issues

- Concise and structured: a one-paragraph summary, then reproduction or scope, then what "done"
  means. No speculative essays.
- `--assignee @me`, plus a label from the existing set.
- Reference the owning crate and the milestone it belongs to.

## Command cheat sheet

```bash
git fetch origin && git switch -c <type>/<slug> origin/main
```

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

```bash
git commit -s
```

```bash
git push -u origin <type>/<slug>
```

```bash
gh pr create --base main --draft --assignee @me --label enhancement --title "<explicit title>" --body-file <path>
```

```bash
gh pr checks --watch
```

```bash
gh pr ready <n>
```
