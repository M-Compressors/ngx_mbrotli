//! Build script.
//!
//! Compiles the C bit-field accessors (`src/bitfields.c`) against the NGINX
//! headers that `nginx-sys` generated its bindings from. `nginx-sys` must be a
//! direct dependency for its `DEP_NGINX_*` metadata to reach this script:
//! <https://github.com/rust-lang/cargo/issues/3544>.

use std::env;
use std::path::Path;

fn main() {
    println!("cargo::rerun-if-changed=src/bitfields.c");
    println!("cargo::rerun-if-env-changed=DEP_NGINX_INCLUDE");
    // Re-running `configure` changes the layout without changing the paths.
    if let Some(dir) = env::var_os("DEP_NGINX_BUILD_DIR") {
        let config = Path::new(&dir).join("ngx_auto_config.h");
        println!("cargo::rerun-if-changed={}", config.display());
    }

    let includes = env::var_os("DEP_NGINX_INCLUDE").expect("nginx-sys exports DEP_NGINX_INCLUDE");

    cc::Build::new()
        .file("src/bitfields.c")
        .includes(env::split_paths(&includes))
        // NGINX headers are not warning-clean under every compiler's defaults.
        .warnings(false)
        .compile("ngx_mbrotli_bitfields");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // NGINX symbols are resolved when the module is loaded.
        println!("cargo::rustc-link-arg=-undefined");
        println!("cargo::rustc-link-arg=dynamic_lookup");
    }
}
