//! NGINX Brotli modules built on [ngx-rust](https://github.com/nginx/ngx-rust) and
//! [mbrotli](https://github.com/M-Compressors/mbrotli).
//!
//! A drop-in replacement for google/ngx_brotli, exposing the same directives and
//! variables from two modules:
//!
//! - `ngx_http_brotli_filter_module` compresses responses on the fly;
//! - `ngx_http_brotli_static_module` serves pre-compressed `.br` files.

mod accept_encoding;
mod encoder_pool;
mod ratio;

// Everything below refers to NGINX symbols that only exist once the module is
// loaded into NGINX, so unit tests build just the self-contained parts above.
#[cfg(not(test))]
mod bitfields;
#[cfg(not(test))]
mod filter;
#[cfg(not(test))]
mod static_module;

#[cfg(not(test))]
pub use filter::ngx_http_brotli_filter_module;
#[cfg(not(test))]
pub use static_module::ngx_http_brotli_static_module;

/// Export tables read by NGINX's `load_module`.
///
/// Written out by hand rather than with `ngx::ngx_modules!` because the filter
/// must be ordered next to the gzip filter: a dynamic module without an order
/// is appended to the end of the module list, which would put the brotli body
/// filter ahead of the copy filter that reads file buffers into memory.
#[cfg(all(feature = "export-modules", not(test)))]
mod exports {
    use core::ffi::c_char;
    use core::ptr;

    use ngx::ffi::ngx_module_t;

    use crate::{ngx_http_brotli_filter_module, ngx_http_brotli_static_module};

    #[unsafe(no_mangle)]
    #[allow(non_upper_case_globals)]
    pub static mut ngx_modules: [*const ngx_module_t; 3] = [
        &raw const ngx_http_brotli_filter_module,
        &raw const ngx_http_brotli_static_module,
        ptr::null(),
    ];

    #[unsafe(no_mangle)]
    #[allow(non_upper_case_globals)]
    pub static mut ngx_module_names: [*const c_char; 3] = [
        c"ngx_http_brotli_filter_module".as_ptr(),
        c"ngx_http_brotli_static_module".as_ptr(),
        ptr::null(),
    ];

    /// Same order as google/ngx_brotli's `config`: the filter is inserted
    /// before the first of the listed modules that is present, i.e. right
    /// after the gzip filter.
    #[unsafe(no_mangle)]
    #[allow(non_upper_case_globals)]
    pub static mut ngx_module_order: [*const c_char; 17] = [
        c"ngx_http_brotli_filter_module".as_ptr(),
        c"ngx_pagespeed".as_ptr(),
        c"ngx_http_postpone_filter_module".as_ptr(),
        c"ngx_http_ssi_filter_module".as_ptr(),
        c"ngx_http_charset_filter_module".as_ptr(),
        c"ngx_http_xslt_filter_module".as_ptr(),
        c"ngx_http_image_filter_module".as_ptr(),
        c"ngx_http_sub_filter_module".as_ptr(),
        c"ngx_http_addition_filter_module".as_ptr(),
        c"ngx_http_gunzip_filter_module".as_ptr(),
        c"ngx_http_userid_filter_module".as_ptr(),
        c"ngx_http_headers_filter_module".as_ptr(),
        c"ngx_http_copy_filter_module".as_ptr(),
        c"ngx_http_range_body_filter_module".as_ptr(),
        c"ngx_http_not_modified_filter_module".as_ptr(),
        c"ngx_http_slice_filter_module".as_ptr(),
        ptr::null(),
    ];
}
