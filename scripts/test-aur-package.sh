#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
#
# Build and test an AUR recipe (packaging/aur/<package>) in a clean Arch
# Linux container, the way an AUR user builds it:
#
#   * .SRCINFO must match the recipe;
#   * namcap checks the recipe and the package;
#   * makepkg -s builds the committed HEAD of the repository mounted at
#     /src (the recipe's source is pointed at it, so unpushed commits are
#     tested, uncommitted changes are not), runs check() and packages it;
#   * the package is installed with pacman -U and its files are checked.
#
# Run it through "just pkg-arch", which starts the container. Inside the
# container it runs as root, prepares an unprivileged user and re-runs
# itself as that user for the makepkg part.
#
# Usage: scripts/test-aur-package.sh [package]   (default: hyprtilt-git)

set -euo pipefail

pkg="${1:-hyprtilt-git}"
src="${HYPRTILT_SRC:-/src}"
recipe="${src}/packaging/aur/${pkg}"

if [[ "$(id -u)" -eq 0 ]]; then
    [[ -f "${recipe}/PKGBUILD" ]] || { echo "no recipe: ${recipe}" >&2; exit 1; }
    pacman -Syu --noconfirm --needed git namcap sudo >/dev/null
    # The repository may belong to another user (CI checkouts).
    git config --global --add safe.directory '*'
    useradd --create-home builder
    printf 'builder ALL=(root) NOPASSWD: /usr/bin/pacman\n' >/etc/sudoers.d/builder
    # A private clone: makepkg must not write to the mounted repository.
    git clone --quiet "${src}" /home/builder/repo
    chown -R builder:builder /home/builder/repo
    exec sudo -u builder env HYPRTILT_SRC=/home/builder/repo bash "$0" "${pkg}"
fi

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

echo "==> .SRCINFO is up to date"
(cd "${recipe}" && diff -u .SRCINFO <(makepkg --printsrcinfo))

echo "==> namcap on the recipe"
namcap "${recipe}/PKGBUILD" | tee "${work}/namcap-pkgbuild.txt"

echo "==> makepkg -s from ${src}"
cp "${recipe}/PKGBUILD" "${work}/"
sed -i "s|^source=(.*|source=(\"\${_pkgname}::git+file://${src}\")|" "${work}/PKGBUILD"
(cd "${work}" && makepkg --syncdeps --noconfirm --cleanbuild --noprogressbar)

package="$(find "${work}" -maxdepth 1 -name "${pkg}-*.pkg.tar.zst" ! -name "${pkg}-debug-*" | head -n 1)"
[[ -n "${package}" ]] || { echo "no package built" >&2; exit 1; }

echo "==> namcap on ${package##*/}"
namcap "${package}" | tee "${work}/namcap-package.txt"
if grep -q ' [EW]: ' "${work}/namcap-pkgbuild.txt" "${work}/namcap-package.txt"; then
    echo "namcap reported warnings or errors" >&2
    exit 1
fi

echo "==> package contents"
# The official image excludes man pages from extraction (NoExtract in
# pacman.conf), so the files are checked in the archive, not on disk.
bsdtar -tf "${package}" >"${work}/contents.txt"
for f in usr/bin/hyprtilt \
    usr/share/man/man1/hyprtilt.1.gz \
    usr/share/bash-completion/completions/hyprtilt \
    usr/share/zsh/site-functions/_hyprtilt \
    usr/share/fish/vendor_completions.d/hyprtilt.fish \
    "usr/share/licenses/${pkg}/LICENSE" \
    "usr/share/doc/${pkg}/README.md"; do
    grep -qx "$f" "${work}/contents.txt" || { echo "missing in package: $f" >&2; exit 1; }
done

echo "==> install and run"
sudo pacman -U --noconfirm "${package}"
hyprtilt --version
if strings /usr/bin/hyprtilt | grep -q "${work}"; then
    echo "the binary contains paths of the build directory" >&2
    exit 1
fi
echo "==> ${pkg}: OK"
