---
cargo/ngx_mbrotli: minor
---

`brotli_retention_policy` now defaults to `aggressive`: an idle encoder keeps all of its working memory, so a worker allocates only what its largest responses grew into. Set `brotli_retention_policy bounded 8m` for the previous default.
