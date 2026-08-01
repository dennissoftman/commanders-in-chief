#!/usr/bin/env python3
"""Check that every relative link and heading anchor in the repository's markdown resolves.

The documents here cross-reference heavily and on purpose: a milestone cites an ADR, an ADR
cites a format, a format cites the invariant it obeys. That is what keeps each fact in one
home. It also means a renamed heading or a moved file breaks a reference silently — nothing
in a build reads markdown, so a dead link survives until a reader hits it.

Two classes are caught:

* A relative link whose target file or directory does not exist. This is how the reference
  to `cic-script/src/real.rs` outlived the extraction of that code into `cic-math`.
* A `#fragment` naming no heading in the target document. This is the common one, because
  renumbering a section changes its anchor while the link keeps pointing at the old slug.

External links are not checked. Fetching them makes the run depend on the network and on
other people's uptime, which turns a documentation check into a flaky one.

Anchors are slugged the way GitHub does it: lowercase, inline markup stripped, non-word
characters dropped, spaces to hyphens, and a `-1`/`-2` suffix on repeats of an earlier
heading in the same file.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Directories with nothing authored in them. `target` holds build output, some of which is
# markdown from dependencies, and none of it is ours to fix.
SKIP_DIRS = {".git", "target", "node_modules"}

LINK = re.compile(r"\[[^\]]*\]\(\s*([^)\s]+?)\s*\)")
HEADING = re.compile(r"^(#{1,6})\s+(.*?)\s*$")
FENCE = re.compile(r"^\s*(```|~~~)")

_INLINE = [
    (re.compile(r"`([^`]*)`"), r"\1"),              # code spans
    (re.compile(r"\*\*([^*]*)\*\*"), r"\1"),        # bold
    (re.compile(r"\*([^*]*)\*"), r"\1"),            # italic
    (re.compile(r"\[([^\]]*)\]\([^)]*\)"), r"\1"),  # links, keeping the text
]


def slug(text: str) -> str:
    for pattern, repl in _INLINE:
        text = pattern.sub(repl, text)
    text = re.sub(r"[^\w\- ]", "", text.lower())
    return text.replace(" ", "-")


def anchors(path: Path) -> set[str]:
    """Every heading anchor in a document, including GitHub's suffixes for repeats."""
    found: set[str] = set()
    counts: dict[str, int] = {}
    in_fence = False
    for line in path.read_text(encoding="utf-8").splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:  # a `# comment` in a shell block is not a heading
            continue
        match = HEADING.match(line)
        if not match:
            continue
        base = slug(match.group(2))
        seen = counts.get(base, 0)
        counts[base] = seen + 1
        found.add(base if seen == 0 else f"{base}-{seen}")
    return found


def documents() -> list[Path]:
    out = []
    for path in sorted(ROOT.rglob("*.md")):
        if any(part in SKIP_DIRS for part in path.relative_to(ROOT).parts):
            continue
        out.append(path)
    return out


def main() -> int:
    cache: dict[Path, set[str]] = {}
    broken: list[str] = []
    checked = 0

    for doc in documents():
        text = doc.read_text(encoding="utf-8")
        here = doc.relative_to(ROOT).as_posix()
        for href in LINK.findall(text):
            if href.startswith(("http://", "https://", "mailto:", "#!")):
                continue
            file_part, _, fragment = href.partition("#")
            target = (doc.parent / file_part).resolve() if file_part else doc
            checked += 1

            if not target.exists():
                broken.append(f"{here}: {href} -> no such file")
                continue
            if not fragment or target.is_dir() or target.suffix.lower() != ".md":
                continue
            if target not in cache:
                cache[target] = anchors(target)
            if fragment not in cache[target]:
                broken.append(f"{here}: {href} -> no such heading")

    for line in broken:
        print(line)
    print(f"checked {checked} links across {len(documents())} documents; {len(broken)} broken")
    return 1 if broken else 0


if __name__ == "__main__":
    raise SystemExit(main())
