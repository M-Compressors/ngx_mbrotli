#!/bin/sh
# Builds the module for one NGINX release and packs it for a GitHub Release.
#
# usage: build-module.sh <nginx-version> <out-dir> [<label>]
#
# NGINX loads a dynamic module only if it was built for exactly the same
# version and "signature", a string of flags that `configure` derives from its
# arguments and the platform.
#
# Without a label, NGINX is configured with the arguments of the nginx.org
# packages. With --with-compat, which the nginx.org, Debian, Ubuntu, RHEL,
# Fedora and Amazon Linux packages all use, every build of the same version has
# the same signature, so one module serves all of them.
#
# With a label (for example alpine3.22), NGINX is configured with the arguments
# of the nginx binary on PATH (`nginx -V`), for distributions that build NGINX
# without --with-compat. Paths, compiler and linker options and dynamic modules
# are dropped, and the macros that dynamic modules add to the build are defined
# by hand (see below); the build fails unless its signature then matches the
# installed nginx's.
#
# The archive is named ngx_mbrotli-<module>-nginx<version>[-<label>]-<target>,
# for example ngx_mbrotli-0.3.0-nginx1.30.5-x86_64-linux-gnu.tar.gz.

set -eu

version=$1
out=$(mkdir -p "$2" && cd "$2" && pwd)
label=${3:-}

# MODULE_DIR builds another checkout, such as an earlier release, with these
# scripts.
repo=${MODULE_DIR:-$(cd "$(dirname "$0")/../.." && pwd)}
work=${TMPDIR:-/tmp}/ngx_mbrotli-build
mkdir -p "$work"

if [ -z "$label" ]; then
    args='--with-compat --with-file-aio --with-threads
        --with-http_addition_module --with-http_auth_request_module
        --with-http_dav_module --with-http_flv_module --with-http_gunzip_module
        --with-http_gzip_static_module --with-http_mp4_module
        --with-http_random_index_module --with-http_realip_module
        --with-http_secure_link_module --with-http_slice_module
        --with-http_ssl_module --with-http_stub_status_module
        --with-http_sub_module --with-http_v2_module --with-http_v3_module
        --with-mail --with-mail_ssl_module --with-stream
        --with-stream_realip_module --with-stream_ssl_module
        --with-stream_ssl_preread_module'
else
    installed=$(nginx -v 2>&1 | sed -n 's|^nginx version: nginx/\([0-9.]*\).*|\1|p')
    if [ "$installed" != "$version" ]; then
        echo "installed nginx is $installed, not $version" >&2
        exit 1
    fi
    # Quoted --with-cc-opt='...' values are removed first, so that splitting
    # the rest on spaces is safe.
    args=$(nginx -V 2>&1 | sed -n 's/^configure arguments: //p' |
        sed "s/--with-[a-z]*-opt='[^']*'//g" | tr ' ' '\n' |
        grep -E '^--(with|without)-' |
        grep -vE '=dynamic$|^--with-[a-z]*-opt=|^--with-perl_modules_path=')
fi

src=$work/nginx-$version
rm -rf "$src"
curl -sSfL "https://nginx.org/download/nginx-$version.tar.gz" | tar xz -C "$work"

# Distributions patch NGINX, and a patch can change the layout of a structure
# without changing the signature (Alpine 3.21's fix for CVE-2026-49975 adds a
# field to ngx_http_headers_in_t). NGINX_PATCHES names a directory with the
# distribution's patches and a `series` file listing them in order.
if [ -n "${NGINX_PATCHES:-}" ]; then
    while read -r patch; do
        echo "applying $patch"
        patch -s -p1 -d "$src" <"$NGINX_PATCHES/$patch"
    done <"$NGINX_PATCHES/series"
fi

# Older releases lack some of the nginx.org arguments (--with-http_v3_module
# appeared in 1.25.0); those modules did not exist then, so skip them.
supported=
for arg in $args; do
    if grep -q -- "${arg%%=*}[)=]" "$src/auto/options"; then
        supported="$supported $arg"
    else
        echo "nginx $version does not support $arg, skipping" >&2
    fi
done

echo "configure$supported"
# shellcheck disable=SC2086
(cd "$src" && ./configure $supported >"$work/configure.log" 2>&1) || {
    tail -50 "$work/configure.log" >&2
    exit 1
}

# Prints the signature that modules built against this configuration get.
signature() {
    cat >"$work/signature.c" <<'EOF'
#include <ngx_config.h>
#include <ngx_core.h>

int
main(void)
{
    puts(NGX_MODULE_SIGNATURE);
    return 0;
}
EOF
    cc -o "$work/signature" "$work/signature.c" -I "$src/objs" \
        -I "$src/src/core" -I "$src/src/event" -I "$src/src/event/modules" \
        -I "$src/src/event/quic" -I "$src/src/os/unix"
    "$work/signature"
}

if [ -n "$label" ]; then
    # The configuration of a third-party module, even a dynamic one, can
    # define macros for the whole build (headers-more defines NGX_HTTP_HEADERS),
    # which change both the signature and the layout of structures such as
    # ngx_http_headers_in_t. Define those that the installed nginx has and this
    # build lacks, as auto/have does, by their position in the signature.
    want=$(grep -ao '[0-9],[0-9],[0-9],[01]\{30,\}' "$(command -v nginx)" | head -1)
    have=$(signature)
    for bit in 26:NGX_HTTP_GZIP 28:NGX_HTTP_X_FORWARDED_FOR 29:NGX_HTTP_REALIP \
        30:NGX_HTTP_HEADERS 31:NGX_HTTP_DAV 32:NGX_HTTP_CACHE \
        33:NGX_HTTP_UPSTREAM_ZONE; do
        pos=${bit%%:*}
        if [ "$(echo "${want##*,}" | cut -c"$pos")" = 1 ] &&
            [ "$(echo "${have##*,}" | cut -c"$pos")" = 0 ]; then
            echo "defining ${bit#*:}, as the installed nginx does"
            printf '#ifndef %s\n#define %s  1\n#endif\n\n' "${bit#*:}" "${bit#*:}" \
                >>"$src/objs/ngx_auto_config.h"
        fi
    done
    have=$(signature)
    if [ "$have" != "$want" ]; then
        echo "signature $have does not match the installed nginx's $want" >&2
        exit 1
    fi
fi

cd "$repo"
NGINX_SOURCE_DIR=$src NGINX_BUILD_DIR=$src/objs cargo build --release --locked

module=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
if ldd --version 2>&1 | grep -q musl; then libc=musl; else libc=gnu; fi
name=ngx_mbrotli-$module-nginx$version${label:+-$label}-$(uname -m)-linux-$libc

stage=$work/$name
rm -rf "$stage"
mkdir -p "$stage"
cp "${CARGO_TARGET_DIR:-target}/release/libngx_mbrotli.so" "$stage/ngx_mbrotli.so"
cp LICENSE "$stage/"
# What the module was built against. When NGINX refuses to load it, compare
# with `nginx -v` and the signature in the binary:
#   grep -ao '[0-9],[0-9],[0-9],[01]\{30,\}' "$(command -v nginx)"
{
    echo "nginx $version"
    echo "signature $(grep -ao '[0-9],[0-9],[0-9],[01]\{30,\}' "$stage/ngx_mbrotli.so" | head -1)"
    echo "configure$supported"
} >"$stage/BUILDINFO"

tar -czf "$out/$name.tar.gz" -C "$work" "$name"
echo "$out/$name.tar.gz"
