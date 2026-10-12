#!/bin/sh
# Installs the prebuilt ngx_mbrotli module for the nginx on this machine.
#
#   curl -fsSL https://raw.githubusercontent.com/M-Compressors/ngx_mbrotli/main/install.sh | sh
#
# It finds the release archive built for this nginx version, architecture and C
# library, checks it against the release's SHA256SUMS and the module signature
# of the nginx binary, and copies the module into nginx's modules directory.
# Then it loads the module: from the directory of module configurations that
# nginx.conf includes (Debian, Ubuntu, RHEL, Fedora, Alpine), or else from the
# first line of nginx.conf, which it backs up first. If `nginx -t` fails with
# the module loaded, the configuration is restored.
#
# Environment:
#   NGX_MBROTLI_VERSION    release tag to install (default: the latest release)
#   NGX_MBROTLI_NO_CONFIG  set to 1 to install the module only
#   NGINX                  nginx binary (default: nginx on PATH, /usr/sbin/nginx)
#   NGX_MBROTLI_BASE_URL   where the archives and SHA256SUMS are, a URL or a
#                          local directory (default: the GitHub release)

set -eu

repo=M-Compressors/ngx_mbrotli

say() { printf '%s\n' "$*"; }
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

fetch() {
    case $1 in
    /*) cp "$1" "$2" ;;
    *)
        if command -v curl >/dev/null; then
            curl -fsSL --retry 3 -o "$2" "$1"
        elif command -v wget >/dev/null; then
            wget -q -O "$2" "$1"
        else
            die "curl or wget is required"
        fi
        ;;
    esac
}

sha256() {
    if command -v sha256sum >/dev/null; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null; then
        shasum -a 256 "$1" | cut -d' ' -f1
    elif command -v openssl >/dev/null; then
        openssl dgst -sha256 -r "$1" | cut -d' ' -f1
    else
        die "sha256sum, shasum or openssl is required to verify the download"
    fi
}

as_root() {
    if [ "$(id -u)" = 0 ]; then
        "$@"
    elif command -v sudo >/dev/null; then
        sudo "$@"
    else
        die "run as root: $*"
    fi
}

# Installs a missing command from the distribution's packages; minimal images
# such as Amazon Linux's lack tar.
need() {
    command -v "$1" >/dev/null && return
    say "installing $2"
    if command -v apk >/dev/null; then
        as_root apk add -q "$2"
    elif command -v apt-get >/dev/null; then
        as_root apt-get update -qq
        as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "$2" >/dev/null
    elif command -v dnf >/dev/null; then
        as_root dnf install -y -q "$2"
    elif command -v yum >/dev/null; then
        as_root yum install -y -q "$2"
    else
        die "$1 is required"
    fi
}

signature() {
    LC_ALL=C grep -ao '[0-9],[0-9],[0-9],[01]\{30,\}' "$1" 2>/dev/null | head -1 || true
}

# What nginx is this?

nginx=${NGINX:-$(command -v nginx || echo /usr/sbin/nginx)}
[ -x "$nginx" ] || die "nginx not found; set NGINX to the nginx binary"

version=$("$nginx" -v 2>&1 | sed -n 's|^nginx version: nginx/\([0-9.]*\).*|\1|p')
[ -n "$version" ] || die "$("$nginx" -v 2>&1) is not supported, only nginx is"
configure=$("$nginx" -V 2>&1 | sed -n 's/^configure arguments: //p')

option() {
    printf '%s\n' "$configure" | tr ' ' '\n' | sed -n "s|^--$1=||p" | head -1
}

[ "$(uname -s)" = Linux ] || die "prebuilt modules are for Linux; build it from source:
https://github.com/$repo#building"

case $(uname -m) in
x86_64 | amd64) arch=x86_64 ;;
aarch64 | arm64) arch=aarch64 ;;
*) die "no prebuilt module for $(uname -m); build it from source" ;;
esac

if [ -e "/lib/ld-musl-$arch.so.1" ]; then libc=musl; else libc=gnu; fi

# Alpine's own nginx gets a module built from its sources and arguments; the
# nginx.org packages, also used by the nginx:alpine images, use /etc/nginx as
# their prefix.
label=
if [ -f /etc/alpine-release ] && [ "$(option prefix)" != /etc/nginx ]; then
    label=alpine$(cut -d. -f1,2 /etc/alpine-release)
fi

suffix=-nginx$version${label:+-$label}-$arch-linux-$libc.tar.gz
say "nginx $version ($nginx), $arch-linux-$libc${label:+, $label}"

# Download and verify.

# GitHub redirects releases/latest/download/<file> to the latest release, so
# plain curl or wget finds it without the API. A release published between the
# two downloads would fail the hash check rather than install the wrong file.
tag=${NGX_MBROTLI_VERSION:-}
if [ -n "${NGX_MBROTLI_BASE_URL:-}" ]; then
    base=$NGX_MBROTLI_BASE_URL
elif [ -n "$tag" ]; then
    base=https://github.com/$repo/releases/download/$tag
else
    base=https://github.com/$repo/releases/latest/download
fi
release=${tag:-latest}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fetch "$base/SHA256SUMS" "$work/SHA256SUMS" ||
    die "release $release has no SHA256SUMS ($base); it may have no prebuilt modules"

line=$(awk -v s="$suffix" 'substr($2, length($2) - length(s) + 1) == s' "$work/SHA256SUMS" | head -1)
if [ -z "$line" ]; then
    others=$(sed -n "s/.*-nginx\([0-9.]*\)${label:+-$label}-$arch-linux-$libc\.tar\.gz$/\1/p" \
        "$work/SHA256SUMS" | tr '\n' ' ')
    die "release $release has no module for nginx $version${label:+ on $label}
(built for nginx: ${others:-none}); build it from source:
https://github.com/$repo#building"
fi
hash=${line%% *}
archive=${line##* }

say "downloading $archive"
fetch "$base/$archive" "$work/$archive" || die "could not download $base/$archive"
actual=$(sha256 "$work/$archive")
[ "$actual" = "$hash" ] || die "SHA-256 of $archive is $actual, expected $hash"
say "SHA-256 verified: $hash"

need tar tar
need gzip gzip
tar -xzf "$work/$archive" -C "$work"
module=$work/${archive%.tar.gz}/ngx_mbrotli.so
[ -f "$module" ] || die "$archive has no ngx_mbrotli.so"

# The check nginx makes when it loads a module, made before installing it.
want=$(signature "$nginx")
have=$(signature "$module")
if [ -n "$want" ] && [ -n "$have" ] && [ "$want" != "$have" ]; then
    die "the module's signature $have differs from nginx's $want: this nginx was
configured differently from the nginx it was built for; build it from source:
https://github.com/$repo#building"
fi

if [ "$libc" = musl ] && ! ls /usr/lib/libgcc_s.so.1 /lib/libgcc_s.so.1 >/dev/null 2>&1; then
    say "installing libgcc, which the module needs"
    as_root apk add -q libgcc
fi

# The configuration as nginx reads it, includes and all.
config=
if [ "${NGX_MBROTLI_NO_CONFIG:-}" != 1 ]; then
    config=$(as_root "$nginx" -T 2>/dev/null || true)
fi

# google/ngx_brotli has modules with the same names; nginx cannot load both.
if brotli=$(printf '%s\n' "$config" | grep '^[[:space:]]*load_module[[:space:]].*brotli' |
    grep -v 'ngx_mbrotli\.so'); then
    die "nginx already loads google/ngx_brotli, which ngx_mbrotli replaces:
$brotli
Remove those lines (or the package that adds them, such as
libnginx-mod-http-brotli-filter or nginx-mod-http-brotli) and run this again."
fi

# Install.

prefix=$(option prefix)
prefix=${prefix:-/usr/local/nginx}
modules=$(option modules-path)
modules=${modules:-$prefix/modules}
as_root mkdir -p "$modules"
as_root cp "$module" "$modules/ngx_mbrotli.so.new"
as_root chmod 0644 "$modules/ngx_mbrotli.so.new"
as_root mv -f "$modules/ngx_mbrotli.so.new" "$modules/ngx_mbrotli.so"
say "installed $modules/ngx_mbrotli.so"

load="load_module $modules/ngx_mbrotli.so;"
reload="Reload nginx to use it (systemctl reload nginx, or nginx -s reload)."
upgrade="nginx refuses to start with a module built for another version: run
this script again whenever nginx is upgraded."

if [ "${NGX_MBROTLI_NO_CONFIG:-}" = 1 ]; then
    say "add to the top of nginx.conf: $load"
    say "$upgrade"
    exit 0
fi

if printf '%s\n' "$config" | grep -q '^[[:space:]]*load_module[[:space:]].*ngx_mbrotli\.so'; then
    say "nginx.conf already loads ngx_mbrotli."
    as_root "$nginx" -t
    say "$reload"
    say "$upgrade"
    exit 0
fi

# The directory of module configurations that nginx.conf includes, if any;
# otherwise the module is loaded from the first line of nginx.conf itself.
dir=$(printf '%s\n' "$config" |
    sed -n 's|^[[:space:]]*include[[:space:]]\{1,\}\([^;]*modules[^;]*\)/\*\.conf;.*|\1|p' | head -1)

if [ -n "$dir" ] && [ -d "$dir" ]; then
    conf=$dir/mod-mbrotli.conf
    printf '%s\n' "$load" | as_root tee "$conf" >/dev/null
    if ! as_root "$nginx" -t; then
        as_root rm -f "$conf"
        die "nginx -t failed with the module loaded; removed $conf"
    fi
    say "added $conf"
else
    conf=$(option conf-path)
    conf=${conf:-$prefix/conf/nginx.conf}
    [ -f "$conf" ] || die "$conf not found; add to the top of nginx.conf: $load"
    backup=$conf.before-mbrotli
    as_root cp -p "$conf" "$backup"
    { printf '%s\n' "$load"; as_root cat "$backup"; } >"$work/nginx.conf"
    # cp onto the existing file keeps its owner and mode.
    as_root cp "$work/nginx.conf" "$conf"
    if ! as_root "$nginx" -t; then
        as_root cp -p "$backup" "$conf"
        die "nginx -t failed with the module loaded; restored $conf"
    fi
    say "added \"$load\" to $conf (previous version: $backup)"
fi
say "$reload"
say "$upgrade"
