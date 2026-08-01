#!/usr/bin/env python3
"""Rewrite the derived counts in the design documents from their source table.

Several documents quote how many engine requirements the mechanics design obliges, how
many of those are already promised or built, and how many are amendments to an accepted
record. Those numbers are *derived* from one table -- the requirements table in
`docs/design/mechanics.md` -- and every one of them was wrong at least once while the
design was being written, because adding a row to a table in one file does not remind
anybody to edit a sentence in another.

So they are generated. A document marks a derived number with an HTML comment span:

    ... a list of <!--count:total-->twenty<!--/count--> requirements ...

and this script replaces what is between the markers. The markers render as nothing, the
prose around them stays in the document where prose belongs, and the script owns only the
number. Capitalisation is preserved from what is already there, so a span opening a
sentence stays capitalised without needing a separate key.

Run it after changing the table. CI runs it and fails on a diff, exactly as it does for
NOTICES.md, so a stale count is a red build rather than something a reader finds later.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# The table these numbers are derived from, and the heading that introduces it.
SOURCE = ROOT / "docs/design/mechanics.md"
SOURCE_HEADING = "## 10. What this document obliges the engine to gain"

# Every document carrying a marked span. The source counts itself.
TARGETS = [
    SOURCE,
    ROOT / "docs/design/README.md",
    ROOT / "docs/adr/3002-corridor-economy.md",
]

SPAN = re.compile(r"(<!--count:(?P<key>[a-z-]+)-->)(?P<body>.*?)(<!--/count-->)", re.DOTALL)

WORDS = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen",
    "eighteen", "nineteen", "twenty",
]
TENS = {
    20: "twenty", 30: "thirty", 40: "forty", 50: "fifty",
    60: "sixty", 70: "seventy", 80: "eighty", 90: "ninety",
}


def word(n: int) -> str:
    """Spell a count. These documents write numbers as words, so the generator does too."""
    if n < 0 or n > 99:
        raise ValueError(f"no spelling for {n}; extend WORDS/TENS if a count grew this far")
    if n <= 20:
        return WORDS[n]
    tens, units = divmod(n, 10)
    base = TENS[tens * 10]
    return base if units == 0 else f"{base}-{WORDS[units]}"


def rows(text: str) -> list[list[str]]:
    """The requirement table's data rows, as lists of stripped cells."""
    section = text.split(SOURCE_HEADING, 1)
    if len(section) != 2:
        raise SystemExit(f"{SOURCE}: heading not found: {SOURCE_HEADING!r}")

    out = []
    for line in section[1].splitlines():
        line = line.strip()
        if not line.startswith("|"):
            # The table is the first one under the heading; stop at the prose after it.
            if out:
                break
            continue
        cells = [c.strip() for c in line.strip("|").split("|")]
        if all(set(c) <= {"-", ":"} for c in cells):  # the |---|---| separator
            continue
        if cells[0].lower().startswith("requirement"):  # the header row
            continue
        out.append(cells)
    if not out:
        raise SystemExit(f"{SOURCE}: found no requirement rows under {SOURCE_HEADING!r}")
    return out


def counts() -> dict[str, int]:
    table = rows(SOURCE.read_text(encoding="utf-8"))
    # "Already promised?" is the last column. "Partly" counts as promised: the row is
    # something the engine part-way has, not new work, which is how the prose reads it.
    promised = [r for r in table if r[-1].startswith(("Yes", "Partly"))]
    amendments = [r for r in table if "amendment" in " ".join(r).lower()]
    return {
        "total": len(table),
        "promised": len(promised),
        "amendments": len(amendments),
    }


def main() -> int:
    values = counts()
    check = "--check" in sys.argv
    stale = []

    for path in TARGETS:
        text = path.read_text(encoding="utf-8")
        seen = set()

        def replace(match: re.Match[str]) -> str:
            key = match.group("key")
            if key not in values:
                raise SystemExit(f"{path}: unknown count key {key!r}; known: {sorted(values)}")
            seen.add(key)
            body = match.group("body")
            spelled = word(values[key])
            if body[:1].isupper():
                spelled = spelled[0].upper() + spelled[1:]
            return f"{match.group(1)}{spelled}{match.group(4)}"

        updated = SPAN.sub(replace, text)
        if not seen:
            raise SystemExit(f"{path}: no <!--count:...--> spans found; is the file still a target?")
        if updated != text:
            stale.append(path)
            if not check:
                path.write_text(updated, encoding="utf-8", newline="\n")

    summary = ", ".join(f"{k}={v}" for k, v in sorted(values.items()))
    if check and stale:
        for path in stale:
            print(f"stale counts: {path.relative_to(ROOT).as_posix()}")
        print(f"run tools/generate-doc-counts.py ({summary})")
        return 1
    if stale:
        for path in stale:
            print(f"updated {path.relative_to(ROOT).as_posix()}")
    print(f"counts: {summary}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
