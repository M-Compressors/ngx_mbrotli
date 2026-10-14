---
cargo/ngx_mbrotli: patch
---

Update mbrotli to 0.5.15. With `brotli_thread_pool`, a response longer than `brotli_parallel_segment_size` now compresses about as well as without a pool: segments after the first no longer write every copy distance in full, and each may copy from the quarter segment of the response before it. At `brotli_comp_level 6` with the default 512k window and 1m segments, 8 MB of repetitive text is 430,507 bytes instead of 2.8 MB (430,026 without a pool), and 8 MB of WebAssembly 1,756,368 instead of 2,001,714 (1,750,366). Responses that fit in one segment are unchanged.
