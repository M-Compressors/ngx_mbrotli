# ngx_mbrotli

## 0.2.0 — 2026-10-10

### Minor changes

- [a6a6bc8](https://github.com/M-Compressors/ngx_mbrotli/commit/a6a6bc89ac20c2abcaec9e2f4c417d32141dbe0c) Add `brotli_reuse_memory on|off` (default `on`) to reuse encoders between responses in each worker, and `brotli_retention_policy aggressive | current_config | release_all | bounded <size>` (default `bounded 8m`) to choose how much working memory an idle encoder keeps. — Thanks @Mnwa!

### Patch changes

- [19958bc](https://github.com/M-Compressors/ngx_mbrotli/commit/19958bc08e48a59bf34cdfd7084bdfb9b3482a58) Fix an out-of-bounds read in `brotli_static`: the name of the `.br` file was given a length past its end, so logging it could read beyond the allocation. — Thanks @Mnwa!
- [7723049](https://github.com/M-Compressors/ngx_mbrotli/commit/7723049337a136209bbc3627dc5be75c38a23a19) Update mbrotli to 0.5.7 and strip symbols from the release build for a smaller module binary. — Thanks @Mnwa!

## 0.1.0 — 2026-10-10

### Minor changes

- [fec5f38](https://github.com/M-Compressors/ngx_mbrotli/commit/fec5f383f9af991908921524dd1f219043d52d80) Initial release: `ngx_http_brotli_filter_module` and `ngx_http_brotli_static_module`, a drop-in replacement for google/ngx_brotli built on ngx-rust and the pure-Rust mbrotli encoder. Supports every ngx_brotli directive (`brotli`, `brotli_types`, `brotli_buffers`, `brotli_comp_level`, `brotli_window`, `brotli_min_length`, `brotli_static`) and the `$brotli_ratio` variable. — Thanks @Mnwa!

