# ngx_mbrotli

## 0.4.4 — 2026-10-18

### Patch changes

- Update mbrotli with a new perf optiomizations

## 0.4.3 — 2026-10-14

### Patch changes

- [78ecdf3](https://github.com/M-Compressors/ngx_mbrotli/commit/78ecdf3912850d2a0991a7a467098e5ffdc06c79) Update mbrotli to 0.5.15. With `brotli_thread_pool`, a response longer than `brotli_parallel_segment_size` now compresses about as well as without a pool: segments after the first no longer write every copy distance in full, and each may copy from the quarter segment of the response before it. At `brotli_comp_level 6` with the default 512k window and 1m segments, 8 MB of repetitive text is 430,507 bytes instead of 2.8 MB (430,026 without a pool), and 8 MB of WebAssembly 1,756,368 instead of 2,001,714 (1,750,366). Responses that fit in one segment are unchanged. — Thanks @Mnwa!

## 0.4.2 — 2026-10-14

### Patch changes

- [ce2ed65](https://github.com/M-Compressors/ngx_mbrotli/commit/ce2ed65550675855a555a77b05d57843016ed5cc) Choose zstd over Brotli when NGINX also has zstd-nginx-module and the client accepts both encodings. `load_module` used to append `ngx_http_brotli_static_module` to the end of the module list, so a `.br` file was served even when a `.zst` file was there too, unlike a static build with ngx_brotli added before zstd-nginx-module. Both modules are now ordered before their zstd counterparts, for pre-compressed files and for compression on the fly. — Thanks @Mnwa!

## 0.4.1 — 2026-10-13

### Patch changes

- [eed3503](https://github.com/M-Compressors/ngx_mbrotli/commit/eed350374d52a3e2d25d924aadadcf0b20b4aa6b) Update mbrotli to 0.5.11. With `brotli_thread_pool`, the encoder now writes straight into uninitialized output buffers, as it already did without a thread pool, so each new `brotli_buffers` buffer is no longer zeroed first. Output is unchanged. — Thanks @Mnwa!

## 0.4.0 — 2026-10-12

### Minor changes

- [e2a96d5](https://github.com/M-Compressors/ngx_mbrotli/commit/e2a96d5df1931adef08de669ec90d865af792f2b) Add `brotli_thread_pool <name> | off` (default `off`) to compress responses on an NGINX thread pool instead of in the worker's event loop, with `brotli_parallel_segment_size` (default `1m`) and `brotli_parallel_tasks` (default `4`) to cut longer responses into segments that are compressed in parallel. Requires NGINX built with `--with-threads`. Update mbrotli to 0.5.10. — Thanks @Mnwa!
- [8f7508a](https://github.com/M-Compressors/ngx_mbrotli/commit/8f7508afbda7cc0717de8a04fe22ec10dfe37a22) Add `brotli_parallel_compressors <number>` (default `4`, `http` context): with `brotli_thread_pool`, a worker keeps segment encoders for at most that many compressors, idle or held by responses. A response holds its compressor while its segments wait for a thread, so before this every response in flight could hold the encoders a previous one grew, and a worker's memory grew with the number of connections. Requires NGINX built with `--with-threads`. Add `brotli_idle_encoders <number>` (default `4`, `http` context) to set how many idle encoders each worker keeps, which was fixed at 4. — Thanks @Mnwa!

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

