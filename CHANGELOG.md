# ngx_mbrotli

## 0.1.0 — 2026-10-10

### Minor changes

- [fec5f38](https://github.com/M-Compressors/ngx_mbrotli/commit/fec5f383f9af991908921524dd1f219043d52d80) Initial release: `ngx_http_brotli_filter_module` and `ngx_http_brotli_static_module`, a drop-in replacement for google/ngx_brotli built on ngx-rust and the pure-Rust mbrotli encoder. Supports every ngx_brotli directive (`brotli`, `brotli_types`, `brotli_buffers`, `brotli_comp_level`, `brotli_window`, `brotli_min_length`, `brotli_static`) and the `$brotli_ratio` variable. — Thanks @Mnwa!

