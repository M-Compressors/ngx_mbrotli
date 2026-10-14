---
cargo/ngx_mbrotli: patch
---

Choose zstd over Brotli when NGINX also has zstd-nginx-module and the client accepts both encodings. `load_module` used to append `ngx_http_brotli_static_module` to the end of the module list, so a `.br` file was served even when a `.zst` file was there too, unlike a static build with ngx_brotli added before zstd-nginx-module. Both modules are now ordered before their zstd counterparts, for pre-compressed files and for compression on the fly.
