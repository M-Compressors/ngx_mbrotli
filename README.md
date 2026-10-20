# ngx_mbrotli

Brotli compression for NGINX, written in Rust with
[ngx-rust](https://github.com/nginx/ngx-rust) and the
[mbrotli](https://github.com/M-Compressors/mbrotli) encoder.

It is a drop-in replacement for [google/ngx_brotli](https://github.com/google/ngx_brotli):
the same two modules, directives, defaults and variable, so existing configurations
keep working unchanged.

- `ngx_http_brotli_filter_module` compresses responses on the fly.
- `ngx_http_brotli_static_module` serves pre-compressed `.br` files.

Both modules are built into one dynamic library, so one `load_module` line loads both.

Unlike google/ngx_brotli, it can also compress on NGINX thread pools out of the
box: one line, `brotli_thread_pool default;`, takes compression out of the
worker's event loop and spreads it over the pool's threads. In the
[benchmarks](#benchmarks), one worker with 4 threads serves 3.8–4.5 times as
many requests as google/ngx_brotli at levels 4–6 on 85 KB–2.2 MB responses, and
4.3–15 times as many at levels 9–11, while other requests keep the latency they
have with no compression at all.

## Contents

- [Example](#example)
- [Installation from packages](#installation-from-packages)
- [Building](#building)
- [Configuration directives](#configuration-directives)
  - [`brotli_static`](#brotli_static)
  - [`brotli`](#brotli)
  - [`brotli_types`](#brotli_types)
  - [`brotli_buffers`](#brotli_buffers)
  - [`brotli_comp_level`](#brotli_comp_level)
  - [`brotli_window`](#brotli_window)
  - [`brotli_min_length`](#brotli_min_length)
  - [`brotli_reuse_memory`](#brotli_reuse_memory)
  - [`brotli_retention_policy`](#brotli_retention_policy)
  - [`brotli_idle_encoders`](#brotli_idle_encoders)
  - [`brotli_thread_pool`](#brotli_thread_pool)
  - [`brotli_parallel_segment_size`](#brotli_parallel_segment_size)
  - [`brotli_parallel_tasks`](#brotli_parallel_tasks)
  - [`brotli_parallel_compressors`](#brotli_parallel_compressors)
- [Variables](#variables)
  - [`$brotli_ratio`](#brotli_ratio)
- [Sample configuration](#sample-configuration)
- [Differences from google/ngx_brotli](#differences-from-googlengx_brotli)
- [Benchmarks](#benchmarks)
  - [Requests per second](#requests-per-second)
  - [Latency of other requests](#latency-of-other-requests)
  - [A single large response](#a-single-large-response)
  - [Worker memory](#worker-memory)
  - [Segment size](#segment-size)
- [Implementation notes](#implementation-notes)
- [Testing](#testing)
- [Contributing and releases](#contributing-and-releases)
- [License](#license)

## Example

Compression on a thread pool, so that long responses and high levels do not hold
up the worker's event loop. NGINX must be built with `--with-threads`:

```nginx
load_module /path/to/libngx_mbrotli.so;

# The pool named `default` exists without being declared: 32 threads per worker
# process, shared with `aio threads`. To size compression on its own, declare
# a pool of its own and name it in `brotli_thread_pool`:
#
#     thread_pool brotli threads=4;

http {
    brotli on;
    brotli_comp_level 6;
    brotli_types text/plain text/css application/javascript application/json
                 image/svg+xml;
    brotli_static on;

    # Responses are cut into 1m segments; up to 4 of them per response are
    # compressed at once on the pool's threads.
    brotli_thread_pool default;
    brotli_parallel_segment_size 1m;
    brotli_parallel_tasks 4;

    server {
        listen 80;

        location / {
            root /var/www;
        }

        # Small API responses: compressing them in the event loop is cheaper.
        location /api/ {
            proxy_pass http://127.0.0.1:8080;
            brotli_thread_pool off;
        }
    }
}
```

Without `brotli_thread_pool` the module compresses in the event loop, as
google/ngx_brotli does. See [Configuration directives](#configuration-directives)
for every directive.

## Installation from packages

Prebuilt packages for amd64 and arm64 are published to
[Cloudsmith](https://cloudsmith.io/~m-compressors/repos/), each built and tested
against the exact nginx package it is for. Add the repository, then install the
package that matches your nginx:

| Your nginx | Repository | Package | Enabled on install |
|---|---|---|---|
| nginx.org stable | `ngx_mbrotli` | `nginx-module-mbrotli` | no |
| nginx.org mainline | `ngx_mbrotli-mainline` | `nginx-module-mbrotli` | no |
| Debian's or Ubuntu's own | `ngx_mbrotli` | `libnginx-mod-http-mbrotli` | yes |
| RHEL-compatible or Fedora's own | `ngx_mbrotli` | `nginx-mod-mbrotli` | yes |

Packages for nginx.org's nginx are built for every distribution release that
nginx.org currently packages (Debian, Ubuntu, RHEL 8 to 10 and compatible,
Amazon Linux 2023). Packages for the distributions' own nginx are built for
Debian, Ubuntu, RHEL-compatible (the default AppStream nginx) and the current
Fedora releases, where that nginx is 1.25.4 or newer, the oldest version the
module builds against: today Debian 13, Ubuntu 26.04, RHEL 10 and Fedora.

### Add the repository on Debian or Ubuntu:

```sh
curl -1sLf 'https://dl.cloudsmith.io/public/m-compressors/ngx_mbrotli/setup.deb.sh' | sudo -E bash
```

### On RHEL, CentOS, AlmaLinux, Rocky Linux, Amazon Linux or Fedora:

```sh
curl -1sLf 'https://dl.cloudsmith.io/public/m-compressors/ngx_mbrotli/setup.rpm.sh' | sudo -E bash
```

### For nginx.org mainline, add `ngx_mbrotli-mainline` instead:

```sh
curl -1sLf 'https://dl.cloudsmith.io/public/m-compressors/ngx_mbrotli-mainline/setup.deb.sh' | sudo -E bash
curl -1sLf 'https://dl.cloudsmith.io/public/m-compressors/ngx_mbrotli-mainline/setup.rpm.sh' | sudo -E bash
```

#### With nginx.org's nginx

```sh
sudo apt-get install nginx-module-mbrotli # Debian, Ubuntu
sudo dnf install nginx-module-mbrotli # RHEL-compatible, Fedora
```

Like nginx.org's own `nginx-module-*` packages, it installs the module without
enabling it. Add this line to the top of `/etc/nginx/nginx.conf` and reload
nginx:

```nginx
load_module modules/ngx_mbrotli.so;
```

The package requires the exact nginx package it was built for, so upgrade both
together. Its version is `<nginx version>+<module version>`, as with nginx.org's
module packages.

#### With the distribution's nginx

```sh
sudo apt-get install libnginx-mod-http-mbrotli   # Debian, Ubuntu
sudo dnf install nginx-mod-mbrotli               # RHEL-compatible, Fedora
```

As with the distribution's own module packages, the module is enabled on
installation: by `/etc/nginx/modules-enabled/50-mod-http-mbrotli.conf` on Debian
and Ubuntu, and `/usr/share/nginx/modules/mod-mbrotli.conf` on RHEL and Fedora.
Configure `brotli` (see [Configuration directives](#configuration-directives))
and reload nginx.

Both modules, `ngx_http_brotli_filter_module` and
`ngx_http_brotli_static_module`, come in the one `ngx_mbrotli.so`. The packages
conflict with those of google/ngx_brotli (`nginx-module-brotli`, and the
distribution's own where it has them, such as Debian's
`libnginx-mod-http-brotli-filter` and `libnginx-mod-http-brotli-static`), which
provide the same directives. They are built and published by the
[Publish workflow](.github/workflows/publish.yml).

Package repository hosting is graciously provided by [Cloudsmith](https://cloudsmith.com).

## Building

Requirements:

- Rust 1.89 or newer;
- libclang (used by `nginx-sys` to generate bindings);
- the NGINX source tree, configured with the same arguments as the NGINX binary
  that will load the module, plus `--with-compat`. Run `nginx -V` to see those
  arguments. Only `./configure` is required, not `make`.

```sh
cd nginx-1.30.5
./configure --with-compat <arguments from nginx -V>

cd /path/to/ngx_mbrotli
NGINX_BUILD_DIR=/path/to/nginx-1.30.5/objs cargo build --release
```

The module is written to `target/release/libngx_mbrotli.so`
(`libngx_mbrotli.dylib` on macOS). Load it at the top level of `nginx.conf`:

```nginx
load_module /path/to/libngx_mbrotli.so;
```

To build against a vendored NGINX instead, enable the `vendored` feature (see
[nginx-src](https://docs.rs/nginx-src/)). Prefer building against the exact
sources and configuration that run in production.

## Configuration directives

### `brotli_static`

- **syntax**: `brotli_static on|off|always`
- **default**: `off`
- **context**: `http`, `server`, `location`

Enables or disables checking for pre-compressed files with the `.br` extension.
With `always`, the pre-compressed file is served whether or not the client
supports it.

### `brotli`

- **syntax**: `brotli on|off`
- **default**: `off`
- **context**: `http`, `server`, `location`, `if`

Enables or disables on-the-fly compression of responses.

### `brotli_types`

- **syntax**: `brotli_types <mime_type> [..]`
- **default**: `text/html`
- **context**: `http`, `server`, `location`

Compresses responses with the listed MIME types as well as `text/html`, which is
always compressed. The special value `*` matches any MIME type.

### `brotli_buffers`

- **syntax**: `brotli_buffers <number> <size>`
- **default**: `32 4k|16 8k`
- **context**: `http`, `server`, `location`

Sets the number and size of the buffers that hold compressed output. The default
is one memory page per buffer, up to 128k in total.
google/ngx_brotli accepts this directive but ignores it; here it works the same
way as [`gzip_buffers`](https://nginx.org/en/docs/http/ngx_http_gzip_module.html#gzip_buffers).

### `brotli_comp_level`

- **syntax**: `brotli_comp_level <level>`
- **default**: `6`
- **context**: `http`, `server`, `location`

Sets the Brotli quality (compression level), from `0` to `11`.

### `brotli_window`

- **syntax**: `brotli_window <size>`
- **default**: `512k`
- **context**: `http`, `server`, `location`

Sets the Brotli window size. Acceptable values are `1k`, `2k`, `4k`, `8k`, `16k`,
`32k`, `64k`, `128k`, `256k`, `512k`, `1m`, `2m`, `4m`, `8m` and `16m`.
When the response length is known, a smaller window is used if it is enough.

### `brotli_min_length`

- **syntax**: `brotli_min_length <length>`
- **default**: `20`
- **context**: `http`, `server`, `location`

Sets the minimum length of a response that will be compressed. The length comes
only from the `Content-Length` response header.

### `brotli_reuse_memory`

- **syntax**: `brotli_reuse_memory on|off`
- **default**: `on`
- **context**: `http`, `server`, `location`

Enables or disables reusing encoders between responses. When on, each worker
keeps up to `brotli_idle_encoders` idle encoders, so a response can reuse the
window, match finder and buffers a previous one allocated. How much memory an idle encoder keeps is set by
`brotli_retention_policy`. When off, every response gets a new encoder that is
freed when the response is complete, as with google/ngx_brotli.

*Not in google/ngx_brotli.*

### `brotli_retention_policy`

- **syntax**: `brotli_retention_policy aggressive | current_config | release_all | bounded <size>`
- **default**: `aggressive`
- **context**: `http`, `server`, `location`

Sets how much working memory an encoder keeps between responses. It matters only
when `brotli_reuse_memory` is on.

- `aggressive` keeps every buffer, so a worker allocates only what its largest
  responses grow into. This is the fastest option, but each worker keeps memory
  for its largest responses at `brotli_comp_level` 11 with a 16m `brotli_window`.
- `current_config` keeps buffers only while they suit the current level and
  window. They are released when the encoder is reused for a location with
  different settings.
- `bounded <size>` keeps buffers while they fit within `size`, and releases them
  after a response that grew past it.
- `release_all` keeps nothing between responses; only the encoder itself is reused.

Idle encoders are reused only by locations with the same retention policy; when a
worker's `brotli_idle_encoders` slots are full, the least recently used encoder
is freed.

With `brotli_thread_pool`, an idle compressor keeps the encoders of up to
`brotli_parallel_tasks` segments, and the policy applies to all of them
together. A response holds its compressor until it has been sent, including
while its segments wait for a free thread, so a worker keeps encoders for at
most `brotli_parallel_compressors` compressors, idle or in use; other responses
allocate encoders for their segments and free them once they have been sent.

*Not in google/ngx_brotli.*

### `brotli_idle_encoders`

- **syntax**: `brotli_idle_encoders <number>`
- **default**: `4`
- **context**: `http`

Sets how many idle encoders each worker keeps for reuse with
`brotli_reuse_memory` on, from `0` to `1024`; when they are all taken, the least
recently used one is freed. Compressors for `brotli_thread_pool` are kept
separately, up to the same number. With `0` nothing is kept, so every response
gets a new encoder.

Idle encoders are reused only by locations with the same
`brotli_retention_policy`, so a worker that serves locations with several
policies may need more of them.

*Not in google/ngx_brotli.*

### `brotli_thread_pool`

- **syntax**: `brotli_thread_pool <name> | off`
- **default**: `off`
- **context**: `http`, `server`, `location`

Compresses responses on the named thread pool instead of in the worker's event
loop, so that a long response or a high `brotli_comp_level` does not hold up the
worker's other connections. The pool is declared with
[`thread_pool`](https://nginx.org/en/docs/ngx_core_module.html#thread_pool) in the
main context; a pool named `default` with 32 threads exists without being
declared. Requires NGINX built with `--with-threads`.

A response is cut into segments of `brotli_parallel_segment_size`, and up to
`brotli_parallel_tasks` of them are compressed on the pool's threads at once. They
are sent in order as they complete, while the next segments are read. A response
that fits in one segment is compressed in one piece by one thread. Segments are
compressed independently: back-references do not cross segment boundaries, so a
response longer than one segment compresses slightly worse, and a flush (for
example with `proxy_buffering off`) ends the current segment early.

Every worker process has its own threads, so `worker_processes` times the pool's
`threads` run at once. A response being compressed holds up to
`brotli_parallel_tasks` + 1 segments of input, their compressed output, and an
encoder for each segment.

*Not in google/ngx_brotli.*

### `brotli_parallel_segment_size`

- **syntax**: `brotli_parallel_segment_size <size>`
- **default**: `1m`
- **context**: `http`, `server`, `location`

Sets the size of the segments a response is cut into for `brotli_thread_pool`,
from `64k` to `16m`. Each segment is compressed without the context of the ones
before it, so the larger the segments, the better a response compresses, but
the fewer of its segments are compressed at once, and the more memory each of
their encoders needs. Smaller segments spread responses over more threads and
compress worse.

*Not in google/ngx_brotli.*

### `brotli_parallel_tasks`

- **syntax**: `brotli_parallel_tasks <number>`
- **default**: `4`
- **context**: `http`, `server`, `location`

Sets how many segments of one response may be compressed, or wait to be sent, at
once with `brotli_thread_pool`, from `1` to `1024`. When they all are, NGINX stops
reading the response until the first of them has been sent.

*Not in google/ngx_brotli.*

### `brotli_parallel_compressors`

- **syntax**: `brotli_parallel_compressors <number>`
- **default**: `4`
- **context**: `http`

Sets for how many compressors used with `brotli_thread_pool` each worker keeps
segment encoders, from `0` to `1024`, counting both idle ones and ones held by
responses. A response holds its compressor until it has been sent, including
while its segments wait for a free thread, so without a limit every response in
flight could keep the encoders a previous response grew. Responses beyond the
limit allocate encoders for their segments and free them once they have been
sent. Idle compressors are also limited by `brotli_idle_encoders`. Requires
NGINX built with `--with-threads`.

A higher number lets more responses reuse encoders, and costs the memory of
their encoders: in single runs with a 4-thread pool and 32 connections on
jquery.min.js at level 9, `32` served 8% more requests per second than the
default, and the worker peaked at 168 MiB instead of 91 MiB. A pool whose
threads often all compress at once, such as `default` with 32 threads, may
benefit from a number closer to its threads, together with a higher
`brotli_idle_encoders`.

*Not in google/ngx_brotli.*

## Variables

### `$brotli_ratio`

The achieved compression ratio: the original size divided by the compressed
size, to two decimal places (for example `3.59`).

## Sample configuration

```nginx
load_module /path/to/libngx_mbrotli.so;

http {
    brotli on;
    brotli_comp_level 6;
    brotli_static on;
    brotli_types application/atom+xml application/javascript application/json application/vnd.api+json application/rss+xml
                 application/vnd.ms-fontobject application/x-font-opentype application/x-font-truetype
                 application/x-font-ttf application/x-javascript application/xhtml+xml application/xml
                 font/eot font/opentype font/otf font/truetype image/svg+xml image/vnd.microsoft.icon
                 image/x-icon image/x-win-bitmap text/css text/javascript text/plain text/xml;
}
```

As with google/ngx_brotli, the `Vary: Accept-Encoding` header is added only
when [`gzip_vary`](https://nginx.org/en/docs/http/ngx_http_gzip_module.html#gzip_vary)
is on.

## Differences from google/ngx_brotli

- **`brotli_buffers` takes effect** (see above).
- **Encoders are reused** by default (`brotli_reuse_memory`), each keeping all
  of its working memory between responses (`brotli_retention_policy`).
  Set `brotli_reuse_memory off` to allocate per response as google/ngx_brotli
  does. Peak memory still follows the number of responses being compressed at
  once, times the working set that `brotli_comp_level` and `brotli_window` need.
- **Responses can be compressed on a thread pool** (`brotli_thread_pool`), in
  segments that are compressed in parallel.
- **The size hint comes from `Content-Length`.** When the length is known it is
  passed to the encoder, which uses it to choose a match finder at levels 4–9.
- **Fonts are compressed in Brotli's font mode.** Responses whose type is an
  uncompressed TrueType or OpenType font (`font/ttf`, `font/otf`, `font/sfnt`,
  `font/collection`, `application/vnd.ms-fontobject`, `application/x-font-ttf`
  and similar) use font mode, which makes them about 0.1–0.3% smaller at levels
  4–11 and does not affect speed. All other responses, including WOFF and WOFF2
  fonts, which are already compressed, use generic mode as google/ngx_brotli
  does. Text mode is not used: it does not change Brotli's output.
- The pure-Rust mbrotli encoder replaces the C library. It produces the same
  bytes as Google's encoder at every level, so you do not need to vendor or build
  the C Brotli library.

## Benchmarks

One NGINX worker compressing static files on the fly with google/ngx_brotli and
with ngx_mbrotli, both in the worker's event loop and on a thread pool of 4
threads. google/ngx_brotli is used as shipped: commit
[`a71f9312`](https://github.com/google/ngx_brotli/commit/a71f9312c2deb28875acc7bacfdd5695a111aa53)
with the Brotli 1.1.0 it pins as a submodule. On the thread pool, ngx_mbrotli
runs with 1 task per response (`brotli_parallel_tasks 1`) and with the default
4. Everything else is left at its default, so ngx_mbrotli reuses encoders and
cuts responses into 1m segments. The worker and its pool share 4 CPUs.

With the thread pool:

- **Other requests are not held up.** While large responses are compressed at
  levels 6 and 11, the p99 latency of small uncompressed requests stays at
  1.7–3.3 ms, as with no compression at all (1.9 ms). With google/ngx_brotli it
  rises to 17–71 ms when the large responses arrive one at a time, and to
  115–514 ms when 8 arrive at once. See [Latency of other
  requests](#latency-of-other-requests).
- **One worker serves 3.8–4.5 times as many requests** as google/ngx_brotli at
  levels 4–6 on 85 KB–2.2 MB responses, 4.3–15 times as many at level 9 and
  6.4–9.6 times as many at level 11.
- **4 tasks compress a single 2.2 MB response 2.0–2.3 times as fast as 1 task**,
  because it is longer than one segment. Under load, with 8 connections, 1 and
  4 tasks serve within 3% of each other except in one case (+17%): the pool's
  threads are already busy with different responses.
- **CPU time per request is within 0.6–1.23 times** that of the event loop. At
  level 11 the pool needs 37–40% less, because 1 MiB segments compress faster than
  a whole file; at level 9 it needs 22–40% more.
- **Responses longer than one segment compress 0.9–5.2% worse**, because
  segments are compressed independently.
- **A worker needs much more memory**: at 8 connections on 1–2.2 MB responses it
  peaks at 138–995 MiB, 11–26 times as much as google/ngx_brotli. See [Worker
  memory](#worker-memory).

In the event loop, compared with google/ngx_brotli:

- **Requests per second are 5–64% higher** on 85 KB–2.2 MB responses, and 4.4
  times as high at level 9 on the 85 KB file.
- **The event loop is blocked just the same**: the p99 latency of other
  requests is 14–41% lower only because each response compresses faster.
- **A worker's peak memory is 2–5 MiB higher.**

### Requests per second

Requests per second at 8 connections, the median of three runs; each difference
is relative to google/ngx_brotli.

| Response | Level | ngx_brotli | ngx_mbrotli, event loop | Difference | Pool, 1 task | Difference | Pool, 4 tasks | Difference |
|---|--:|--:|--:|--:|--:|--:|--:|--:|
| jquery.min.js, 85 KB | 4 | 985 | 1,135 | +15% | 4,180 | 4.2× | 4,154 | 4.2× |
| jquery.min.js, 85 KB | 6 | 674 | 706 | +5% | 2,528 | 3.8× | 2,545 | 3.8× |
| jquery.min.js, 85 KB | 9 | 108 | 472 | 4.4× | 1,655 | 15.3× | 1,669 | 15.4× |
| jquery.min.js, 85 KB | 11 | 14.9 | 24.4 | +64% | 94.5 | 6.4× | 95.0 | 6.4× |
| react-dom.development.js, 1.03 MB | 4 | 134 | 154 | +15% | 512 | 3.8× | 512 | 3.8× |
| react-dom.development.js, 1.03 MB | 6 | 63.3 | 75.3 | +19% | 282 | 4.5× | 276 | 4.4× |
| react-dom.development.js, 1.03 MB | 9 | 24.5 | 40.1 | +64% | 110 | 4.5× | 128 | 5.2× |
| react-dom.development.js, 1.03 MB | 11 | 1.02 | 1.60 | +57% | 9.76 | 9.6× | 9.78 | 9.6× |
| JSON, 2.2 MB | 4 | 68.4 | 78.1 | +14% | 263 | 3.8× | 268 | 3.9× |
| JSON, 2.2 MB | 6 | 32.7 | 37.8 | +16% | 146 | 4.5× | 142 | 4.3× |
| JSON, 2.2 MB | 9 | 14.8 | 20.1 | +35% | 63.3 | 4.3× | 63.3 | 4.3× |
| JSON, 2.2 MB | 11 | 0.55 | 0.87 | +58% | 5.11 | 9.3× | 5.26 | 9.6× |

At level 9 the C encoder allocates a 32 MiB hash table for every response over
64 KB and frees it afterwards. That cost does not depend on the response size,
so the difference is largest on the 85 KB file.

CPU time per request, in ms, includes the pool's threads:

| Response | Level | ngx_brotli | ngx_mbrotli, event loop | Pool, 1 task | Pool, 4 tasks |
|---|--:|--:|--:|--:|--:|
| react-dom.development.js, 1.03 MB | 6 | 16.0 | 13.4 | 14.2 | 14.5 |
| react-dom.development.js, 1.03 MB | 9 | 42.3 | 25.6 | 35.8 | 31.2 |
| react-dom.development.js, 1.03 MB | 11 | 1,153 | 695 | 414 | 415 |
| JSON, 2.2 MB | 9 | 71.4 | 51.1 | 63.0 | 62.9 |
| JSON, 2.2 MB | 11 | 2,432 | 1,256 | 789 | 785 |

### Latency of other requests

Small uncompressed requests (1.9 KB, 1,000 per second, 32 connections) sent
alongside a stream of large compressed responses, at about half the rate that
google/ngx_brotli can sustain. All variants kept up with both streams. The p99
latency of the small requests, in ms, with "no compression" serving the large
responses uncompressed:

| Large responses | Arrival | No compression | ngx_brotli | ngx_mbrotli, event loop | Pool, 1 task | Pool, 4 tasks |
|---|---|--:|--:|--:|--:|--:|
| react-dom.development.js, level 6, 31 per second | 8 at once | 1.9 | 115 | 94 | 3.3 | 2.8 |
| react-dom.development.js, level 6, 31 per second | One at a time | 1.9 | 17.0 | 14.7 | 1.7 | 1.8 |
| jquery.min.js, level 11, 7 per second | 8 at once | 1.9 | 514 | 306 | 2.0 | 1.9 |
| jquery.min.js, level 11, 7 per second | One at a time | 1.9 | 71.2 | 45.7 | 1.8 | 1.9 |

In the event loop, a small request that arrives during compression waits until
it ends. When the 8 connections of the large stream send at once, it waits for
up to 8 responses to be compressed back to back; when they arrive one at a
time, for one. On the thread pool, the maximum latency one at a time was
3.6 ms.

### A single large response

Time to serve one `libraries.json` response (2.2 MB, three 1m segments) at 1
connection, in ms, the median of three runs:

| Level | ngx_brotli | ngx_mbrotli, event loop | Pool, 1 task | Pool, 4 tasks |
|--:|--:|--:|--:|--:|
| 4 | 14.6 | 12.9 | 13.1 | 6.6 |
| 6 | 30.6 | 26.6 | 25.2 | 12.5 |
| 9 | 69.1 | 50.1 | 54.9 | 24.1 |
| 11 | 2,004 | 1,253 | 771 | 371 |

Responses of up to 1 MiB fit in one segment, so the number of tasks does not
change how fast they are served. `react-dom.development.js` is cut into a 1 MiB
segment and a 31 KB one, so 4 tasks serve it only 4–34% faster than 1.

### Worker memory

The worker's peak resident set size (`VmHWM`) at 8 connections, in MiB, the
highest of three runs:

| Response | Level | ngx_brotli | ngx_mbrotli, event loop | Pool, 1 task | Pool, 4 tasks |
|---|--:|--:|--:|--:|--:|
| react-dom.development.js, 1.03 MB | 4 | 9 | 12 | 117 | 138 |
| react-dom.development.js, 1.03 MB | 6 | 11 | 14 | 144 | 143 |
| react-dom.development.js, 1.03 MB | 9 | 39 | 41 | 482 | 483 |
| react-dom.development.js, 1.03 MB | 11 | 25 | 28 | 296 | 269 |
| JSON, 2.2 MB | 4 | 9 | 14 | 143 | 215 |
| JSON, 2.2 MB | 6 | 11 | 14 | 210 | 276 |
| JSON, 2.2 MB | 9 | 39 | 43 | 519 | 995 |
| JSON, 2.2 MB | 11 | 28 | 30 | 390 | 505 |

In the event loop, a response is compressed in one go, so a worker needs one
encoder at a time. On a thread pool, every response being sent holds the
compressors of its segments, so memory grows with the number of responses in
flight and with the number of tasks per response: on the 2.2 MB file, 4 tasks
need up to 1.9 times as much as 1. `brotli_retention_policy bounded <size>` or
`release_all` keep less of it between responses.

### Segment size

With `brotli_parallel_segment_size 4m` instead of 1m, both files fit in one
segment, so the output is as small as in the event loop instead of 0.9–5.2%
larger, and the peak memory is 19–42% lower. At levels 4–9 it serves 9% fewer
to 14% more requests per second than 1m. At level 11 it serves 35–37% fewer,
because a whole file compresses slower than 1 MiB segments, and its CPU time
per request returns to that of the event loop. Measured with 1 task per
response at 8 connections, the median of three runs:

| Response | Level | Requests per second, 1m | Requests per second, 4m | CPU per request, 1m | CPU per request, 4m | Peak memory, 1m | Peak memory, 4m |
|---|--:|--:|--:|--:|--:|--:|--:|
| react-dom.development.js, 1.03 MB | 4 | 497 | 486 | | | | |
| react-dom.development.js, 1.03 MB | 6 | 275 | 257 | | | | |
| react-dom.development.js, 1.03 MB | 9 | 109 | 123 | 36.3 ms | 32.4 ms | 499 MiB | 324 MiB |
| react-dom.development.js, 1.03 MB | 11 | 9.6 | 6.1 | 418 ms | 669 ms | 292 MiB | 215 MiB |
| JSON, 2.2 MB | 4 | 260 | 258 | | | | |
| JSON, 2.2 MB | 6 | 145 | 133 | | | | |
| JSON, 2.2 MB | 9 | 62.3 | 63.3 | 63.8 ms | 63.8 ms | 526 MiB | 331 MiB |
| JSON, 2.2 MB | 11 | 5.1 | 3.3 | 787 ms | 1,257 ms | 398 MiB | 230 MiB |

With 4m, a response is compressed in parallel only when it is longer than
4 MiB, so the default stays at 1m.

<details>
<summary>Hardware, build and method</summary>

**Hardware**

- Intel Core i7-13700KF.
- Windows 10.0.26300.9711, WSL 2.7.14.0 with kernel
  6.18.33.2-microsoft-standard-WSL2, Ubuntu 22.04.5. WSL2 shows 12 identical
  cores with 2 threads each and does not expose which are performance cores, so
  some of the CPUs pinned below may be efficiency cores.
- Files are served from tmpfs (`/dev/shm`).

**Build**

- NGINX 1.30.6, `./configure --with-compat --with-threads
  --without-http_rewrite_module`, both modules dynamic, gcc 11.4.0.
- google/ngx_brotli at commit `a71f9312c2deb28875acc7bacfdd5695a111aa53`, with
  its `deps/brotli` submodule at `ed738e842d2fbdf2d6459e39267a633c4a9b2f5d`
  (Brotli 1.1.0).
- ngx_mbrotli at commit `8f7508a` with mbrotli 0.5.10, `cargo build --release`
  (rustc 1.99.0).

**Configuration**

`worker_processes 1`, with the worker and its pool pinned to 4 physical cores
(CPUs 0, 2, 4 and 6), and `thread_pool cp threads=4`. For the pool,
`brotli_thread_pool cp` with `brotli_parallel_tasks 1` or the default 4.
Everything else is left at its default, including
`brotli_parallel_segment_size 1m` and `brotli_reuse_memory on`.

**Requests per second**

- wrk 4.1.0 on 4 other physical cores (CPUs 8, 10, 12 and 14) over loopback,
  without TLS, with 2 threads and 8 connections, after a 1-second warm-up.
- 4 seconds per run, 8 seconds for files over 1 MB, and 5 times as long at
  level 11.
- Three runs; the order of cells and variants is shuffled between runs. NGINX
  is restarted for every run, and the run starts once the worker has been idle
  for 300 ms.
- CPU time per request is the worker's user and system time over the run, from
  `/proc/<pid>/stat`, including the pool's threads.
- Before the runs, every response in every combination of variant, level and
  file was decoded with curl's Brotli decoder and compared with the original
  file by SHA-256.

**Latency of other requests**

- Two wrk2 (commit `44a94c1`) streams at once, at constant rates: the small
  requests from CPUs 16 and 18, and the large ones from CPUs 8, 10, 12 and 14.
- p99 is corrected for coordinated omission.
- 8 at once: the large stream's 8 connections, which wrk2 starts together and
  which stay in phase. Median of three 30-second runs.
- One at a time: the large stream on a single connection at a constant
  interval. One 30-second run.

**Files**

jQuery 3.7.1 (`jquery.min.js`) and React DOM 18.3.1 (`react-dom.development.js`)
from cdnjs, the full cdnjs API listing (`libraries.json`, 2.2 MB), and its
first 10 entries (1.9 KB), all fetched on 2026-10-20.

</details>

## Implementation notes

The filter sits in the same place in the filter chain as google/ngx_brotli's, just
above the gzip filter. The dynamic library exports an `ngx_module_order` table so
that NGINX inserts it there.

All access to C bit-fields (request, buffer, variable and open-file flags) goes
through `src/bitfields.c`, compiled against the same headers as NGINX. The bit-field
accessors that bindgen generates for `ngx_http_request_t` do not match the C
compiler's layout in NGINX 1.30 (they are two bits off from `uri_changes`
onwards), so they cannot be used safely.

With `brotli_thread_pool`, each segment becomes an `ngx_thread_task_t`. Its
completion handler runs in the event loop and calls the brotli filter to send
what is ready, then runs the connection's write handler, as a write event would.
While tasks are posted, the request is blocked (`r->main->blocked`), so that
terminating it waits for them, and stays buffered, so that it is not finalized
before the last completion handler has run, even when the response has already
been sent.

## Testing

Unit tests do not need a running NGINX:

```sh
NGINX_BUILD_DIR=/path/to/nginx/objs cargo test
```

The integration tests in `t/` use the
[nginx-tests](https://github.com/nginx/nginx-tests) framework and need the
`brotli` command-line tool:

```sh
git clone https://github.com/nginx/nginx-tests
TEST_NGINX_BINARY=/path/to/nginx/objs/nginx \
TEST_NGINX_GLOBALS="load_module $PWD/target/release/libngx_mbrotli.so;" \
    prove -I nginx-tests/lib t/
```

## Contributing and releases

CI (`.github/workflows/ci.yml`) runs on pull requests and pushes to `main`:

- rustfmt and clippy;
- unit and integration tests against NGINX 1.28, 1.30 and 1.31, built with
  `--with-threads`; clippy also checks a build without it;
- the integration tests under AddressSanitizer and LeakSanitizer, with NGINX
  and the module (including the Rust standard library) instrumented, and
  `NGX_DEBUG_PALLOC` so that each pool allocation is checked on its own;
- the unit tests under Miri. The body filter (`src/body.rs`) reaches NGINX only
  through a small `Host` trait, so Miri runs it against a model of NGINX's
  buffer and chain API (`src/body/tests.rs`). The model frees drained input
  buffers and chain links, and leaves output buffers uninitialized, so stale
  pointers and reads of unwritten output show up as undefined behaviour. Its
  thread pool is either a queue that the tests run and report on their own
  schedule, or real threads, so that Miri also looks for data races between
  the filter and the segments being encoded.

Changelogs and versions are managed with [Sampo](https://github.com/bruits/sampo).
Every user-facing change needs a changeset, a short Markdown file in
`.sampo/changesets/`:

```sh
cargo install sampo
sampo add          # pick the bump level (patch, minor, major) and describe the change
```

On every push to `main`, the release workflow (`.github/workflows/release.yml`)
collects the pending changesets into a **Release (main)** pull request that bumps
the version in `Cargo.toml` and updates `CHANGELOG.md`. Merging that pull request
tags the release (`vX.Y.Z`) and publishes a GitHub Release with the changelog
entry. The crate is not published to crates.io.

For the workflow to open pull requests, enable *Settings → Actions → General →
Allow GitHub Actions to create and approve pull requests*.

Packages are published by hand: run the **Publish** workflow
(`.github/workflows/publish.yml`) on the release tag. It builds a package for
every target, tests each in a clean container of its distribution (install,
`nginx -t`, a brotli-compressed response with and without a thread pool, a
pre-compressed `.br` file with `brotli_static`, removal), and pushes them to
Cloudsmith only if all of them pass. With `dry_run` it builds and tests without
pushing, also from a branch. A rerun on the same tag skips the packages already
published. Targets come from nginx.org and the distributions at run time
(`.github/scripts/publish-matrix.py`, which also says how to add a
distribution); `nginx_stable` and `nginx_mainline` override the nginx.org
versions.

## License

MIT; see [LICENSE](LICENSE). The directives and their behaviour follow
[google/ngx_brotli](https://github.com/google/ngx_brotli) (BSD 2-Clause).
