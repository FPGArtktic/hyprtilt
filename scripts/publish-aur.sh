#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
#
# Publish the AUR recipes of a release:
#
#   * hyprtilt with the release's version and the checksum of the source
#     archive GitHub serves for its tag;
#   * hyprtilt-git with its recipe from the tag and the version its
#     pkgver() prints at the tag.
#
# A recipe whose content did not change is not pushed. The release
# workflow runs this in the build image (job "aur"), only for a final
# release and only when the AUR_SSH_KEY secret is set.
#
# Environment:
#   AUR_SSH_KEY    private SSH key of the AUR account
#   AUR_HOST_KEY   known_hosts line(s) of aur.archlinux.org; compare the
#                  output of "ssh-keyscan aur.archlinux.org" with the
#                  fingerprints on https://aur.archlinux.org before setting
#                  it. Without it the script refuses to connect.
#
# Usage: scripts/publish-aur.sh <version>

set -euo pipefail

version="${1:?usage: scripts/publish-aur.sh <version>}"
[[ -n "${AUR_SSH_KEY:-}" ]] || { echo "AUR_SSH_KEY is not set" >&2; exit 1; }
[[ -n "${AUR_HOST_KEY:-}" ]] || { echo "AUR_HOST_KEY is not set" >&2; exit 1; }
[[ "${version}" != *-* ]] || { echo "not publishing pre-release ${version}" >&2; exit 1; }

root="$(git rev-parse --show-toplevel)"
url="https://github.com/FPGArtktic/hyprtilt"
tag="v${version}"
work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT
chmod 755 "${work}"

install -m 600 /dev/null "${work}/key"
printf '%s\n' "${AUR_SSH_KEY}" >"${work}/key"
printf '%s\n' "${AUR_HOST_KEY}" >"${work}/known_hosts"
export GIT_SSH_COMMAND="ssh -i ${work}/key -o IdentitiesOnly=yes -o UserKnownHostsFile=${work}/known_hosts -o StrictHostKeyChecking=yes"
export GIT_AUTHOR_NAME="Mateusz Okulanis" GIT_AUTHOR_EMAIL="FPGArtktic@outlook.com"
export GIT_COMMITTER_NAME="${GIT_AUTHOR_NAME}" GIT_COMMITTER_EMAIL="${GIT_AUTHOR_EMAIL}"

# Clone the AUR repository of a package and put the recipe of the tag in it.
checkout()
{
    local pkg="$1"

    git clone --quiet "ssh://aur@aur.archlinux.org/${pkg}.git" "${work}/${pkg}"
    for f in PKGBUILD LICENSE; do
        git -C "${root}" show "${tag}:packaging/aur/${pkg}/${f}" >"${work}/${pkg}/${f}"
    done
}

# Regenerate .SRCINFO, then commit and push when anything changed. makepkg
# refuses to run as root, so it runs as the build image's user then.
push()
{
    local pkg="$1" message="$2" dir="${work}/$1"

    chmod -R a+rX "${dir}"
    if [[ "$(id -u)" -eq 0 ]]; then
        (cd "${dir}" && runuser -u builder -- makepkg --printsrcinfo) >"${work}/srcinfo"
    else
        (cd "${dir}" && makepkg --printsrcinfo) >"${work}/srcinfo"
    fi
    mv "${work}/srcinfo" "${dir}/.SRCINFO"
    git -C "${dir}" add PKGBUILD .SRCINFO LICENSE
    if git -C "${dir}" diff --cached --quiet; then
        echo "==> ${pkg}: unchanged"
        return
    fi
    git -C "${dir}" commit --quiet -m "${message}"
    git -C "${dir}" push --quiet origin HEAD:master
    echo "==> ${pkg}: published"
}

echo "==> hyprtilt ${version}"
checkout hyprtilt
sum="$(curl -fsSL --retry 3 "${url}/archive/${tag}.tar.gz" | sha256sum | cut -d ' ' -f 1)"
sed -i -e "s/^pkgver=.*/pkgver=${version}/" -e "s/^pkgrel=.*/pkgrel=1/" \
    -e "s/^sha256sums=.*/sha256sums=('${sum}')/" "${work}/hyprtilt/PKGBUILD"
push hyprtilt "Update to ${version}"

echo "==> hyprtilt-git"
checkout hyprtilt-git
commit="$(git -C "${root}" rev-parse --short=7 "${tag}^{commit}")"
sed -i -e "s/^pkgver=.*/pkgver=${version}.r0.g${commit}/" -e "s/^pkgrel=.*/pkgrel=1/" \
    "${work}/hyprtilt-git/PKGBUILD"
push hyprtilt-git "Update the recipe to ${version}"
