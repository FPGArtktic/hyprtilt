#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
#
# Check that a release tag may be released:
#
#   * it is an annotated tag named v<version>, where <version> is the
#     workspace version in Cargo.toml (SemVer, optionally -rc.N);
#   * it points at the checked-out commit;
#   * it carries a good signature from one of the keys in $RELEASE_TAG_KEYS:
#     OpenPGP public keys (ASCII armored) or SSH public keys, one per line.
#     The keys come from a repository variable, which only the repository
#     settings can change, unlike a key file in the tagged tree.
#
# Usage: RELEASE_TAG_KEYS=... scripts/verify-release-tag.sh TAG

set -euo pipefail

tag="${1:?usage: $0 TAG}"
ref="refs/tags/${tag}"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)"

fail() {
    echo "::error::$*" >&2
    exit 1
}

[[ "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-rc\.[0-9]+)?$ ]] || fail "tag ${tag} is not v<major>.<minor>.<patch>[-rc.N]"
[[ "${tag}" == "v${version}" ]] || fail "tag ${tag} does not match the version ${version} in Cargo.toml"
[[ "$(git cat-file -t "${ref}" 2>/dev/null)" == "tag" ]] || fail "${tag} is not an annotated tag"
[[ "$(git rev-parse "${ref}^{commit}")" == "$(git rev-parse HEAD)" ]] || fail "${tag} does not point at HEAD"
[[ -n "${RELEASE_TAG_KEYS:-}" ]] || fail "the repository variable RELEASE_TAG_KEYS (keys allowed to sign release tags) is not set"

home="$(mktemp -d)"
trap 'rm -rf "${home}"' EXIT
if grep -q 'BEGIN PGP PUBLIC KEY BLOCK' <<<"${RELEASE_TAG_KEYS}"; then
    export GNUPGHOME="${home}/gnupg"
    mkdir -m 0700 "${GNUPGHOME}"
    gpg --batch --quiet --import <<<"${RELEASE_TAG_KEYS}"
    git verify-tag "${tag}"
else
    # SSH keys: every signer may sign tags.
    sed -n 's/^\(ssh-\|ecdsa-\|sk-\)/* \1/p' <<<"${RELEASE_TAG_KEYS}" >"${home}/allowed_signers"
    [[ -s "${home}/allowed_signers" ]] || fail "RELEASE_TAG_KEYS holds neither OpenPGP nor SSH public keys"
    git -c gpg.format=ssh -c gpg.ssh.allowedSignersFile="${home}/allowed_signers" verify-tag "${tag}"
fi
echo "tag ${tag}: annotated, at HEAD, version ${version}, good signature"
