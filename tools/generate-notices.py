#!/usr/bin/env python3
"""Generates NOTICES.md from the workspace's resolved dependency graph.

    python3 tools/generate-notices.py

Permissive licences require their notices to accompany a distributed binary, so this file is an
obligation rather than documentation. Generated rather than maintained by hand: a dependency bump
becomes a diff to review instead of an audit to redo, and the one failure mode that matters -- a new
dependency whose licence nobody looked at -- shows up as a changed line.

The listing records each crate's declared SPDX expression. It is the summary a release needs, not a
substitute for shipping the licence texts themselves; several licences require the full text, which a
release process should collect from the vendored sources.
"""

import json
import pathlib
import subprocess
import sys

HEADER = """# Third-party notices

The engine depends on the crates listed below. Each is distributed under the licence shown, and this
listing exists because those licences require their notices to accompany a binary.

**Generated — do not edit by hand.** Regenerate with:

```bash
python3 tools/generate-notices.py
```

Every entry is permissive. If a regeneration ever introduces a copyleft licence, that is a decision to
make deliberately rather than a diff to wave through: see [LICENSING.md](LICENSING.md).

Note when scanning this list: an SPDX expression joined by `OR` offers a *choice*, and a copyleft option
among permissive ones is not an obligation — `r-efi` reads `MIT OR Apache-2.0 OR LGPL-2.1-or-later`, and
this project takes Apache-2.0. Only a licence with no permissive alternative would need attention.

Where a dependency offers Apache-2.0 among its options, this project takes it, matching the engine's own
licence — one set of redistribution obligations to satisfy at packaging time rather than several.

A release must also ship the full licence *texts*, which several of these require and a summary cannot
satisfy. Collect them from the vendored sources at packaging time.

"""


def main() -> int:
    root = pathlib.Path(__file__).resolve().parent.parent
    try:
        raw = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--all-features"],
            cwd=root,
            capture_output=True,
            check=True,
            text=True,
        ).stdout
    except (OSError, subprocess.CalledProcessError) as error:
        print(f"could not run cargo metadata: {error}", file=sys.stderr)
        return 1

    metadata = json.loads(raw)

    # This workspace's own crates are excluded, because the notices are about third-party code.
    #
    # Derived from `workspace_members` rather than from a list written down here. A hardcoded set has
    # to be updated every time a crate is added, nothing fails when it is not, and the symptom is one
    # of this project's own Apache-2.0 crates listed as a dependency it must give notice for -- which
    # is how `cic-ui` first appeared in this file. Matching on package id rather than on name because
    # the id is what `workspace_members` holds, and its exact string format has changed between cargo
    # releases.
    ours = set(metadata.get("workspace_members", ()))
    if not ours:
        print(
            "cargo metadata reported no workspace members, which would list this project's own "
            "crates as third-party dependencies; refusing to write NOTICES.md",
            file=sys.stderr,
        )
        return 1

    packages = [
        package for package in metadata["packages"] if package["id"] not in ours
    ]
    packages.sort(key=lambda package: (package["name"].lower(), package["version"]))

    unlicensed = [package["name"] for package in packages if not package.get("license")]

    lines = [HEADER, f"{len(packages)} dependencies.\n", "| Crate | Version | Licence |", "|---|---|---|"]
    for package in packages:
        licence = package.get("license") or "**UNSPECIFIED**"
        lines.append(f"| `{package['name']}` | {package['version']} | {licence} |")
    lines.append("")

    (root / "NOTICES.md").write_text("\n".join(lines), encoding="utf-8")
    print(f"wrote NOTICES.md with {len(packages)} dependencies")

    if unlicensed:
        # Not a generation failure, but the one result that needs a human: a crate declaring no licence
        # cannot be redistributed on assumption.
        print(
            f"warning: {len(unlicensed)} crate(s) declare no licence: {', '.join(unlicensed)}",
            file=sys.stderr,
        )
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main())
