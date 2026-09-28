# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
#
# Task runner. "just <target>" runs a target with the host toolchain,
# "just podman <target>..." runs the same "just <target>..." inside the
# pinned build image (containers/build/Containerfile). CI runs the targets
# inside that image too, so the commands are identical everywhere.

set shell := ["bash", "-euo", "pipefail", "-c"]

image := env("HYPRTILT_IMAGE", "localhost/hyprtilt-build:latest")
msrv := `sed -n 's/^rust-version = "\(.*\)"/\1/p' Cargo.toml`
release_targets := "x86_64-unknown-linux-musl aarch64-unknown-linux-musl"

# List the recipes.
default:
    @just --list --unsorted

# Build every crate and target (debug).
build:
    cargo build --workspace --all-targets --locked

# Run unit, integration and documentation tests.
test:
    cargo test --workspace --all-targets --locked
    cargo test --workspace --doc --locked

# Check formatting, run clippy with warnings as errors, lint scripts and workflows.
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    shellcheck scripts/*.sh
    actionlint

# Build the API documentation with warnings as errors.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked

# Check dependency licenses, advisories, bans and sources.
deny:
    cargo deny --locked check

# Measure line coverage and enforce the thresholds (core >= 85 %, all >= 70 %).
cov:
    cargo llvm-cov clean --workspace
    cargo llvm-cov --workspace --all-targets --locked --no-report
    cargo llvm-cov report --lcov --output-path lcov.info
    cargo llvm-cov report --json --summary-only --output-path coverage.json
    python3 scripts/check-coverage.py coverage.json

# Check that the workspace builds with the minimum supported Rust version.
msrv:
    cargo +{{ msrv }} check --workspace --all-targets --locked

# Check the commit messages of BASE..HEAD (subject format, DCO sign-off).
commits base="origin/main":
    scripts/check-commits.sh "{{ base }}"

# Everything the main CI job checks.
ci: lint build test doc deny cov

# Build static release binaries for every release target into dist/.
release-build:
    #!/usr/bin/env bash
    set -euo pipefail
    target_dir="${CARGO_TARGET_DIR:-target}"
    mkdir -p dist
    for target in {{ release_targets }}; do
        cargo build --release --locked --target "$target" -p hyprtilt
        install -Dm0755 "$target_dir/$target/release/hyprtilt" "dist/$target/hyprtilt"
    done
    file dist/*/hyprtilt 2>/dev/null || true

# Build the build image.
image:
    podman build -f containers/build/Containerfile -t {{ image }} .

# Run "just ARGS" inside the build image, e.g. "just podman ci".
podman +args:
    podman run --rm --init=false \
        --userns=keep-id \
        --security-opt label=disable \
        -v "$PWD:/src" -w /src \
        -v hyprtilt-cargo:/cache/cargo \
        -v hyprtilt-target:/cache/target \
        -v hyprtilt-pacman:/var/cache/pacman/pkg \
        {{ image }} just {{ args }}
