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
[benchmarks](#benchmarks), one worker with 4 threads serves 3.7–4.9 times as
many requests as google/ngx_brotli at levels 1–6 on 85 KB–1 MB responses, and
5–16 times as many at level 9.

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
with ngx_mbrotli, both in the worker's event loop (1 thread) and on a thread pool
of 4 threads. google/ngx_brotli is used as shipped: commit
[`a71f9312`](https://github.com/google/ngx_brotli/commit/a71f9312c2deb28875acc7bacfdd5695a111aa53)
with the Brotli 1.1.0 it pins as a submodule. All run with default settings apart
from `brotli_comp_level` and, for 4 threads, `brotli_thread_pool`, so ngx_mbrotli
reuses encoders (`brotli_reuse_memory on`). On the thread pool it cuts responses
into 1m segments, up to 4 at a time. The worker and its pool share 4 CPUs.

With 4 threads:

- **One worker serves 3.2–3.9 times as many requests** as ngx_mbrotli in the
  event loop on 85 KB–1 MB responses (6.3 times on the 1 MB file at level 11),
  and 3.7–16 times as many as google/ngx_brotli.
- **CPU time per request is up to 14% higher** than in the event loop on
  85 KB–1 MB responses, and 26% higher on the 1 MB file at level 9.
- **On 1.9 KB responses, level 1 serves 15% fewer requests** than
  google/ngx_brotli: handing such a response to a thread costs more than
  compressing it. Levels 4–9 serve 1.5–1.7 times as many, and level 11 4.4 times.
- **Responses longer than one segment compress 1–13% worse**, because segments
  are compressed independently. Of the files here, only the 1 MB one is longer
  than the default 1m segment.
- **A worker needs more memory**: at 32 connections it peaks at 75–172 MiB,
  against 7–13 MiB in the event loop, and keeps 42–128 MiB when idle. See
  [Worker memory](#worker-memory).

In the event loop (1 thread), compared with google/ngx_brotli:

- **Level 11 is 45–61% faster** at every response size.
- **Level 9 is 2.8–4.5 times faster** on 85–230 KB responses and 63% faster at
  1 MB, but 4% slower on 2 KB responses.
- **Level 6, the default, is up to 9% faster** on 85 KB–1 MB responses, but 8%
  slower on 2 KB responses.
- **Level 4 is up to 4% slower**, except at 1 MB, where it is 5% faster.
- **Level 1 is 23–47% faster** on responses over 32 KB, and its output is smaller.
- **Output matches byte for byte** at levels 2–11. The exceptions are levels 0–1,
  where ngx_mbrotli's output is smaller on responses over 32 KB (6–15% at
  level 1), and
  responses of 1 MB or more at levels 4–9, where the size hint selects a
  different match finder.
- **A worker's peak memory is 1.3–4 MiB higher**, except at level 9, where it is
  under a third of google/ngx_brotli's. With the default
  `brotli_retention_policy aggressive`, the worker keeps that memory when idle.

### Requests per second

Requests per second are the median of three runs; each difference is relative
to google/ngx_brotli.

| Response | Level | ngx_brotli | ngx_mbrotli, 1 thread | Difference | ngx_mbrotli, 4 threads | Difference |
|---|--:|--:|--:|--:|--:|--:|
| JSON, 1.9 KB | 1 | 52,693 | 52,772 | 0% | 44,612 | -15% |
| JSON, 1.9 KB | 4 | 29,555 | 28,736 | -3% | 44,523 | +51% |
| JSON, 1.9 KB | 6 | 28,549 | 26,247 | -8% | 44,984 | +58% |
| JSON, 1.9 KB | 9 | 24,398 | 23,301 | -4% | 41,700 | +71% |
| JSON, 1.9 KB | 11 | 619 | 897 | +45% | 2,727 | 4.4× |
| jquery.min.js, 85 KB | 1 | 3,012 | 3,719 | +23% | 12,198 | 4.1× |
| jquery.min.js, 85 KB | 4 | 1,138 | 1,134 | 0% | 4,319 | 3.8× |
| jquery.min.js, 85 KB | 6 | 662 | 711 | +7% | 2,642 | 4.0× |
| jquery.min.js, 85 KB | 9 | 108 | 460 | 4.3× | 1,666 | 15.4× |
| jquery.min.js, 85 KB | 11 | 14.8 | 23.7 | +61% | 92.2 | 6.2× |
| JSON, 93 KB | 1 | 3,643 | 4,564 | +25% | 14,912 | 4.1× |
| JSON, 93 KB | 4 | 1,426 | 1,407 | -1% | 5,287 | 3.7× |
| JSON, 93 KB | 6 | 867 | 886 | +2% | 3,251 | 3.8× |
| JSON, 93 KB | 9 | 120 | 544 | 4.5× | 1,970 | 16.4× |
| JSON, 93 KB | 11 | 15.1 | 22.9 | +52% | 88.0 | 5.8× |
| HTML, 166 KB | 1 | 2,243 | 3,041 | +36% | 10,519 | 4.7× |
| HTML, 166 KB | 4 | 965 | 950 | -2% | 3,616 | 3.7× |
| HTML, 166 KB | 6 | 566 | 582 | +3% | 2,177 | 3.8× |
| HTML, 166 KB | 9 | 106 | 371 | 3.5× | 1,297 | 12.3× |
| HTML, 166 KB | 11 | 7.3 | 11.4 | +55% | 44.1 | 6.0× |
| bootstrap.min.css, 227 KB | 1 | 1,936 | 2,844 | +47% | 9,611 | 5.0× |
| bootstrap.min.css, 227 KB | 4 | 847 | 816 | -4% | 3,137 | 3.7× |
| bootstrap.min.css, 227 KB | 6 | 506 | 518 | +2% | 1,965 | 3.9× |
| bootstrap.min.css, 227 KB | 9 | 113 | 315 | 2.8× | 1,142 | 10.1× |
| bootstrap.min.css, 227 KB | 11 | 5.3 | 8.0 | +51% | 31.1 | 5.8× |
| react-dom.development.js, 1.03 MB | 1 | 305 | 409 | +34% | 1,494 | 4.9× |
| react-dom.development.js, 1.03 MB | 4 | 144 | 151 | +5% | 548 | 3.8× |
| react-dom.development.js, 1.03 MB | 6 | 69.1 | 75.3 | +9% | 286 | 4.1× |
| react-dom.development.js, 1.03 MB | 9 | 25.6 | 41.7 | +63% | 132 | 5.1× |
| react-dom.development.js, 1.03 MB | 11 | 1.0 | 1.6 | +57% | 9.9 | 9.9× |

At level 9 the C encoder allocates a 32 MiB hash table (2^15 buckets of 256
entries) for every response over 64 KB and frees it afterwards. That cost does
not depend on the response size, so the difference shrinks as responses grow.

On the 1 MB file at level 11, the pool needs 36% less CPU time per request than
the event loop, and its output is 4% larger: the file is cut into a 1 MiB
segment and a 31 KB one.

### Compressed size

Response bodies in bytes, where they differ; everywhere else all three produce
the same bytes.

| Response | Level | ngx_brotli | ngx_mbrotli, 1 thread | ngx_mbrotli, 4 threads |
|---|--:|--:|--:|--:|
| jquery.min.js, 85 KB | 1 | 37,331 | 35,042 | 35,042 |
| JSON, 93 KB | 1 | 28,563 | 26,112 | 26,112 |
| HTML, 166 KB | 1 | 42,423 | 36,720 | 36,720 |
| bootstrap.min.css, 227 KB | 1 | 45,205 | 39,929 | 39,929 |
| react-dom.development.js, 1.03 MB | 1 | 318,221 | 270,279 | 305,415 |
| react-dom.development.js, 1.03 MB | 4 | 234,457 | 227,902 | 229,925 |
| react-dom.development.js, 1.03 MB | 6 | 210,434 | 207,478 | 211,016 |
| react-dom.development.js, 1.03 MB | 9 | 201,502 | 201,701 | 206,390 |
| react-dom.development.js, 1.03 MB | 11 | 183,001 | 183,001 | 189,942 |

At levels 0–1 the C encoder compresses each call's input on its own, and
google/ngx_brotli calls it once per buffer that NGINX reads, 32 KB by default
(`output_buffers`). That is why ngx_mbrotli's output is smaller there.

On a thread pool, the 1 MB file is cut into two segments that are compressed
independently. With `brotli_parallel_segment_size 16m` it fits in one, and the
output is the same as in the event loop at every level.

### Worker memory

Peak is the worker's highest resident set size during a 6-second load; idle is
its resident set size one second after the load ends. Both are the median of
three runs, each on a freshly started NGINX, and include NGINX itself, about
1.8 MiB before the first request (2.5 MiB with the pool's threads).

| Response | Level | Connections | Module | Peak | Idle |
|---|--:|--:|---|--:|--:|
| JSON, 1.9 KB | 6 | 64 | ngx_brotli | 3.4 MiB | 3.4 MiB |
| | | | ngx_mbrotli, 1 thread | 4.7 MiB | 4.7 MiB |
| | | | ngx_mbrotli, 4 threads | 34.9 MiB | 32.1 MiB |
| jquery.min.js, 85 KB | 6 | 32 | ngx_brotli | 5.6 MiB | 5.6 MiB |
| | | | ngx_mbrotli, 1 thread | 7.1 MiB | 7.1 MiB |
| | | | ngx_mbrotli, 4 threads | 74.8 MiB | 42.4 MiB |
| jquery.min.js, 85 KB | 9 | 32 | ngx_brotli | 33.9 MiB | 3.8 MiB |
| | | | ngx_mbrotli, 1 thread | 9.1 MiB | 9.1 MiB |
| | | | ngx_mbrotli, 4 threads | 98.4 MiB | 70.2 MiB |
| react-dom.development.js, 1.03 MB | 6 | 32 | ngx_brotli | 9.0 MiB | 3.3 MiB |
| | | | ngx_mbrotli, 1 thread | 12.5 MiB | 12.5 MiB |
| | | | ngx_mbrotli, 4 threads | 172.1 MiB | 127.9 MiB |
| react-dom.development.js, 1.03 MB | 11 | 8 | ngx_brotli | 22.9 MiB | 17.7 MiB |
| | | | ngx_mbrotli, 1 thread | 26.8 MiB | 26.8 MiB |
| | | | ngx_mbrotli, 4 threads | 251.2 MiB | 169.1 MiB |

Idle memory includes encoders that ngx_mbrotli keeps for reuse (up to 4 per
worker, `brotli_idle_encoders`) and memory that glibc has not returned to the
system. With the default
`brotli_retention_policy aggressive`, the kept encoders hold all of their
buffers, so ngx_mbrotli's idle memory in the event loop equals its peak;
`bounded <size>` or `release_all` trade some of that memory for allocations on
later responses.

In the event loop, a response is compressed in one go, so a worker needs one
encoder at a time. On a thread pool, a response holds its compressor until it
has been sent, including while its segments wait for a free thread, and the 4
pool threads each need an encoder at once. A worker keeps encoders for at most 4
compressors, idle or in use (`brotli_parallel_compressors`), so that its memory
does not grow with the number of connections; other responses allocate encoders for their segments and free them
once they have been sent. How large the kept encoders are depends on the level
and the segment size: on the 1 MB file, whose 1 MiB segments need the largest
ones, the worker peaks at 172 MiB at level 6 and 251 MiB at level 11.
`brotli_retention_policy bounded <size>` or `release_all` keep less of them.

Idle memory on a thread pool is the kept encoders plus memory that glibc keeps
in the pool threads' malloc arenas after the other responses freed theirs.

<details>
<summary>Hardware, build and method</summary>

**Hardware**

- Intel Core i7-13700KF, 47 GiB RAM.
- Ubuntu 22.04.5 under WSL2, kernel 6.18.33.2-microsoft-standard-WSL2. WSL2
  does not expose which cores are performance cores, so each round runs every
  module back to back, in a different order each round, to spread host noise
  evenly.

**Build**

- NGINX 1.30.5, `./configure --with-compat --with-cc-opt=-O2
  --without-http_rewrite_module --without-pcre --with-threads`, gcc 11.4.0.
- google/ngx_brotli at commit `a71f9312c2deb28875acc7bacfdd5695a111aa53`
  (2023-10-09), as a dynamic module, with its `deps/brotli` submodule at
  `ed738e842d2fbdf2d6459e39267a633c4a9b2f5d` (Brotli 1.1.0).
- Brotli built as static libraries with the flags of CMake's Release build,
  `-O3 -DNDEBUG -fPIC`. google/ngx_brotli's README suggests `-march=native`,
  which was not used; ngx_mbrotli is not built with `target-cpu=native` either.
- ngx_mbrotli 0.3.0 with mbrotli 0.5.10, `cargo build --release` (rustc 1.98.1).

**Configuration**

`worker_processes 1` with `worker_cpu_affinity` set to 4 CPUs on different
physical cores, `access_log off`, `open_file_cache`, `brotli on`, and one
`location` per `brotli_comp_level` serving the same files. For 4 threads,
`thread_pool brotli threads=4` and `brotli_thread_pool brotli`, with the default
`brotli_parallel_segment_size 1m` and `brotli_parallel_tasks 4`. Everything else
is left at its default, including `brotli_window 512k`,
`brotli_retention_policy aggressive`, `brotli_idle_encoders 4` and
`brotli_parallel_compressors 4`.

The 4-thread numbers come from a later run on the same day, with these defaults;
the other modules' numbers come from the first run, where every round ran all
three back to back.

**Load**

- wrk 4.1.0 on 4 other physical cores over loopback, without TLS, sending
  `Accept-Encoding: br`.
- Levels 1–9: 2 threads and 8 connections, for 4 seconds (8 seconds for the 1 MB
  file). Level 11: 4 connections, enough to keep the pool's 4 threads busy, for
  20 seconds (40 seconds for the 1 MB file).
- Each run follows a 1-second warm-up, and starts only once every worker has been
  idle for 300 ms.
- CPU time per request is the worker's user and system time over the run, from
  `/proc/<pid>/stat`, including the pool's threads.
- Every response was decoded with curl's Brotli decoder (Brotli 1.0.9) and
  compared with the original file.

**Files**

jQuery 3.7.1 (`jquery.min.js`), Bootstrap 5.3.3 (`bootstrap.min.css`) and React
DOM 18.3.1 (`react-dom.development.js`) from cdnjs, the English Wikipedia page
"HTTP compression", the cdnjs API listing
`/libraries?fields=description&limit=500` (93 KB), and its first 10 entries
(1.9 KB). The page and the listing were fetched on 2026-10-12.

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
