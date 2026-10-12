---
cargo/ngx_mbrotli: minor
---

Add `brotli_parallel_compressors <number>` (default `4`, `http` context): with `brotli_thread_pool`, a worker keeps segment encoders for at most that many compressors, idle or held by responses. A response holds its compressor while its segments wait for a thread, so before this every response in flight could hold the encoders a previous one grew, and a worker's memory grew with the number of connections. Requires NGINX built with `--with-threads`. Add `brotli_idle_encoders <number>` (default `4`, `http` context) to set how many idle encoders each worker keeps, which was fixed at 4.
