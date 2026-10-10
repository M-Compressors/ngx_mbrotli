---
cargo/ngx_mbrotli: patch
---

Fix an out-of-bounds read in `brotli_static`: the name of the `.br` file was given a length past its end, so logging it could read beyond the allocation.
