---
cargo/ngx_mbrotli: patch
---

Releases now include prebuilt modules for x86_64 and aarch64. They cover NGINX 1.24–1.31 from nginx.org, the official Docker images, Debian 13, Ubuntu 24.04 and 26.04, RHEL 8–10, Fedora, Amazon Linux 2023 and Alpine 3.21–3.24, and each one is tested in that distribution's own nginx. The build script now also recompiles the C bit-field accessors when NGINX is reconfigured in the same directory. `install.sh` installs the right one with `curl … | sh`: it verifies the download against the release's SHA256SUMS and nginx's module signature, loads the module and checks the configuration with `nginx -t`.
