# ngx_mbrotli

## 0.3.0 — 2026-10-11

### Minor changes

- [1dd6380](https://github.com/M-Compressors/ngx_mbrotli/commit/1dd6380c57fce0546b311e8d4c7a363879c49d4d) Compress uncompressed TrueType and OpenType fonts (`font/ttf`, `font/otf`, `application/vnd.ms-fontobject` and similar types) in Brotli's font mode, which makes them about 0.1–0.3% smaller. Other responses, including WOFF and WOFF2 fonts, are still compressed in generic mode. — Thanks @Mnwa!
- [60afc80](https://github.com/M-Compressors/ngx_mbrotli/commit/60afc801b58c7b7ab864de9970389a07922aaf2a) `brotli_retention_policy` now defaults to `aggressive`: an idle encoder keeps all of its working memory, so a worker allocates only what its largest responses grew into. Set `brotli_retention_policy bounded 8m` for the previous default. — Thanks @Mnwa!

### Patch changes

- [60afc80](https://github.com/M-Compressors/ngx_mbrotli/commit/60afc801b58c7b7ab864de9970389a07922aaf2a) Update mbrotli to 0.5.9, which makes `brotli_comp_level 4` 1–2% faster on 2 KB–1 MB responses. Output is unchanged. — Thanks @Mnwa!

## 0.2.0 — 2026-10-10

### Minor changes

- [a6a6bc8](https://github.com/M-Compressors/ngx_mbrotli/commit/a6a6bc89ac20c2abcaec9e2f4c417d32141dbe0c) Add `brotli_reuse_memory on|off` (default `on`) to reuse encoders between responses in each worker, and `brotli_retention_policy aggressive | current_config | release_all | bounded <size>` (default `bounded 8m`) to choose how much working memory an idle encoder keeps. — Thanks @Mnwa!

### Patch changes

- [19958bc](https://github.com/M-Compressors/ngx_mbrotli/commit/19958bc08e48a59bf34cdfd7084bdfb9b3482a58) Fix an out-of-bounds read in `brotli_static`: the name of the `.br` file was given a length past its end, so logging it could read beyond the allocation. — Thanks @Mnwa!
- [7723049](https://github.com/M-Compressors/ngx_mbrotli/commit/7723049337a136209bbc3627dc5be75c38a23a19) Update mbrotli to 0.5.7 and strip symbols from the release build for a smaller module binary. — Thanks @Mnwa!

## 0.1.0 — 2026-10-10

### Minor changes

- [fec5f38](https://github.com/M-Compressors/ngx_mbrotli/commit/fec5f383f9af991908921524dd1f219043d52d80) Initial release: `ngx_http_brotli_filter_module` and `ngx_http_brotli_static_module`, a drop-in replacement for google/ngx_brotli built on ngx-rust and the pure-Rust mbrotli encoder. Supports every ngx_brotli directive (`brotli`, `brotli_types`, `brotli_buffers`, `brotli_comp_level`, `brotli_window`, `brotli_min_length`, `brotli_static`) and the `$brotli_ratio` variable. — Thanks @Mnwa!

