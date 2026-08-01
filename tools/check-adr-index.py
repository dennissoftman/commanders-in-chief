#!/usr/bin/env python3
"""Check the ADR index against the records it indexes.

`docs/adr/README.md` carries a table of every record and its status, and that table is not
decoration: the rule that a record adds its row in the same commit is the whole mechanism
stopping two branches from silently claiming the same number. A table that has drifted out of
step with the files is a mechanism nobody can trust.

Three things are checked, and all three had gone wrong at least once:

* **Every record has a row, and every row has a record.** A file with no row defeats the
  collision mechanism; a row with no file is a dangling reference.
* **The row's status matches the record's own.** Six records said `accepted` in the index while
  their implementations had long landed, and two said different things in the two places —
  ADR 7002's file read `accepted` while its row read `accepted, implemented` and its own
  "What implementing it established" section described the implementation.
* **The status is one the process defines.** `docs/adr/README.md` says status moves from
  `proposed` to `accepted` to `accepted, implemented`, so anything else is either a typo or a
  process change that should be made deliberately.

Free prose after the status is allowed and encouraged — several records use it to say which
decision was reversed, or what is still outstanding. Only the leading token has to agree.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ADR = ROOT / "docs/adr"
INDEX = ADR / "README.md"

# Longest first, so "accepted, implemented" is not read as "accepted".
STATUSES = ("accepted, implemented", "accepted", "proposed", "superseded")

ROW = re.compile(r"^\|\s*\[(\d{4})\]\(([^)]+)\)\s*\|([^|]*)\|\s*([^|]+?)\s*\|\s*$", re.MULTILINE)
STATUS_LINE = re.compile(r"^- Status:[ \t]+(.*)$", re.MULTILINE)


def canonical(text: str, where: str, problems: list[str]) -> str | None:
    for status in STATUSES:
        if text.startswith(status):
            return status
    problems.append(f"{where}: status starts with none of {STATUSES}: {text[:60]!r}")
    return None


def main() -> int:
    problems: list[str] = []

    index_text = INDEX.read_text(encoding="utf-8")
    rows = {}
    for number, href, _title, status in ROW.findall(index_text):
        rows[number] = (href, status)
        if not (ADR / href).exists():
            problems.append(f"index row {number}: links to {href}, which does not exist")

    files = {p.name.split("-", 1)[0]: p for p in sorted(ADR.glob("[0-9]*.md"))}

    for number in sorted(set(rows) | set(files)):
        if number not in files:
            problems.append(f"index row {number}: no record file")
            continue
        if number not in rows:
            problems.append(f"{files[number].name}: no row in the index — add one in this commit")
            continue

        match = STATUS_LINE.search(files[number].read_text(encoding="utf-8"))
        if not match:
            problems.append(f"{files[number].name}: no '- Status: ...' line")
            continue

        in_file = canonical(match.group(1).strip(), files[number].name, problems)
        in_index = canonical(rows[number][1], f"index row {number}", problems)
        if in_file and in_index and in_file != in_index:
            problems.append(
                f"{number}: file says {in_file!r}, index says {in_index!r} — one of them is stale"
            )

    for line in problems:
        print(line)
    print(f"checked {len(files)} records against {len(rows)} index rows; {len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    raise SystemExit(main())
