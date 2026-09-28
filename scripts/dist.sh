#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
#
# Assemble the release artifacts in dist/ from binaries and assets that
# "just release-build assets pkg-deb pkg-rpm" produced:
#
#   hyprtilt-<version>-<target>.tar.gz   binary, man page, completions,
#                                        LICENSE, README.md
#   hyprtilt-<version>-vendor.tar.gz     source with vendored dependencies
#                                        and a .cargo/config.toml, for
#                                        offline builds
#   hyprtilt-<version>.spdx.json         SBOM of the dependency tree
#   SHA256SUMS                           checksums of all of the above and
#                                        of the .deb and .rpm packages
#
# The archives are reproducible: fixed owner, sorted names, and the commit
# time as the file time.
#
# Usage: scripts/dist.sh

set -euo pipefail

version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"
epoch="$(git log -1 --format=%ct)"
targets=(x86_64-unknown-linux-musl aarch64-unknown-linux-musl)
tar_opts=(--sort=name --owner=0 --group=0 --numeric-owner --mtime="@${epoch}")
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

for target in "${targets[@]}"; do
    name="hyprtilt-${version}-${target}"
    dir="${work}/${name}"
    install -Dm755 "dist/${target}/hyprtilt" "${dir}/hyprtilt"
    install -Dm644 dist/assets/hyprtilt.1.gz "${dir}/man/hyprtilt.1.gz"
    install -Dm644 -t "${dir}/completions" \
        dist/assets/hyprtilt.bash dist/assets/_hyprtilt dist/assets/hyprtilt.fish
    install -Dm644 -t "${dir}" LICENSE README.md
    tar "${tar_opts[@]}" -C "${work}" -cf - "${name}" | gzip -9n >"dist/${name}.tar.gz"
done

src="${work}/hyprtilt-${version}"
git archive --format=tar --prefix="hyprtilt-${version}/" HEAD | tar -x -C "${work}"

# The SBOM describes the dependency tree (Cargo.lock), so it is taken
# before the dependencies are vendored.
echo "==> SBOM"
SYFT_CHECK_FOR_APP_UPDATE=false syft scan "dir:${src}" --source-name hyprtilt \
    --source-version "${version}" -o "spdx-json=dist/hyprtilt-${version}.spdx.json" -q

echo "==> vendored source"
(
    cd "${src}"
    mkdir -p .cargo
    cargo vendor --locked --versioned-dirs vendor >.cargo/config.toml
)
tar "${tar_opts[@]}" -C "${work}" -cf - "hyprtilt-${version}" | gzip -9n \
    >"dist/hyprtilt-${version}-vendor.tar.gz"

echo "==> checksums"
(
    cd dist
    sha256sum -- *.tar.gz *.deb *.rpm ./*.spdx.json | sed 's| \./| |' >SHA256SUMS
    cat SHA256SUMS
)
