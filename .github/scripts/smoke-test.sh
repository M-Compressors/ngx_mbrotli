#!/bin/sh
# Tests the built modules in a distribution's own nginx: starts a container
# from the image (see container-test.sh) and checks from the host that both
# modules work. A response is compressed on the fly and decodes to the
# original, and a pre-compressed .br file is served as is. Then runs install.sh
# in a fresh container of the image.
#
# usage: smoke-test.sh <image> <dist-dir> [<label>]
#
# SETUP is passed to the container. The checks run on the host so that the
# image needs nothing but nginx: older images cannot install packages any more
# once their distribution's repositories are archived. Needs docker, curl, cmp
# and the brotli command-line tool.

set -eu

image=$1
dist=$(cd "$2" && pwd)
label=${3:-}
scripts=$(cd "$(dirname "$0")" && pwd)

html=$(mktemp -d)
i=0
while [ $i -lt 2000 ]; do
    echo "<p>Line $i of a page that ngx_mbrotli should compress.</p>"
    i=$((i + 1))
done >"$html/index.html"
echo 'body { color: red; }' >"$html/static.css"
brotli -c "$html/static.css" >"$html/static.css.br"
chmod -R a+rX "$html"

# Registries limit how fast an address can pull.
for i in 1 2 3 4 5; do
    docker pull -q "$image" >/dev/null && break
    sleep $((i * 10))
done

cid=$(docker run -d -p 127.0.0.1::8080 -e SETUP \
    -v "$scripts:/scripts:ro" -v "$dist:/dist:ro" -v "$html:/html:ro" \
    "$image" /scripts/container-test.sh /dist ${label:+"$label"})
trap 'docker rm -f "$cid" >/dev/null' EXIT

fail() {
    echo "FAIL: $*" >&2
    docker logs "$cid" >&2
    exit 1
}

# Installing nginx can take a while; stop waiting if the container exits.
port=
i=0
while [ $i -lt 600 ]; do
    if [ "$(docker inspect -f '{{.State.Running}}' "$cid")" != true ]; then
        docker logs "$cid"
        if [ "$(docker inspect -f '{{.State.ExitCode}}' "$cid")" = 0 ] &&
            docker logs "$cid" 2>&1 | grep -q '^::warning::'; then
            exit 0
        fi
        fail "nginx exited"
    fi
    port=$(docker port "$cid" 8080/tcp | head -1 | sed 's/.*://')
    if [ -n "$port" ] && curl -s -o /dev/null "http://127.0.0.1:$port/"; then
        break
    fi
    sleep 1
    i=$((i + 1))
done
[ $i -lt 600 ] || fail "nginx did not start"
docker logs "$cid" 2>&1 | grep -E '^(testing|nginx version)'

out=$(mktemp -d)
curl -sS -H 'Accept-Encoding: br' -D "$out/headers" -o "$out/body.br" \
    "http://127.0.0.1:$port/index.html"
grep -qi '^content-encoding: br' "$out/headers" || fail "index.html is not compressed"
brotli -dc "$out/body.br" >"$out/body" || fail "index.html does not decode"
cmp "$out/body" "$html/index.html" || fail "index.html decodes to different bytes"
echo "on the fly: $(wc -c <"$html/index.html") -> $(wc -c <"$out/body.br") bytes"

curl -sS -H 'Accept-Encoding: br' -D "$out/headers" -o "$out/static.br" \
    "http://127.0.0.1:$port/static.css"
grep -qi '^content-encoding: br' "$out/headers" || fail "static.css.br is not served"
cmp "$out/static.br" "$html/static.css.br" || fail "static.css.br differs"
echo "static: ok"

if docker logs "$cid" 2>&1 | grep -E '\[(alert|emerg|crit)\]'; then
    fail "errors in the log"
fi
echo "serve: PASS"

# install.sh in a fresh container, from the archives and their SHA256SUMS as a
# release has them.
release=$(mktemp -d)
cp "$dist"/*.tar.gz "$release/"
(
    cd "$release"
    if command -v sha256sum >/dev/null; then
        sha256sum -- *.tar.gz
    else
        shasum -a 256 -- *.tar.gz
    fi
) >"$release/SHA256SUMS"
docker run --rm -e SETUP -e MODE=install \
    -v "$scripts:/scripts:ro" -v "$scripts/../../install.sh:/install.sh:ro" \
    -v "$release:/release:ro" \
    "$image" /scripts/container-test.sh /release
echo PASS
