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

# Build the documentation site into site/ (warnings are errors).
docs:
    mkdocs build --strict --site-dir site

# Build without network access from vendored dependencies, as distributions do.
offline:
    #!/usr/bin/env bash
    set -euo pipefail
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
    git archive HEAD | tar -x -C "$work"
    cd "$work"
    mkdir -p .cargo
    cargo vendor --locked --versioned-dirs vendor > .cargo/config.toml
    CARGO_HOME="$work/cargo-home" cargo build --offline --frozen --release -p hyprtilt
    "${CARGO_TARGET_DIR:-target}/release/hyprtilt" --version

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

# Pinned clean Arch Linux image for package tests (same pins as the build image).
arch_image := "docker.io/library/archlinux:base-devel-20260920.0.596911@sha256:8745817f349ed24373341ddb92776209eeec3f0364ea48f7f645ac5800d30a50"

# Build and test the AUR recipe (default hyprtilt-git) in a clean Arch Linux container.
pkg-arch package="hyprtilt-git":
    podman run --rm --security-opt label=disable -v "$PWD:/src:ro" {{ arch_image }} \
        bash /src/scripts/test-aur-package.sh {{ package }}

# Regenerate .SRCINFO of an AUR recipe (needs makepkg, e.g. in the build image).
srcinfo package="hyprtilt-git":
    cd packaging/aur/{{ package }} && makepkg --printsrcinfo > .SRCINFO

# Generate the man page and shell completions into dist/assets/ with the
# x86_64 release binary (the content is the same for every architecture).
assets:
    #!/usr/bin/env bash
    set -euo pipefail
    bin="${CARGO_TARGET_DIR:-target}/x86_64-unknown-linux-musl/release/hyprtilt"
    [[ -x "$bin" ]] || cargo build --release --locked --target x86_64-unknown-linux-musl -p hyprtilt
    mkdir -p dist/assets
    "$bin" man | gzip -9n > dist/assets/hyprtilt.1.gz
    "$bin" completions bash > dist/assets/hyprtilt.bash
    "$bin" completions zsh > dist/assets/_hyprtilt
    "$bin" completions fish > dist/assets/hyprtilt.fish
    ls -l dist/assets

# Build .deb packages for amd64 and arm64 into dist/.
pkg-deb: release-build assets
    #!/usr/bin/env bash
    set -euo pipefail
    for target in {{ release_targets }}; do
        cargo deb --locked --no-build --no-strip -p hyprtilt --target "$target" --output dist/
    done
    ls -l dist/*.deb

# Build .rpm packages for x86_64 and aarch64 into dist/.
pkg-rpm: release-build assets
    #!/usr/bin/env bash
    set -euo pipefail
    for target in {{ release_targets }}; do
        cargo generate-rpm -p crates/hyprtilt --target "$target" --target-dir "${CARGO_TARGET_DIR:-target}" -o dist/
    done
    ls -l dist/*.rpm

# Clean distribution images for package installation tests, pinned by digest.
deb_images := "docker.io/library/ubuntu:22.04@sha256:b8b6ee6aa931ecd9d0d952abc34dc0e5f7c6a30c6bb71b079fe399fde0329c02 docker.io/library/ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3 docker.io/library/debian:bookworm@sha256:f37a335e82bca302e955fa39f9dfe28f1be618f016f8a2b56318e5a5111afc26"
rpm_images := "docker.io/library/fedora:43@sha256:a651ddf48ea28a06ed4e1e6519f51c9f47e7a5a138722ade87369b8fbb7e5b42"

# Install the amd64 .deb and the x86_64 .rpm from dist/ in clean containers (host, needs podman).
pkg-test:
    #!/usr/bin/env bash
    set -euo pipefail
    deb="$(ls dist/hyprtilt_*_amd64.deb)"
    rpm="$(ls dist/hyprtilt-*.x86_64.rpm)"
    for image in {{ deb_images }}; do
        podman run --rm --security-opt label=disable -v "$PWD:/src:ro" "$image" \
            bash /src/scripts/test-package-install.sh deb "/src/$deb"
    done
    for image in {{ rpm_images }}; do
        podman run --rm --security-opt label=disable -v "$PWD:/src:ro" "$image" \
            bash /src/scripts/test-package-install.sh rpm "/src/$rpm"
    done

# Release artifacts in dist/: archives, vendored source, SBOM, SHA256SUMS.
dist: pkg-deb pkg-rpm
    scripts/dist.sh

# Record the screenshots and the demo GIF of the documentation from the real binary.
media:
    cargo build --release --locked -p hyprtilt
    uv run --no-project --with pyte==0.8.2 --with pillow==12.3.0 python3 scripts/docs-media.py

# Changelog in Keep a Changelog form from the commit history.
changelog:
    git-cliff --output CHANGELOG.md

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
