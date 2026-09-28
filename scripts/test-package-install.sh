#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-only
# Copyright (C) 2026 Mateusz Okulanis <FPGArtktic@outlook.com>
#
# Install a built .deb or .rpm in a clean container of the distribution and
# check it: the binary runs, the man page is found by man, and the shell
# completions are where the shells look for them. Runs as root inside the
# container; "just pkg-test" starts one container per distribution.
#
# Usage: scripts/test-package-install.sh deb|rpm PACKAGE-FILE

set -euo pipefail

kind="$1"
package="$2"

case "$kind" in
deb)
    # Minimal Debian and Ubuntu images exclude documentation from
    # installation; man pages must be installed for this test.
    rm -f /etc/dpkg/dpkg.cfg.d/excludes
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq man-db >/dev/null
    apt-get install -y -qq "$package" >/dev/null
    zsh_dir=/usr/share/zsh/vendor-completions
    ;;
rpm)
    sed -i '/^tsflags=nodocs/d' /etc/dnf/dnf.conf 2>/dev/null || true
    dnf install -y -q man-db >/dev/null
    dnf install -y -q "$package" >/dev/null
    zsh_dir=/usr/share/zsh/site-functions
    ;;
*)
    echo "usage: $0 deb|rpm PACKAGE-FILE" >&2
    exit 2
    ;;
esac

hyprtilt --version
man -w hyprtilt
for f in /usr/bin/hyprtilt \
    /usr/share/bash-completion/completions/hyprtilt \
    "${zsh_dir}/_hyprtilt" \
    /usr/share/fish/vendor_completions.d/hyprtilt.fish; do
    test -f "$f" || { echo "missing: $f" >&2; exit 1; }
done
# shellcheck source=/dev/null
echo "==> $(. /etc/os-release && echo "$PRETTY_NAME"): OK"
