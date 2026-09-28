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
# Usage: scripts/check-coverage.py coverage.json

import json
import sys

THRESHOLDS = [
    # (label, path fragment selecting the files, minimum percent)
    ("hyprtilt-core", "/crates/hyprtilt-core/src/", 85.0),
    ("workspace", "/crates/", 70.0),
]


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} coverage.json", file=sys.stderr)
        return 2
    with open(sys.argv[1], encoding="utf-8") as f:
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
    return status


if __name__ == "__main__":
    sys.exit(main())
