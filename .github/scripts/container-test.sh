#!/bin/sh
# Runs inside a distribution's container for smoke-test.sh. Installs its nginx
# if the image has none, then, depending on MODE:
#
#   serve    (default) picks the module built for that nginx version and runs
#            nginx in the foreground with it, serving /html on port 8080;
#   install  runs /install.sh against the archives in <dist-dir>, as a user
#            would, and checks that nginx loads the module it installed.
#
# usage: container-test.sh <dist-dir> [<label>]
#
# SETUP, if set, is a shell command run first, for example to enable a module
# stream (`dnf module enable -y nginx:1.24`).

set -eu

dist=$1
label=${2:-}

eval "${SETUP:-}"

if ! command -v nginx >/dev/null; then
    if command -v apk >/dev/null; then
        apk add -q nginx
    elif command -v apt-get >/dev/null; then
        apt-get update -qq
        DEBIAN_FRONTEND=noninteractive apt-get install -y -qq --no-install-recommends nginx >/dev/null
    else
        "$(command -v dnf || command -v yum)" install -y -q nginx
    fi
fi

if [ "${MODE:-serve}" = install ]; then
    # The installer finds the archive itself, installs what it lacks (tar,
    # libgcc) and makes nginx.conf load the module.
    out=$(NGX_MBROTLI_BASE_URL=$dist sh /install.sh)
    echo "$out"
    path=$(echo "$out" | sed -n 's/^installed //p')

    nginx -T 2>/dev/null | grep -q "load_module $path;" ||
        { echo "nginx.conf does not load $path" >&2; exit 1; }
    nginx -t
    # Running it again must leave the configuration as it is.
    NGX_MBROTLI_BASE_URL=$dist sh /install.sh | grep -q 'already loads'
    echo PASS
    exit 0
fi

# Rust's musl target links libgcc_s for unwinding; Amazon Linux's image has no
# tar.
if command -v apk >/dev/null; then
    apk add -q libgcc
fi
if ! command -v tar >/dev/null; then
    "$(command -v dnf || command -v yum)" install -y -q tar gzip
fi

version=$(nginx -v 2>&1 | sed -n 's|^nginx version: nginx/\([0-9.]*\).*|\1|p')
if ldd --version 2>&1 | grep -q musl; then libc=musl; else libc=gnu; fi
name=nginx$version${label:+-$label}-$(uname -m)-linux-$libc

archive=
for f in "$dist"/ngx_mbrotli-*-"$name".tar.gz; do
    if [ -e "$f" ]; then archive=$f; fi
done
# Distributions update their nginx between module releases; that is a reason
# to build for the new version, not a failure of this build.
if [ -z "$archive" ]; then
    echo "::warning::no module built for $name; add nginx $version to the build matrix"
    exit 0
fi
echo "testing $(basename "$archive")"

dir=$(mktemp -d)
mkdir "$dir/logs"
tar -xzf "$archive" -C "$dir"
cat >"$dir/nginx.conf" <<EOF
load_module $(echo "$dir"/*/ngx_mbrotli.so);

user root;
daemon off;
pid $dir/nginx.pid;
error_log stderr info;

events {
}

http {
    types {
        text/html html;
        text/css  css;
    }

    client_body_temp_path $dir/client_body;
    proxy_temp_path       $dir/proxy;
    fastcgi_temp_path     $dir/fastcgi;
    uwsgi_temp_path       $dir/uwsgi;
    scgi_temp_path        $dir/scgi;
    access_log off;

    server {
        listen 8080;
        root /html;

        brotli on;

        location /static.css {
            brotli_static on;
        }
    }
}
EOF

nginx -V 2>&1 | head -1
nginx -p "$dir" -c "$dir/nginx.conf" -t
exec nginx -p "$dir" -c "$dir/nginx.conf"
