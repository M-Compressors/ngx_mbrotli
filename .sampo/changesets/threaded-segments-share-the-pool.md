---
cargo/ngx_mbrotli: minor
---

Add `brotli_thread_pool <name> | off` (default `off`) to compress responses on an NGINX thread pool instead of in the worker's event loop, with `brotli_parallel_segment_size` (default `1m`) and `brotli_parallel_tasks` (default `4`) to cut longer responses into segments that are compressed in parallel. Requires NGINX built with `--with-threads`. Update mbrotli to 0.5.10.
