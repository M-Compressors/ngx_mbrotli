---
cargo/ngx_mbrotli: minor
---

Add `brotli_reuse_memory on|off` (default `on`) to reuse encoders between responses in each worker, and `brotli_retention_policy aggressive | current_config | release_all | bounded <size>` (default `bounded 8m`) to choose how much working memory an idle encoder keeps.
