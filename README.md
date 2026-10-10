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
- The pure-Rust mbrotli encoder replaces the C library. It produces the same
  bytes as Google's encoder at every level, so you do not need to vendor or build
  the C Brotli library.

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
rustfmt, clippy, Miri over the unit tests, and the unit and integration tests
against NGINX 1.28, 1.30 and 1.31.

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
