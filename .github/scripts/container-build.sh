#!/bin/sh
# Runs inside a build container (AlmaLinux or Alpine): installs the toolchain,
# and with a label also the distribution's nginx and its patches, then builds
# the module.
#
# usage: container-build.sh <nginx-version> <out-dir> [<label>]
#
# Builds for glibc run on AlmaLinux 8, so that the module needs no newer glibc
# than 2.28 (RHEL 8, Debian 10, Ubuntu 18.10). Builds for musl link libgcc_s
# dynamically: nginx's Alpine images ship it, plain Alpine needs `apk add libgcc`.
# The only label supported is alpine<release>, for example alpine3.22.

set -eu

version=$1
label=${3:-}

if command -v apk >/dev/null; then
    apk add -q bash curl gcc make musl-dev linux-headers patch pcre-dev \
        pcre2-dev zlib-dev openssl-dev clang-dev
    # Rust links musl statically by default, which rules out a cdylib.
    export RUSTFLAGS="-C target-feature=-crt-static"
else
    dnf install -y -q --setopt=install_weak_deps=False gcc make pcre2-devel \
        zlib-devel openssl-devel clang-devel
fi

case $label in
'') ;;
alpine*)
    apk add -q nginx
    # The patches that Alpine applies to the NGINX sources: those in the
    # APKBUILD's source list, in order, except the ones named module~fix.patch,
    # which apply to third-party modules.
    aports=https://gitlab.alpinelinux.org/alpine/aports/-/raw/${label#alpine}-stable/main/nginx
    NGINX_PATCHES=$(mktemp -d)
    export NGINX_PATCHES
    curl -sSfL "$aports/APKBUILD" -o "$NGINX_PATCHES/APKBUILD"
    pkgver=$(sed -n 's/^pkgver=//p' "$NGINX_PATCHES/APKBUILD")
    if [ "$pkgver" != "$version" ]; then
        echo "aports packages nginx $pkgver, not $version" >&2
        exit 1
    fi
    sed -n '/^source="/,/"/p' "$NGINX_PATCHES/APKBUILD" | tr -d '\t"' |
        grep -E '^[A-Za-z0-9._-]+\.patch$' >"$NGINX_PATCHES/series" || true
    while read -r patch; do
        curl -sSfL "$aports/$patch" -o "$NGINX_PATCHES/$patch"
    done <"$NGINX_PATCHES/series"
    ;;
*)
    echo "unknown label: $label" >&2
    exit 1
    ;;
esac

if ! command -v cargo >/dev/null; then
    curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
fi

exec "$(dirname "$0")/build-module.sh" "$@"
