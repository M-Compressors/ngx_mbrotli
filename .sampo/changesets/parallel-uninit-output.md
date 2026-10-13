---
cargo/ngx_mbrotli: patch
---

Update mbrotli to 0.5.11. With `brotli_thread_pool`, the encoder now writes straight into uninitialized output buffers, as it already did without a thread pool, so each new `brotli_buffers` buffer is no longer zeroed first. Output is unchanged.
