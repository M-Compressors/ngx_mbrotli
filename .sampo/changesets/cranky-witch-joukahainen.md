---
cargo/ngx_mbrotli: minor
---

Initial release: `ngx_http_brotli_filter_module` and `ngx_http_brotli_static_module`, a drop-in replacement for google/ngx_brotli built on ngx-rust and the pure-Rust mbrotli encoder. Supports every ngx_brotli directive (`brotli`, `brotli_types`, `brotli_buffers`, `brotli_comp_level`, `brotli_window`, `brotli_min_length`, `brotli_static`) and the `$brotli_ratio` variable.
