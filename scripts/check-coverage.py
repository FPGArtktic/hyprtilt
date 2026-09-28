#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
#
# Enforce the line coverage thresholds from CONTRIBUTING.md on the JSON
# summary written by "cargo llvm-cov report --json --summary-only".
#
# Thresholds: hyprtilt-core >= 85 %, whole workspace >= 70 %. A group without
# any instrumented line passes (there is nothing to cover yet).
#
# With --badge FILE it also writes a shields.io endpoint description of the
# workspace coverage, which CI publishes so that the README badge needs no
# third-party service.
#
# Usage: scripts/check-coverage.py coverage.json [--badge badge.json]

import json
import sys

THRESHOLDS = [
    # (label, path fragment selecting the files, minimum percent)
    ("hyprtilt-core", "/crates/hyprtilt-core/src/", 85.0),
    ("workspace", "/crates/", 70.0),
]

# (minimum percent, colour) in descending order, as shields.io names colours.
BADGE_COLORS = [(90.0, "brightgreen"), (80.0, "green"), (70.0, "yellowgreen"), (60.0, "yellow"), (0.0, "red")]


def write_badge(path: str, percent: float) -> None:
    """Write a shields.io endpoint description of the coverage."""
    color = next(c for minimum, c in BADGE_COLORS if percent >= minimum)
    badge = {
        "schemaVersion": 1,
        "label": "coverage",
        "message": f"{percent:.1f}%",
        "color": color,
    }
    with open(path, "w", encoding="utf-8") as f:
        json.dump(badge, f, indent=2)
        f.write("\n")
    print(f"badge: {path} ({badge['message']}, {color})")


def main() -> int:
    args = sys.argv[1:]
    badge_path = None
    if "--badge" in args:
        i = args.index("--badge")
        if len(args) < i + 2:
            print(f"usage: {sys.argv[0]} coverage.json [--badge badge.json]", file=sys.stderr)
            return 2
        badge_path = args[i + 1]
        del args[i : i + 2]
    if len(args) != 1:
        print(f"usage: {sys.argv[0]} coverage.json [--badge badge.json]", file=sys.stderr)
        return 2
    with open(args[0], encoding="utf-8") as f:
        report = json.load(f)

    files = [f for data in report["data"] for f in data["files"]]
    status = 0
    for label, fragment, minimum in THRESHOLDS:
        selected = [f for f in files if fragment in f["filename"]]
        count = sum(f["summary"]["lines"]["count"] for f in selected)
        covered = sum(f["summary"]["lines"]["covered"] for f in selected)
        if count == 0:
            print(f"{label}: no instrumented lines, threshold {minimum:.0f} % skipped")
            continue
        percent = 100.0 * covered / count
        verdict = "OK" if percent >= minimum else "FAIL"
        print(f"{label}: {percent:.2f} % of {count} lines (minimum {minimum:.0f} %) {verdict}")
        if percent < minimum:
            status = 1
        if label == "workspace" and badge_path:
            write_badge(badge_path, percent)
    return status


if __name__ == "__main__":
    sys.exit(main())
