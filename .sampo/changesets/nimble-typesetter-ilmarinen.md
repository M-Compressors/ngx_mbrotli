---
cargo/ngx_mbrotli: minor
---

Compress uncompressed TrueType and OpenType fonts (`font/ttf`, `font/otf`, `application/vnd.ms-fontobject` and similar types) in Brotli's font mode, which makes them about 0.1–0.3% smaller. Other responses, including WOFF and WOFF2 fonts, are still compressed in generic mode.
