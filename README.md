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
[benchmarks](#benchmarks), one worker with 4 threads serves 3.4–4.4 times as
many requests as google/ngx_brotli at levels 4–6 on 85 KB–2.2 MB responses, and
4.5–15 times as many at levels 9–11, while other requests keep the latency they
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
  - [Latency of other requests](#latency-of-other-requests)
  - [One large response](#one-large-response)
  - [CPU time per request](#cpu-time-per-request)
  - [Thread pool overhead](#thread-pool-overhead)
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
that fits in one segment is compressed in one piece by one thread. At
`brotli_comp_level` 2–9, every segment but the first also runs through the input
before it, a quarter of `brotli_parallel_segment_size` at most and never more
than the window, so that its back-references can reach into it: 256 KiB for 1m
segments with the default 512k window. At other levels a segment starts from
nothing, and back-references do not cross segment boundaries. Either way, a
response longer than one segment compresses worse than in one piece: in the
[benchmarks](#benchmarks), by 0.1–0.6% at levels 4–9 and by 3.8–5.2% at
level 11. A flush (for example with `proxy_buffering off`) ends the current
segment early.

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
from `64k` to `16m`. At levels 2–9 a segment can refer back to at most a quarter
of its size of the input before it, and at other levels to none of it (see
[`brotli_thread_pool`](#brotli_thread_pool)), so the larger the segments, the
better a response compresses and, at levels 2–9, the less input is run through
twice, but the fewer of its segments are compressed at once, and the more memory
each of their encoders needs. Smaller segments spread responses over more
threads and compress worse. See [Thread pool overhead](#thread-pool-overhead)
for 1m and 4m compared.

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
threads, with 1 or 4 tasks per response (`brotli_parallel_tasks`).
google/ngx_brotli is used as shipped: commit
[`a71f9312`](https://github.com/google/ngx_brotli/commit/a71f9312c2deb28875acc7bacfdd5695a111aa53)
with the Brotli 1.1.0 it pins as a submodule. All run with default settings
apart from `brotli_comp_level` and, on the pool, `brotli_thread_pool` and
`brotli_parallel_tasks`, so ngx_mbrotli reuses encoders and cuts responses into
1m segments. The worker and its pool share 4 CPUs.

These are results of a reduced run under WSL2 (see *Hardware, build and
method* at the end of this section): good for comparing the modules with each
other, less so as absolute numbers.

With the thread pool:

- **Other requests are not held up.** While large responses are compressed, the
  99th percentile latency of small uncompressed requests on the same worker
  stays at 1.7–1.8 ms, as when nothing is compressed (1.9 ms). With
  google/ngx_brotli it grows to 17 ms at level 6 and 72 ms at level 11, and to
  117–527 ms when 8 large requests arrive at once.
- **One worker serves 3.4–15 times as many requests** as with
  google/ngx_brotli on responses of 85 KB and more. The gain is largest at
  levels 9–11.
- **A 2.2 MB response is sent 1.9–2.3 times as fast with 4 tasks as with 1**,
  and 1.9–5.4 times as fast as by google/ngx_brotli. Only responses longer than
  one segment gain from more tasks.
- **CPU time per request is 1.2–1.5 times higher** than in the event loop at
  levels 4–9 on responses of 1 MB and more, and 37–38% lower at level 11. It
  is higher than with google/ngx_brotli only at level 4. See [Thread pool
  overhead](#thread-pool-overhead).
- **Responses longer than one segment compress 0.1–0.6% worse** at levels 4–9
  and 3.8–5.2% worse at level 11. Of the files here, only the 1 MB and 2.2 MB
  ones are longer than the default 1m segment.
- **A worker needs much more memory**: each response in flight holds encoders
  for its segments, and with 8 connections a worker peaks at up to 1.1 GiB
  (level 9, 2.2 MB response, 4 tasks). `brotli_retention_policy bounded <size>`
  or `release_all` keep less of it between responses.

In the event loop, compared with google/ngx_brotli:

- **One worker serves 1.10–4.2 times as many requests** on responses of 85 KB
  and more, the most on jquery.min.js at level 9: for every response over
  64 KB at level 9, the C encoder allocates a 32 MiB hash table and frees it
  afterwards.
- **On 1.9 KB responses, it serves 4–122% more requests**, the most at level 6.
  With the pool, ngx_mbrotli serves 1.5–6.1 times as many as
  google/ngx_brotli.
- **A worker's memory is 0–4 MiB higher**, except on jquery.min.js at level 9,
  where it peaks at 8 MiB against 36 MiB.
- **The event loop is blocked all the same.** Small requests wait 21–45% less,
  only because compression itself is faster.

### Latency of other requests

The 99th percentile latency of small uncompressed requests, 1,000 per second,
while one connection requests a large compressed response at a fixed rate,
about half of what google/ngx_brotli can serve, in ms:

| Level, large requests | No compression | ngx_brotli | ngx_mbrotli, event loop | ngx_mbrotli, pool, 1 task | ngx_mbrotli, pool, 4 tasks |
|---|--:|--:|--:|--:|--:|
| 6, react-dom.development.js (1.03 MB), 31 per second | 1.9 | 17.0 | 13.5 | 1.8 | 1.7 |
| 11, jquery.min.js (85 KB), 7 per second | 1.9 | 72.2 | 43.2 | 1.8 | 1.8 |

On the pool, no small request takes more than 4.0 ms. When the large requests
come from 8 connections at once, google/ngx_brotli compresses them one after
another, so at level 11 small requests wait for up to 8 × 70 ms (p99 527 ms);
on the pool they stay at 1.9–3.1 ms. The large responses themselves then take
a median 69 ms with 4 tasks, 70 ms with 1 task and 280 ms with
google/ngx_brotli at level 11, and 21, 28 and 71 ms at level 6.

### One large response

Time to send libraries.json (2.2 MB, 3 segments of 1m) over one connection, one
response at a time, median of three runs, in ms:

| Level | ngx_brotli | ngx_mbrotli, event loop | ngx_mbrotli, pool, 1 task | ngx_mbrotli, pool, 4 tasks |
|--:|--:|--:|--:|--:|
| 4 | 14.9 | 13.0 | 15.1 | 7.8 |
| 6 | 31.6 | 24.9 | 27.1 | 13.6 |
| 9 | 72.9 | 46.6 | 58.9 | 25.2 |
| 11 | 2,004 | 1,113 | 771 | 371 |

The gain from more tasks is bounded by the number of segments: react-dom (1 MiB
and 31 KB) is only 4–50% faster with 4 tasks than with 1, and responses up to
1 MiB do not depend on the number of tasks at all. At level 11 even 1 task is
faster than the event loop, because 1 MiB segments compress faster than a
whole file.

### CPU time per request

The worker's CPU time per request, including the pool's threads, with 8
connections, median of three runs, in ms:

| Response | Level | ngx_brotli | ngx_mbrotli, event loop | ngx_mbrotli, pool, 1 task | ngx_mbrotli, pool, 4 tasks |
|---|--:|--:|--:|--:|--:|
| react-dom.development.js, 1.03 MB | 4 | 7.8 | 6.6 | 9.0 | 8.9 |
| react-dom.development.js, 1.03 MB | 6 | 16.5 | 12.7 | 15.3 | 15.1 |
| react-dom.development.js, 1.03 MB | 9 | 43.4 | 23.9 | 35.1 | 31.4 |
| react-dom.development.js, 1.03 MB | 11 | 1,188 | 651 | 405 | 408 |
| libraries.json, 2.2 MB | 4 | 15.1 | 13.1 | 17.2 | 16.7 |
| libraries.json, 2.2 MB | 6 | 32.1 | 24.8 | 29.5 | 30.2 |
| libraries.json, 2.2 MB | 9 | 75.7 | 47.6 | 61.6 | 63.5 |
| libraries.json, 2.2 MB | 11 | 2,522 | 1,207 | 765 | 766 |

### Thread pool overhead

The extra CPU time on the pool at levels 4–9 has two measured causes:

- **Segment history.** At levels 2–9, every segment but the first first runs
  through the quarter segment of input before it, within the window, so that
  its matches can reach back: 256 KiB for each 1 MiB segment. Levels 10 and 11
  take no history and start every segment from nothing, so their segments
  compress worse but cost no extra time.
- **Encoders allocated anew.** A worker keeps encoders for
  `brotli_parallel_compressors` compressors, 4 by default; with 8 responses in
  flight, the others allocate their encoders for every response. At level 9 on
  libraries.json with 8 connections and 4 tasks, `brotli_parallel_compressors 8`
  lowers CPU time per request from 64 to 54 ms and serves 20% more requests,
  and the worker's peak memory grows from 1.1 to 1.3 GiB. With 1 task it
  changes neither (62 ms).

Even a response that fits in one segment costs 1.11–1.24 times the event loop's
CPU time on the pool at levels 4–9.

A larger `brotli_parallel_segment_size` means less history to run through. With
4m, both files here fit in one segment, so a response is compressed by one task
without history. With 1 task and 8 connections:

| Response | Level | CPU, event loop | CPU, 1m | CPU, 4m | Peak memory, 1m | Peak memory, 4m |
|---|--:|--:|--:|--:|--:|--:|
| react-dom.development.js, 1.03 MB | 4 | 6.6 ms | 9.0 ms | 8.2 ms | 114 MiB | 114 MiB |
| react-dom.development.js, 1.03 MB | 6 | 12.7 ms | 15.4 ms | 14.1 ms | 155 MiB | 137 MiB |
| react-dom.development.js, 1.03 MB | 9 | 23.9 ms | 34.7 ms | 29.5 ms | 584 MiB | 332 MiB |
| react-dom.development.js, 1.03 MB | 11 | 651 ms | 405 ms | 612 ms | 277 MiB | 222 MiB |
| libraries.json, 2.2 MB | 4 | 13.1 ms | 17.1 ms | 15.1 ms | 139 MiB | 133 MiB |
| libraries.json, 2.2 MB | 6 | 24.8 ms | 29.4 ms | 27.5 ms | 174 MiB | 145 MiB |
| libraries.json, 2.2 MB | 9 | 47.6 ms | 61.5 ms | 53.8 ms | 587 MiB | 338 MiB |
| libraries.json, 2.2 MB | 11 | 1,207 ms | 758 ms | 1,162 ms | 350 MiB | 231 MiB |

With 4m, CPU time per request at levels 4–9 is 1.11–1.24 times the event
loop's instead of 1.19–1.45 times, a worker serves 7–19% more requests, and the
output is the same size as in the event loop. At level 11 it serves 32–33%
fewer, because a whole file compresses slower than 1 MiB segments. In
exchange, only responses longer than 4 MiB are compressed by more than one
thread at once.

<details>
<summary>Hardware, build and method</summary>

**Hardware**

- Intel Core i7-13700KF.
- Ubuntu 22.04.5 under WSL2 2.7.14.0, kernel 6.18.33.2-microsoft-standard-WSL2,
  on Windows 10.0.26300. WSL2 shows 12 identical cores of 2 threads each and
  does not expose which of them are performance cores.

**Build**

- NGINX 1.30.6, `./configure --with-compat --with-threads
  --without-http_rewrite_module`, gcc 11.4.0, both modules dynamic.
- google/ngx_brotli at commit `a71f9312c2deb28875acc7bacfdd5695a111aa53`, with
  its `deps/brotli` submodule at `ed738e842d2fbdf2d6459e39267a633c4a9b2f5d`
  (Brotli 1.1.0).
- ngx_mbrotli 0.4.5 with mbrotli 0.5.20, `cargo build --release` (rustc 1.99.0).

**Configuration**

`worker_processes 1`, pinned to 4 CPUs on different physical cores, and
`thread_pool cp threads=4`. On the pool, `brotli_thread_pool cp` with
`brotli_parallel_tasks 1` or `4`. Everything else is left at its default,
including `brotli_parallel_segment_size 1m`, `brotli_parallel_compressors 4`
and `brotli_retention_policy aggressive`. Files are served from tmpfs.

**Load**

- Over loopback, without TLS, from 4 other physical cores, and the small
  requests of the latency test from 2 more.
- Requests per second and CPU time: wrk 4.1.0, 2 threads and 8 connections, for
  4 seconds (8 seconds for files over 1 MB, 5 times longer at level 11), after a
  1-second warm-up. Each run starts NGINX anew and waits until the worker has
  been idle for 300 ms. Three runs in a shuffled order; the tables show the
  median.
- Time to send one response: the same with 1 connection, for 4 seconds
  (10 seconds at level 11), divided by the number of responses.
- CPU time per request is the worker's user and system time over the run, from
  `/proc/<pid>/stat`, including the pool's threads; peak memory is the worker's
  `VmHWM`.
- Latency of other requests: wrk2 (`44a94c1`), open loop with coordinated
  omission corrected, two streams at once: small.json without
  compression at 1,000 requests per second over 32 connections, and the large
  compressed response at a fixed rate, over 1 connection (one 30-second run) or
  8 connections (median of three 30-second runs).
- Every response, for every module, level and file, was decoded with curl's
  Brotli decoder and compared with the original file by SHA-256.

**Files**

jQuery 3.7.1 (`jquery.min.js`, 85 KB) and React DOM 18.3.1
(`react-dom.development.js`, 1.03 MB) from cdnjs, the full cdnjs API listing
(`libraries.json`, 2.2 MB), and its first 10 entries (`small.json`, 1.9 KB),
fetched on 2026-10-20. Measured on 2026-11-01.

Not measured yet: more than one worker, slow clients, long runs and
compression ratios beyond the correctness check.

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

Bug reports and pull requests are welcome. Read [CONTRIBUTING.md](CONTRIBUTING.md)
for the setup and the checks a change must pass, and follow the
[Code of Conduct](CODE_OF_CONDUCT.md). If the module behaves differently from
google/ngx_brotli with the same configuration, open an issue with the
[Differs from google/ngx_brotli](https://github.com/M-Compressors/ngx_mbrotli/issues/new?template=compatibility.yml)
template. Report security vulnerabilities privately as described in
[SECURITY.md](SECURITY.md), not as public issues.

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
