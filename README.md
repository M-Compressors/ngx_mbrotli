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
keeps up to 4 idle encoders, so a response can reuse the window, match finder and
buffers a previous one allocated. How much memory an idle encoder keeps is set by
`brotli_retention_policy`. When off, every response gets a new encoder that is
freed when the response is complete, as with google/ngx_brotli.

*Not in google/ngx_brotli.*

### `brotli_retention_policy`

- **syntax**: `brotli_retention_policy aggressive | current_config | release_all | bounded <size>`
- **default**: `bounded 8m`
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
worker's 4 idle slots are full, the least recently used encoder is freed.

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
- **Encoders are reused** by default (`brotli_reuse_memory`), each keeping at
  most 8 MiB of working memory between responses (`brotli_retention_policy`).
  Set `brotli_reuse_memory off` to allocate per response as google/ngx_brotli
  does. Peak memory still follows the number of responses being compressed at
  once, times the working set that `brotli_comp_level` and `brotli_window` need.
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

One NGINX worker compressing static files on the fly with ngx_mbrotli and with
google/ngx_brotli, as each is shipped: google/ngx_brotli at commit
[`a71f9312`](https://github.com/google/ngx_brotli/commit/a71f9312c2deb28875acc7bacfdd5695a111aa53)
with the Brotli 1.1.0 it pins as a submodule. Both run with default settings apart
from `brotli_comp_level`, so ngx_mbrotli reuses encoders
(`brotli_reuse_memory on`).

- **Level 11 is 38–63% faster** at every response size.
- **Level 9 is 2.6–4.3 times faster** on 85–230 KB responses and 21% faster at
  1 MB, but 8% slower on 2 KB responses.
- **Level 6, the default, is 2–7% faster** on 85 KB–1 MB responses, but 12%
  slower on 2 KB responses.
- **Level 4 is 1–5% slower**, except at 1 MB, where it is 6% faster.
- **Level 1 is 17–41% faster** on responses over 32 KB, and its output is smaller.
- **Output matches byte for byte** at levels 2–11. The exceptions are levels 0–1,
  where ngx_mbrotli's output is 3–15% smaller on responses over 32 KB, and
  responses of 1 MB or more at levels 4–9, where the size hint selects a
  different match finder.
- **A worker uses 2–5 MiB more memory**, except at level 9, where its peak is
  under a third of google/ngx_brotli's.

### Requests per second and compressed size

Requests per second are the median of three runs; the difference is ngx_mbrotli
relative to google/ngx_brotli. Sizes are response bodies in bytes.

| Response | Level | ngx_brotli, req/s | ngx_mbrotli, req/s | Difference | ngx_brotli, bytes | ngx_mbrotli, bytes |
|---|--:|--:|--:|--:|--:|--:|
| JSON, 1.9 KB | 1 | 51,825 | 52,403 | +1% | 717 | 717 |
| JSON, 1.9 KB | 4 | 29,265 | 28,510 | -3% | 647 | 647 |
| JSON, 1.9 KB | 6 | 27,783 | 24,351 | -12% | 575 | 575 |
| JSON, 1.9 KB | 9 | 23,773 | 21,905 | -8% | 576 | 576 |
| JSON, 1.9 KB | 11 | 624 | 861 | +38% | 504 | 504 |
| jquery.min.js, 85 KB | 1 | 3,042 | 3,574 | +17% | 37,331 | 35,042 |
| jquery.min.js, 85 KB | 4 | 1,142 | 1,126 | -1% | 31,402 | 31,402 |
| jquery.min.js, 85 KB | 6 | 670 | 716 | +7% | 29,596 | 29,596 |
| jquery.min.js, 85 KB | 9 | 111 | 466 | +322% | 29,478 | 29,478 |
| jquery.min.js, 85 KB | 11 | 15.0 | 24.4 | +63% | 27,446 | 27,446 |
| JSON, 94 KB | 1 | 3,603 | 4,362 | +21% | 28,756 | 26,228 |
| JSON, 94 KB | 4 | 1,425 | 1,375 | -3% | 23,473 | 23,473 |
| JSON, 94 KB | 6 | 850 | 865 | +2% | 21,813 | 21,813 |
| JSON, 94 KB | 9 | 127 | 547 | +331% | 21,528 | 21,528 |
| JSON, 94 KB | 11 | 14.7 | 22.4 | +52% | 19,346 | 19,346 |
| HTML, 166 KB | 1 | 2,253 | 2,958 | +31% | 42,564 | 36,817 |
| HTML, 166 KB | 4 | 974 | 943 | -3% | 31,435 | 31,435 |
| HTML, 166 KB | 6 | 568 | 581 | +2% | 28,786 | 28,786 |
| HTML, 166 KB | 9 | 105 | 330 | +215% | 28,370 | 28,370 |
| HTML, 166 KB | 11 | 7.5 | 11.6 | +55% | 25,456 | 25,456 |
| bootstrap.min.css, 227 KB | 1 | 1,931 | 2,729 | +41% | 45,205 | 39,929 |
| bootstrap.min.css, 227 KB | 4 | 856 | 816 | -5% | 33,206 | 33,206 |
| bootstrap.min.css, 227 KB | 6 | 509 | 523 | +3% | 27,432 | 27,432 |
| bootstrap.min.css, 227 KB | 9 | 113 | 289 | +155% | 26,561 | 26,561 |
| bootstrap.min.css, 227 KB | 11 | 5.5 | 8.1 | +47% | 22,709 | 22,709 |
| react-dom.development.js, 1.03 MB | 1 | 309 | 404 | +31% | 318,221 | 270,279 |
| react-dom.development.js, 1.03 MB | 4 | 145 | 153 | +6% | 234,457 | 227,902 |
| react-dom.development.js, 1.03 MB | 6 | 69.8 | 73.9 | +6% | 210,434 | 207,478 |
| react-dom.development.js, 1.03 MB | 9 | 25.5 | 30.8 | +21% | 201,502 | 201,701 |
| react-dom.development.js, 1.03 MB | 11 | 1.0 | 1.6 | +60% | 183,001 | 183,001 |

At level 9 the C encoder allocates a 32 MiB hash table (2^15 buckets of 256
entries) for every response over 64 KB and frees it afterwards. That cost does
not depend on the response size, so the difference shrinks as responses grow.

At levels 0–1 the C encoder compresses each call's input on its own, and
google/ngx_brotli calls it once per buffer that NGINX reads, 32 KB by default
(`output_buffers`). That is why ngx_mbrotli's output is smaller there.

### Worker memory

Peak is the worker's highest resident set size during a 6-second load; idle is
its resident set size one second after the load ends. Both are the median of
three runs and include NGINX itself, about 1.8 MiB before the first request.

| Response | Level | Connections | ngx_brotli, peak | ngx_mbrotli, peak | ngx_brotli, idle | ngx_mbrotli, idle |
|---|--:|--:|--:|--:|--:|--:|
| JSON, 1.9 KB | 6 | 64 | 3.3 MiB | 5.4 MiB | 3.3 MiB | 5.4 MiB |
| jquery.min.js, 85 KB | 6 | 32 | 5.7 MiB | 8.0 MiB | 5.7 MiB | 8.0 MiB |
| jquery.min.js, 85 KB | 9 | 32 | 33.9 MiB | 9.9 MiB | 3.7 MiB | 9.9 MiB |
| react-dom.development.js, 1.03 MB | 6 | 32 | 9.2 MiB | 13.6 MiB | 3.4 MiB | 7.7 MiB |
| react-dom.development.js, 1.03 MB | 11 | 8 | 23.0 MiB | 27.8 MiB | 17.2 MiB | 22.2 MiB |

Idle memory includes encoders that ngx_mbrotli keeps for reuse (up to 4 per
worker, see `brotli_retention_policy`) and memory that glibc has not returned to
the system. After level 11 it varied between runs, from 17 to 21 MiB for
google/ngx_brotli and from 19 to 23 MiB for ngx_mbrotli.

<details>
<summary>Hardware, build and method</summary>

**Hardware**

- Intel Core i7-13700KF, 47 GiB RAM.
- Ubuntu 22.04.5 under WSL2, kernel 6.18.33.2-microsoft-standard-WSL2. WSL2
  does not expose which cores are performance cores, so each round runs every
  module back to back to spread host noise evenly.

**Build**

- NGINX 1.30.5, `./configure --with-compat --with-cc-opt=-O2
  --without-http_rewrite_module --without-pcre`, gcc 11.4.0.
- google/ngx_brotli at commit `a71f9312c2deb28875acc7bacfdd5695a111aa53`
  (2023-10-09), as a dynamic module, with its `deps/brotli` submodule at
  `ed738e842d2fbdf2d6459e39267a633c4a9b2f5d` (Brotli 1.1.0).
- Brotli built as static libraries with CMake's Release flags,
  `-O3 -DNDEBUG -fPIC`. google/ngx_brotli's README suggests `-march=native`,
  which was not used; ngx_mbrotli is not built with `target-cpu=native` either.
- ngx_mbrotli 0.2.0 with mbrotli 0.5.7, `cargo build --release` (rustc 1.98.1).

**Configuration**

`worker_processes 1` pinned to one CPU, `access_log off`, `open_file_cache`,
`brotli on`, and one `location` per `brotli_comp_level` serving the same files.
Everything else is left at its default, including `brotli_window 512k`.

**Load**

- wrk 4.1.0 on 4 other CPUs over loopback, without TLS, sending
  `Accept-Encoding: br`.
- Levels 1–9: 2 threads and 8 connections, for 4 seconds (8 seconds for the 1 MB
  file). Levels 10–11: 2 connections for 15 seconds.
- Each run follows a 1-second warm-up, and starts only once every worker has been
  idle for 300 ms.
- Every response was decoded with the Brotli 1.2.0 CLI and compared with the
  original file.

**Files**

jQuery 3.7.1 (`jquery.min.js`), Bootstrap 5.3.3 (`bootstrap.min.css`) and React
DOM 18.3.1 (`react-dom.development.js`) from cdnjs, the English Wikipedia page
"HTTP compression", a cdnjs API listing (94 KB), and its first entries (1.9 KB).

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
- unit and integration tests against NGINX 1.28, 1.30 and 1.31;
- the integration tests under AddressSanitizer and LeakSanitizer, with NGINX
  and the module (including the Rust standard library) instrumented, and
  `NGX_DEBUG_PALLOC` so that each pool allocation is checked on its own;
- the unit tests under Miri. The body filter (`src/body.rs`) reaches NGINX only
  through a small `Host` trait, so Miri runs it against a model of NGINX's
  buffer and chain API (`src/body/tests.rs`). The model frees drained input
  buffers and chain links, and leaves output buffers uninitialized, so stale
  pointers and reads of unwritten output show up as undefined behaviour.

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

## License

MIT; see [LICENSE](LICENSE). The directives and their behaviour follow
[google/ngx_brotli](https://github.com/google/ngx_brotli) (BSD 2-Clause).
