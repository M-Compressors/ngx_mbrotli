---
cargo/ngx_mbrotli: patch
---

The body filter now checks that the encoder never reports consuming more input or producing more output than the buffers it was given, before moving the buffers' pointers. A response that would break this fails with "brotli encoder failed: progress out of bounds" in the error log instead of sending memory past the end of an output buffer. mbrotli has never reported such counts, so responses are unchanged.
