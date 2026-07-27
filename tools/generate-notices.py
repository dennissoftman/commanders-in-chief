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

# Crates in this workspace, excluded because the notices are about third-party code.
OURS = {"cic-assets", "cic-camera", "cic-core", "cic-render", "cic-vfs"}

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
this project takes MIT. Only a licence with no permissive alternative would need attention.

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

    packages = [
        package
        for package in json.loads(raw)["packages"]
        if package["name"] not in OURS
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
