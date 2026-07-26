---
name: github-workflow
description: Use the GitHub CLI to manage work on dennissoftman/commanders-in-chief. Apply when filing, reading, or updating issues, creating branches or commits, pushing changes, opening or editing pull requests, checking CI, or performing any other GitHub client operation in this repository. Enforces the `main` base branch, `denys/` branch naming, the `dennissoftman` assignee, the full local Rust gate (rustfmt, strict Clippy, workspace tests on the pinned 1.93.0 toolchain), explicit PR titles, work-type-appropriate PR bodies with real verification output, repo label discipline, issue linkage, and the AGENTS.md documentation protocol.
---

# GitHub workflow — commanders-in-chief

This skill **supersedes the generic `anthropic-skills:github-workflow` skill** for this
repository. Where the two disagree, this file wins: the base branch is `main` (not
`development`), branches are prefixed `denys/` (not bare `denys`), and the assignee and sole
contributor is `dennissoftman` (not `denysmitin`). Never request review from `dennissoftman` —
that is the repository owner and the author of every branch here.

## Repository facts

| Fact | Value |
| --- | --- |
| Repository | `dennissoftman/commanders-in-chief` |
| Base branch | `main` |
| Branch prefix | `denys/` |
| Assignee / contributor | `dennissoftman` |
| Merge style | merge commit (`Merge pull request #N from dennissoftman/denys/...`) |
| Toolchain | pinned `1.93.0` in [rust-toolchain.toml](rust-toolchain.toml) |
| CI | single job in [.github/workflows/ci.yml](.github/workflows/ci.yml), runs on every push and PR |

Read [AGENTS.md](AGENTS.md) before any change. It is normative and this skill does not
restate it — in particular the documentation-home rules, determinism invariants, provenance
requirements, and the ban on retail EA data.

## Branches

- Always branch from an up-to-date `main`: `git fetch origin && git switch -c denys/<slug> origin/main`.
- Name is `denys/<kebab-slug>` describing the change, not the milestone number alone.
  Good: `denys/gbuffer-msaa-antialiasing`, `denys/fix-generals-tree-sway`,
  `denys/options-ini-discovery`. Optional leading verb (`fix-`, `upd-`) is fine.
- Never commit directly to `main` and never push to `main`.
- Prefer merging `main` into a long-lived branch over rebasing — the history here carries
  explicit `Merge main into ...` commits and merge commits from PRs.

## Commits

- Imperative mood, no trailing period, wrapped at ~72 characters.
- Prefer Conventional Commits with crate scopes when the change is scoped to crates:
  `fix(formats,tools): decode real WND grammar and mount PatchWindow.big`,
  `feat(render): add cascaded shadow maps`. A plain imperative sentence is acceptable for
  cross-cutting or documentation work: `Restructure README and correct stale command facts`.
- Scope names are the crate suffixes: `core`, `formats`, `vfs`, `render`, `camera`, `tools`,
  plus `fuzz`, `docs`, `ci`.
- Keep formatting-only churn in its own commit (`Apply rustfmt across the workspace`) rather
  than mixed into a behavioral change.

## The local gate — run before every push and before declaring work complete

CI runs exactly three checks on the pinned toolchain, all with `-D warnings`. Run the same
three locally first; a red CI run on a pushed branch is avoidable noise.

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

Details that actually bite here:

- **Use the pinned toolchain.** `rust-toolchain.toml` pins `1.93.0`. Clippy lint sets differ
  between toolchains, so a clean run on a newer default toolchain proves nothing about CI.
  Do not add `+stable`/`+nightly` overrides to the gate commands.
- **`--all-targets` is not optional.** It pulls in tests, benches, and examples. Clippy over
  the library targets alone routinely misses warnings that fail CI.
- **`-D warnings` plus `clippy::pedantic`.** [Cargo.toml](Cargo.toml) sets
  `[workspace.lints.clippy] all = "warn"`, `pedantic = "warn"`, so every pedantic hit is a CI
  failure. Fix the code. Only reach for `#[allow(...)]` with a one-line comment justifying it.
- **`unsafe_code = "forbid"`** at the workspace level. Introducing `unsafe` requires an
  accepted ADR under `docs/adr/` changing that policy first — see AGENTS.md.
- **Fix formatting with `cargo fmt --all`,** not by hand-wrapping lines.
- **`fuzz/` is a separate workspace** ([fuzz/Cargo.toml](fuzz/Cargo.toml) declares its own
  `[workspace]`) and is *not* covered by root `--workspace`/`--all` commands, nor by CI. If a
  change touches `fuzz/`, additionally run `cargo fmt --all --check` from inside `fuzz/`, and
  exercise the affected target with `cargo fuzz run <target>` (nightly + `cargo-fuzz`; the
  targets are `big`, `csf`, `map`, `water_ini`). State in the PR that fuzz coverage is
  outside CI and how you verified it.
- Record the real numbers from the run (test pass/fail counts) — the PR body must quote what
  actually happened, not "all green".

## Pull requests

Open with `gh pr create`. Every PR must have:

1. **An explicit `--title`.** Never let GitHub derive the title from the branch name — that
   produced titles like "Denys/map shadow cascades ao rts camera" on merged PRs here. Use the
   same style as a good commit subject.
2. **`--base main`** and **`--assignee dennissoftman`**.
3. **`--draft` while work is in progress**, marked ready with `gh pr ready <n>` once the local
   gate passes and the body is complete.
4. **A label** from the labels that exist in this repo — `bug`, `documentation`,
   `duplicate`, `enhancement`, `good first issue`, `help wanted`, `invalid`, `question`,
   `wontfix`. There is no `chore`/`refactor` label: map refactors and maintenance to
   `enhancement`, and docs-only work to `documentation`. Do not invent labels; if a new one is
   genuinely needed, create it deliberately with `gh label create` and say why.
5. **A body written for the work type** (below). An empty body is not acceptable — several
   merged PRs here have one, and that is the gap this skill closes.
6. **Issue linkage.** `Closes #N` when it resolves an issue; an explicit "No associated issue"
   line with a sentence of origin when it does not.

Body templates — keep the headings, drop sections that genuinely do not apply:

**Fix** (`bug`)

```markdown
## Issue
Closes #N   <!-- or: No associated issue — <one sentence on where this came from>. -->

## Symptoms
What the user or tool actually observed, and under which edition/profile
(`Generals` vs `--zh`), map, or fixture. Include the wrong output.

## Root cause
Why it happened, naming the file and the mechanism. If a suspected symptom turned out
not to be a bug, say so explicitly and show the evidence that cleared it.

## Treatment
What changed, file by file, and why this fix over the alternatives considered.

## Verification
- `cargo test --workspace` — N passed, 0 failed (M new tests)
- `cargo clippy --workspace --all-targets -- -D warnings` — clean
- `cargo fmt --all --check` — clean
- Any end-to-end evidence (renders, `cic-inspect` runs, determinism re-runs).
```

**Feature** (`enhancement`)

```markdown
## Issue
Closes #N   <!-- or: No associated issue — ... -->

## Motivation
What this unlocks, tied to the active milestone in `CURRENT.md` / `docs/milestones/`.

## Design
The approach, the boundaries it respects (dependency direction, bounded reads, no
simulation-owned resources), and the alternatives rejected.

## Changes
Per crate.

## Compatibility and provenance
Any deliberate divergence from retail behavior, recorded in `COMPATIBILITY.md`; any
source-derived algorithm, with its source and pinned revision per AGENTS.md.

## Verification
(same three gate lines, plus fixtures/negative tests added)
```

**Chore / refactor / docs** (`enhancement` or `documentation`)

```markdown
## Rationale
Why now, and what it makes possible or prevents.

## Changes
What moved or changed. For docs, state which single documentation home each fact now
lives in and what stale content was removed.

## Verification
(the three gate lines; for docs-only, note that no behavior changed)
```

Before marking a PR ready, confirm the AGENTS.md change protocol is satisfied: `CURRENT.md`
points at the real next verified step, milestone progress and completion evidence live in
`docs/milestones/<milestone>.md`, user-visible work is in `CHANGELOG.md` under the active
milestone, permanent design choices are in `docs/adr/`, and nothing is duplicated across them.

Never attach retail EA assets — screenshots, map bytes, strings, audio — to a PR, issue, or
comment. Use synthetic fixtures, or describe the observation from a user-owned install
without shipping its data.

## CI

- `gh pr checks <n> --watch` after pushing; do not mark a draft ready or merge while red.
- The job is `rust` on `ubuntu-latest`. Failures are almost always one of the three gate
  commands, so reproduce locally with the exact command from the log rather than guessing.
- `gh run view <id> --log-failed` to read a failure without scrolling the whole log.

## Issues

- Concise and structured: one-paragraph summary, then reproduction or scope, then what
  "done" means. No speculative essays.
- `--assignee dennissoftman`, plus a label from the existing set.
- Reference the owning crate and the milestone it belongs to.

## Command cheat sheet

```bash
git fetch origin && git switch -c denys/<slug> origin/main
```

```bash
cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

```bash
git push -u origin denys/<slug>
```

```bash
gh pr create --base main --draft --assignee dennissoftman --label enhancement --title "<explicit title>" --body-file <path>
```

```bash
gh pr checks --watch
```

```bash
gh pr ready <n>
```
