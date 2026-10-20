#!/bin/sh
# Tests the package of one Publish target in a clean container of the
# target's distribution, next to the nginx it was built for:
#
#   1. the package installs, and nginx -t passes; nginx-org packages are
#      loaded with the load_module line their banner gives, distro packages
#      must have enabled themselves;
#   2. nginx with `brotli on` serves a static file brotli-compressed, and it
#      decompresses to the original; again on a thread pool when nginx has
#      threads. Each time, `brotli_static on` serves a pre-compressed .br
#      file as it is;
#   3. the package purges cleanly, leaving nginx installed and working.
#
# usage: package-test.sh <dist-dir>
#
# The target comes from the environment, as for package-build.sh.

set -eu

dist=$1
scripts=$(cd "$(dirname "$0")" && pwd)

export EXTRA_PACKAGES=brotli
# shellcheck source-path=SCRIPTDIR
. "$scripts/install-nginx.sh"
command -v curl >/dev/null || {
    if [ "$PKG_FORMAT" = deb ]; then apt_get install curl ca-certificates >/dev/null; else dnf_ install curl >/dev/null; fi
}

package=$(ls "$dist"/*."$PKG_FORMAT")
[ "$(echo "$package" | wc -l)" = 1 ] || {
    echo "expected one package in $dist: $package" >&2
    exit 1
}
echo "testing $(basename "$package")"

fail() {
    echo "FAIL: $*" >&2
    if [ -e /tmp/brotli-test/error.log ]; then
        cat /tmp/brotli-test/error.log >&2
    fi
    exit 1
}

if [ "$PKG_FORMAT" = deb ]; then
    name=$(dpkg-deb -f "$package" Package)
    apt_get install "$package"
else
    name=$(rpm -qp --qf '%{NAME}' "$package")
    dnf_ install "$package"
fi
modules_path=$(nginx -V 2>&1 | sed -n 's/.*--modules-path=\([^ ]*\).*/\1/p')
[ -f "$modules_path/ngx_mbrotli.so" ] || fail "$modules_path/ngx_mbrotli.so is not installed"

# How the test configuration loads the module.
case $GROUP-$PKG_FORMAT in
nginx-org-*)
    load='load_module modules/ngx_mbrotli.so;'
    ;;
distro-deb)
    [ -L /etc/nginx/modules-enabled/50-mod-http-mbrotli.conf ] ||
        fail "the package did not enable the module in /etc/nginx/modules-enabled"
    load='include /etc/nginx/modules-enabled/*.conf;'
    ;;
distro-rpm)
    load='include /usr/share/nginx/modules/*.conf;'
    ;;
esac

# The distribution's own configuration must keep working, and for distro
# packages it loads the module itself.
nginx -t
if [ "$GROUP" = distro ]; then
    nginx -T 2>/dev/null | grep -q "^load_module .*ngx_mbrotli\.so" ||
        fail "nginx.conf does not load the module"
fi

dir=/tmp/brotli-test
mkdir -p "$dir/html"
i=0
while [ $i -lt 2000 ]; do
    echo "Line $i of a file that ngx_mbrotli should compress."
    i=$((i + 1))
done >"$dir/html/file.txt"
# Only static/ has a .br file next to the original.
mkdir -p "$dir/html/static"
cp "$dir/html/file.txt" "$dir/html/static/file.txt"
brotli -c "$dir/html/file.txt" >"$dir/html/static/file.txt.br"
chmod -R a+rX "$dir"

# Runs nginx with the module and checks a response compressed on the fly and a
# pre-compressed one. Extra http directives are passed as $1.
serve() {
    cat >"$dir/nginx.conf" <<EOF
$load

user root;
pid $dir/nginx.pid;
error_log $dir/error.log info;

events {
}

http {
    types {
        text/plain txt;
    }
    access_log off;
    client_body_temp_path $dir/client_body;
    proxy_temp_path       $dir/proxy;
    fastcgi_temp_path     $dir/fastcgi;
    uwsgi_temp_path       $dir/uwsgi;
    scgi_temp_path        $dir/scgi;

    brotli on;
    brotli_types text/plain;
    brotli_static on;
    $1

    server {
        listen 127.0.0.1:8080;
        root $dir/html;
    }
}
EOF
    nginx -t -c "$dir/nginx.conf" || fail "nginx -t with ${1:-the module}"
    nginx -c "$dir/nginx.conf" || fail "nginx does not start with ${1:-the module}"
    i=0
    until curl -s -o /dev/null "http://127.0.0.1:8080/"; do
        i=$((i + 1))
        [ $i -lt 50 ] || fail "nginx does not answer"
        sleep 0.2
    done

    curl -sS -H 'Accept-Encoding: br' -D "$dir/headers" -o "$dir/body.br" \
        "http://127.0.0.1:8080/file.txt"
    curl -sS -H 'Accept-Encoding: br' -D "$dir/static-headers" -o "$dir/static-body.br" \
        "http://127.0.0.1:8080/static/file.txt"
    nginx -c "$dir/nginx.conf" -s stop
    i=0
    while [ -e "$dir/nginx.pid" ]; do
        i=$((i + 1))
        [ $i -lt 50 ] || fail "nginx does not stop"
        sleep 0.2
    done

    grep -qi '^content-encoding: br' "$dir/headers" || {
        cat "$dir/headers" >&2
        fail "the response is not brotli-compressed${1:+ with $1}"
    }
    brotli -d -c "$dir/body.br" >"$dir/body" || fail "the response does not decompress"
    [ "$(sha256sum <"$dir/body")" = "$(sha256sum <"$dir/html/file.txt")" ] ||
        fail "the response decompresses to different bytes"
    grep -qi '^content-encoding: br' "$dir/static-headers" || {
        cat "$dir/static-headers" >&2
        fail "brotli_static does not serve the .br file${1:+ with $1}"
    }
    [ "$(sha256sum <"$dir/static-body.br")" = "$(sha256sum <"$dir/html/static/file.txt.br")" ] ||
        fail "brotli_static serves different bytes than the .br file"
    if grep -E '\[(alert|emerg|crit)\]' "$dir/error.log"; then
        fail "errors in the log"
    fi
    echo "${1:-brotli on}: $(wc -c <"$dir/html/file.txt") -> $(wc -c <"$dir/body.br") bytes"
}

serve ""
if nginx -V 2>&1 | grep -q -- '--with-threads'; then
    serve 'brotli_thread_pool default;'
fi

# Removal leaves nginx as it was.
if [ "$PKG_FORMAT" = deb ]; then
    apt-get -y -qq purge "$name"
    dpkg-query -W -f='${Status}\n' nginx | grep -q 'install ok installed' ||
        fail "purging $name removed nginx"
    leftovers=$(ls /etc/nginx/modules-enabled/*mbrotli* /usr/share/nginx/modules-available/*mbrotli* 2>/dev/null || true)
else
    dnf -y -q remove "$name"
    rpm -q nginx >/dev/null || fail "removing $name removed nginx"
    leftovers=$(ls /usr/share/nginx/modules/*mbrotli* 2>/dev/null || true)
fi
[ ! -e "$modules_path/ngx_mbrotli.so" ] || fail "ngx_mbrotli.so is left after removal"
[ -z "$leftovers" ] || fail "left after removal: $leftovers"
nginx -t || fail "nginx -t fails after removal"
echo PASS
