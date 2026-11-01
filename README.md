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
[benchmarks](#benchmarks), one worker with 4 threads serves 3.5–4.4 times as
many requests as google/ngx_brotli at levels 4–6 on 85 KB–2.2 MB responses, and
4.5–14 times as many at levels 9–11.

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
  - [CPU time per request](#cpu-time-per-request)
  - [Peak memory](#peak-memory)
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
response longer than one segment compresses worse than in one piece: on the
1 MB and 2.2 MB files of the [benchmarks](#benchmarks), by 0.1–0.6% at levels
4–9 and by 3.8–5.2% at level 11. A flush (for example with `proxy_buffering off`) ends the current
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
threads and compress worse.

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

One NGINX worker compresses static files on the fly for 8 connections at once,
in three setups:

- **google/ngx_brotli**: as shipped, commit [`a71f9312`](https://github.com/google/ngx_brotli/commit/a71f9312c2deb28875acc7bacfdd5695a111aa53) with the Brotli 1.1.0 it pins as a submodule.
- **Event loop**: ngx_mbrotli with its default settings, compressing in the worker's
  event loop like google/ngx_brotli.
- **Thread pool**: ngx_mbrotli with `brotli_thread_pool` on a pool of 4 threads, and
  otherwise default settings: up to 4 tasks per response and 1m segments.

Only `brotli_comp_level` changes from row to row. The worker and its pool share 4
CPUs. Each number is the median of three runs under WSL2 (see *Hardware, build
and method* below): good for comparing the setups with each other, less so as
absolute numbers.

### Requests per second

Higher is better. In brackets, relative to google/ngx_brotli.

| Response | Level | google/ngx_brotli | Event loop | Thread pool |
|---|--:|--:|--:|--:|
| small.json, 1.9 KB | 4 | 28,370 | 31,114 (1.10×) | 42,238 (1.49×) |
| small.json, 1.9 KB | 6 | 12,750 | 28,301 (2.22×) | 41,483 (3.25×) |
| small.json, 1.9 KB | 9 | 23,218 | 24,194 (1.04×) | 36,406 (1.57×) |
| small.json, 1.9 KB | 11 | 613 | 965 (1.57×) | 3,714 (6.06×) |
| jquery.min.js, 85 KB | 4 | 955 | 1,145 (1.20×) | 4,177 (4.37×) |
| jquery.min.js, 85 KB | 6 | 655 | 717 (1.10×) | 2,559 (3.91×) |
| jquery.min.js, 85 KB | 9 | 104 | 442 (4.24×) | 1,452 (13.9×) |
| jquery.min.js, 85 KB | 11 | 14.5 | 25.4 (1.75×) | 99.9 (6.88×) |
| react-dom.development.js, 1.03 MB | 4 | 130 | 152 (1.17×) | 448 (3.45×) |
| react-dom.development.js, 1.03 MB | 6 | 61.5 | 79.4 (1.29×) | 265 (4.30×) |
| react-dom.development.js, 1.03 MB | 9 | 24.0 | 42.6 (1.78×) | 128 (5.33×) |
| react-dom.development.js, 1.03 MB | 11 | 1.00 | 1.70 (1.70×) | 9.96 (9.96×) |
| libraries.json, 2.2 MB | 4 | 67.1 | 76.9 (1.15×) | 240 (3.58×) |
| libraries.json, 2.2 MB | 6 | 31.9 | 40.8 (1.28×) | 133 (4.16×) |
| libraries.json, 2.2 MB | 9 | 14.0 | 21.6 (1.54×) | 63.5 (4.54×) |
| libraries.json, 2.2 MB | 11 | 0.520 | 0.900 (1.73×) | 5.39 (10.4×) |

### CPU time per request

The worker's user and system CPU time per request, including the pool's
threads, in ms. Lower is better. On the thread pool, responses longer than one
segment (1 MB and more here) cost more at levels 4–9, because every segment but
the first also runs through part of the input before it, and less at level 11,
because 1 MiB segments compress faster than a whole file.

| Response | Level | google/ngx_brotli | Event loop | Thread pool |
|---|--:|--:|--:|--:|
| small.json, 1.9 KB | 4 | 0.035 | 0.032 | 0.050 |
| small.json, 1.9 KB | 6 | 0.078 | 0.035 | 0.054 |
| small.json, 1.9 KB | 9 | 0.043 | 0.041 | 0.075 |
| small.json, 1.9 KB | 11 | 1.63 | 1.04 | 1.06 |
| jquery.min.js, 85 KB | 4 | 1.05 | 0.875 | 0.922 |
| jquery.min.js, 85 KB | 6 | 1.53 | 1.40 | 1.53 |
| jquery.min.js, 85 KB | 9 | 9.74 | 2.27 | 2.72 |
| jquery.min.js, 85 KB | 11 | 70.5 | 39.9 | 40.1 |
| react-dom.development.js, 1.03 MB | 4 | 7.76 | 6.60 | 8.92 |
| react-dom.development.js, 1.03 MB | 6 | 16.5 | 12.7 | 15.1 |
| react-dom.development.js, 1.03 MB | 9 | 43.4 | 23.9 | 31.4 |
| react-dom.development.js, 1.03 MB | 11 | 1,188 | 651 | 408 |
| libraries.json, 2.2 MB | 4 | 15.1 | 13.1 | 16.7 |
| libraries.json, 2.2 MB | 6 | 32.1 | 24.8 | 30.2 |
| libraries.json, 2.2 MB | 9 | 75.7 | 47.6 | 63.5 |
| libraries.json, 2.2 MB | 11 | 2,522 | 1,207 | 766 |

### Peak memory

The worker's peak resident set size (`VmHWM`), in MiB. Lower is better. On the
thread pool, every response in flight holds encoders for its segments.

| Response | Level | google/ngx_brotli | Event loop | Thread pool |
|---|--:|--:|--:|--:|
| small.json, 1.9 KB | 4 | 5 | 6 | 9 |
| small.json, 1.9 KB | 6 | 5 | 6 | 15 |
| small.json, 1.9 KB | 9 | 6 | 7 | 22 |
| small.json, 1.9 KB | 11 | 6 | 6 | 21 |
| jquery.min.js, 85 KB | 4 | 5 | 6 | 22 |
| jquery.min.js, 85 KB | 6 | 7 | 8 | 45 |
| jquery.min.js, 85 KB | 9 | 36 | 8 | 43 |
| jquery.min.js, 85 KB | 11 | 9 | 11 | 68 |
| react-dom.development.js, 1.03 MB | 4 | 9 | 12 | 141 |
| react-dom.development.js, 1.03 MB | 6 | 11 | 14 | 169 |
| react-dom.development.js, 1.03 MB | 9 | 39 | 41 | 500 |
| react-dom.development.js, 1.03 MB | 11 | 25 | 28 | 264 |
| libraries.json, 2.2 MB | 4 | 9 | 13 | 220 |
| libraries.json, 2.2 MB | 6 | 10 | 14 | 273 |
| libraries.json, 2.2 MB | 9 | 39 | 42 | 1,089 |
| libraries.json, 2.2 MB | 11 | 28 | 30 | 492 |

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
`thread_pool cp threads=4`. For the thread pool, `brotli_thread_pool cp`.
Everything else is left at its default, including `brotli_parallel_tasks 4`,
`brotli_parallel_segment_size 1m`, `brotli_parallel_compressors 4` and
`brotli_retention_policy aggressive`. Files are served from tmpfs.

**Load**

- wrk 4.1.0 over loopback, without TLS, from 4 other physical cores, with 2
  threads and 8 connections, for 4 seconds (8 seconds for files over 1 MB, 5
  times longer at level 11), after a 1-second warm-up.
- Each run starts NGINX anew and waits until the worker has been idle for
  300 ms. Three runs in a shuffled order; the tables show the median.
- CPU time is read from `/proc/<pid>/stat` of the worker, including the pool's
  threads, and peak memory from `VmHWM` in `/proc/<pid>/status`.
- Every response, for every setup, level and file, was decoded with curl's
  Brotli decoder and compared with the original file by SHA-256.

**Files**

jQuery 3.7.1 (`jquery.min.js`) and React DOM 18.3.1
(`react-dom.development.js`) from cdnjs, the full cdnjs API listing
(`libraries.json`), and its first 10 entries (`small.json`), fetched on
2026-10-20. Measured on 2026-11-01.

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
