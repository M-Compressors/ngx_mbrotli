//! `ngx_http_brotli_filter_module`: on-the-fly Brotli compression of responses.
//!
//! Directives and behaviour follow google/ngx_brotli; output buffering follows
//! the stock gzip filter, so `brotli_buffers` is honoured rather than ignored.

use core::ffi::{c_char, c_void};
use core::{fmt, ptr};

use mbrotli::{ConfigError, EncoderConfig, InputSize, Quality, RetentionPolicy, Window};
use ngx::core::{NGX_CONF_ERROR, NGX_CONF_OK, Pool};
use ngx::ffi::{
    NGX_CONF_1MORE, NGX_CONF_FLAG, NGX_CONF_TAKE1, NGX_CONF_TAKE2, NGX_CONF_TAKE12, NGX_ERROR,
    NGX_HTTP_FORBIDDEN, NGX_HTTP_GZIP_BUFFERED, NGX_HTTP_LIF_CONF, NGX_HTTP_LOC_CONF,
    NGX_HTTP_LOC_CONF_OFFSET, NGX_HTTP_MAIN_CONF, NGX_HTTP_MODULE, NGX_HTTP_NOT_FOUND, NGX_HTTP_OK,
    NGX_HTTP_SRV_CONF, NGX_LOG_ALERT, NGX_LOG_EMERG, NGX_OK, ngx_alloc_chain_link, ngx_array_t,
    ngx_buf_t, ngx_bufs_t, ngx_chain_add_copy, ngx_chain_t, ngx_chain_update_chains, ngx_command_t,
    ngx_conf_check_num_bounds, ngx_conf_num_bounds_t, ngx_conf_post_t, ngx_conf_set_bufs_slot,
    ngx_conf_set_flag_slot, ngx_conf_set_num_slot, ngx_conf_set_size_slot, ngx_conf_t,
    ngx_create_temp_buf, ngx_flag_t, ngx_hash_t, ngx_http_add_variable,
    ngx_http_html_default_types, ngx_http_merge_types, ngx_http_module_t,
    ngx_http_output_body_filter_pt, ngx_http_output_header_filter_pt, ngx_http_request_t,
    ngx_http_test_content_type, ngx_http_top_body_filter, ngx_http_top_header_filter,
    ngx_http_types_slot, ngx_http_variable_value_t, ngx_int_t, ngx_list_push, ngx_log_t,
    ngx_module_t, ngx_pagesize, ngx_palloc, ngx_parse_size, ngx_pcalloc, ngx_pnalloc, ngx_pool_t,
    ngx_str_t, ngx_table_elt_t, ngx_uint_t, off_t,
};
use ngx::{ngx_conf_log_error, ngx_log_debug_mask, ngx_log_error, ngx_string};

use crate::accept_encoding;
use crate::bitfields::{
    buf_flags, ngx_mbrotli_buf_set_flush, ngx_mbrotli_buf_set_last_buf,
    ngx_mbrotli_buf_set_recycled, ngx_mbrotli_r_header_only, ngx_mbrotli_r_prepare_encoded,
    ngx_mbrotli_r_set_buffered, ngx_mbrotli_r_set_gzip_vary, ngx_mbrotli_var_set,
    ngx_mbrotli_var_set_not_found,
};
use crate::body::{self, BodyFilter, BufFlags, Host, Settings};
use crate::encoder_pool;
use crate::ratio;

/// The gzip and brotli filters never both encode a response, so they can
/// share the "buffered" bit, exactly as google/ngx_brotli does.
const BROTLI_BUFFERED: u32 = NGX_HTTP_GZIP_BUFFERED;

const MIN_WINDOW_BITS: usize = Window::MIN_BITS as usize;
const MAX_WINDOW_BITS: usize = Window::MAX_STANDARD_BITS as usize;

const UNSET: isize = -1;
const UNSET_SIZE: usize = usize::MAX;

/// Location configuration. Field offsets are used by the stock NGINX slot
/// setters, so the layout must stay `repr(C)`.
#[repr(C)]
struct FilterConf {
    enable: ngx_flag_t,
    types: ngx_hash_t,
    types_keys: *mut ngx_array_t,
    min_length: isize,
    bufs: ngx_bufs_t,
    quality: ngx_int_t,
    lg_win: usize,
    reuse_memory: ngx_flag_t,
    /// `None` until `brotli_retention_policy` is set or merged.
    retention: Option<RetentionPolicy>,
}

static mut COMP_LEVEL_BOUNDS: ngx_conf_num_bounds_t = ngx_conf_num_bounds_t {
    post_handler: Some(ngx_conf_check_num_bounds),
    low: Quality::MIN.get() as ngx_int_t,
    high: Quality::MAX.get() as ngx_int_t,
};

static mut WINDOW_POST: ngx_conf_post_t = ngx_conf_post_t {
    post_handler: Some(parse_window_bits),
};

const ANY_CONF: u32 = NGX_HTTP_MAIN_CONF | NGX_HTTP_SRV_CONF | NGX_HTTP_LOC_CONF;

static mut COMMANDS: [ngx_command_t; 9] = [
    ngx_command_t {
        name: ngx_string!("brotli"),
        type_: (ANY_CONF | NGX_HTTP_LIF_CONF | NGX_CONF_FLAG) as ngx_uint_t,
        set: Some(ngx_conf_set_flag_slot),
        conf: NGX_HTTP_LOC_CONF_OFFSET,
        offset: core::mem::offset_of!(FilterConf, enable),
        post: ptr::null_mut(),
    },
    ngx_command_t {
        name: ngx_string!("brotli_buffers"),
        type_: (ANY_CONF | NGX_CONF_TAKE2) as ngx_uint_t,
        set: Some(ngx_conf_set_bufs_slot),
        conf: NGX_HTTP_LOC_CONF_OFFSET,
        offset: core::mem::offset_of!(FilterConf, bufs),
        post: ptr::null_mut(),
    },
    ngx_command_t {
        name: ngx_string!("brotli_types"),
        type_: (ANY_CONF | NGX_CONF_1MORE) as ngx_uint_t,
        set: Some(ngx_http_types_slot),
        conf: NGX_HTTP_LOC_CONF_OFFSET,
        offset: core::mem::offset_of!(FilterConf, types_keys),
        post: &raw mut ngx_http_html_default_types as *mut c_void,
    },
    ngx_command_t {
        name: ngx_string!("brotli_comp_level"),
        type_: (ANY_CONF | NGX_CONF_TAKE1) as ngx_uint_t,
        set: Some(ngx_conf_set_num_slot),
        conf: NGX_HTTP_LOC_CONF_OFFSET,
        offset: core::mem::offset_of!(FilterConf, quality),
        post: &raw mut COMP_LEVEL_BOUNDS as *mut c_void,
    },
    ngx_command_t {
        name: ngx_string!("brotli_window"),
        type_: (ANY_CONF | NGX_CONF_TAKE1) as ngx_uint_t,
        set: Some(ngx_conf_set_size_slot),
        conf: NGX_HTTP_LOC_CONF_OFFSET,
        offset: core::mem::offset_of!(FilterConf, lg_win),
        post: &raw mut WINDOW_POST as *mut c_void,
    },
    ngx_command_t {
        name: ngx_string!("brotli_min_length"),
        type_: (ANY_CONF | NGX_CONF_TAKE1) as ngx_uint_t,
        set: Some(ngx_conf_set_size_slot),
        conf: NGX_HTTP_LOC_CONF_OFFSET,
        offset: core::mem::offset_of!(FilterConf, min_length),
        post: ptr::null_mut(),
    },
    ngx_command_t {
        name: ngx_string!("brotli_reuse_memory"),
        type_: (ANY_CONF | NGX_CONF_FLAG) as ngx_uint_t,
        set: Some(ngx_conf_set_flag_slot),
        conf: NGX_HTTP_LOC_CONF_OFFSET,
        offset: core::mem::offset_of!(FilterConf, reuse_memory),
        post: ptr::null_mut(),
    },
    ngx_command_t {
        name: ngx_string!("brotli_retention_policy"),
        type_: (ANY_CONF | NGX_CONF_TAKE12) as ngx_uint_t,
        set: Some(set_retention_policy),
        conf: NGX_HTTP_LOC_CONF_OFFSET,
        offset: 0,
        post: ptr::null_mut(),
    },
    ngx_command_t::empty(),
];

static MODULE_CTX: ngx_http_module_t = ngx_http_module_t {
    preconfiguration: Some(add_variables),
    postconfiguration: Some(filter_init),
    create_main_conf: None,
    init_main_conf: None,
    create_srv_conf: None,
    merge_srv_conf: None,
    create_loc_conf: Some(create_conf),
    merge_loc_conf: Some(merge_conf),
};

#[used]
#[allow(non_upper_case_globals)]
#[cfg_attr(not(feature = "export-modules"), unsafe(no_mangle))]
pub static mut ngx_http_brotli_filter_module: ngx_module_t = ngx_module_t {
    ctx: &raw const MODULE_CTX as _,
    commands: unsafe { &raw mut COMMANDS[0] },
    type_: NGX_HTTP_MODULE as _,
    ..ngx_module_t::default()
};

static mut NEXT_HEADER_FILTER: ngx_http_output_header_filter_pt = None;
static mut NEXT_BODY_FILTER: ngx_http_output_body_filter_pt = None;

fn module() -> &'static ngx_module_t {
    unsafe { &*ptr::addr_of!(ngx_http_brotli_filter_module) }
}

/// Tags the output buffers this module owns, for `ngx_chain_update_chains`.
fn buf_tag() -> *mut c_void {
    (&raw mut ngx_http_brotli_filter_module).cast()
}

unsafe fn next_header_filter(r: *mut ngx_http_request_t) -> ngx_int_t {
    let next = unsafe { NEXT_HEADER_FILTER }.expect("brotli filter is initialized");
    unsafe { next(r) }
}

unsafe fn next_body_filter(r: *mut ngx_http_request_t, chain: *mut ngx_chain_t) -> ngx_int_t {
    let next = unsafe { NEXT_BODY_FILTER }.expect("brotli filter is initialized");
    unsafe { next(r, chain) }
}

unsafe fn loc_conf<'a>(r: *mut ngx_http_request_t) -> &'a FilterConf {
    unsafe { &*(*(*r).loc_conf.add(module().ctx_index)).cast::<FilterConf>() }
}

type FilterCtx = BodyFilter<NginxHost>;

unsafe fn request_ctx(r: *mut ngx_http_request_t) -> *mut FilterCtx {
    unsafe { (*(*r).ctx.add(module().ctx_index)).cast() }
}

/// Header filter: decides whether the response is compressed.
unsafe extern "C" fn header_filter(r: *mut ngx_http_request_t) -> ngx_int_t {
    let req = unsafe { &mut *r };
    let conf = unsafe { loc_conf(r) };

    if conf.enable == 0 {
        return unsafe { next_header_filter(r) };
    }

    let status = req.headers_out.status;
    if status != NGX_HTTP_OK as ngx_uint_t
        && status != NGX_HTTP_FORBIDDEN as ngx_uint_t
        && status != NGX_HTTP_NOT_FOUND as ngx_uint_t
    {
        return unsafe { next_header_filter(r) };
    }

    if unsafe { ngx_mbrotli_r_header_only(r) } != 0 {
        return unsafe { next_header_filter(r) };
    }

    if let Some(encoding) = unsafe { req.headers_out.content_encoding.as_ref() }
        && encoding.value.len != 0
    {
        return unsafe { next_header_filter(r) };
    }

    let content_length = req.headers_out.content_length_n;
    if content_length != -1 && content_length < conf.min_length as off_t {
        return unsafe { next_header_filter(r) };
    }

    if unsafe { ngx_http_test_content_type(r, &conf.types as *const _ as *mut _) }.is_null() {
        return unsafe { next_header_filter(r) };
    }

    unsafe { ngx_mbrotli_r_set_gzip_vary(r) };

    if !unsafe { accept_encoding::check_request(r) } {
        return unsafe { next_header_filter(r) };
    }

    let settings = match settings(conf, content_length) {
        Ok(settings) => settings,
        Err(err) => {
            let log = unsafe { (*req.connection).log };
            ngx_log_error!(NGX_LOG_ALERT, log, "brotli encoder config failed: {err}");
            return unsafe { next_header_filter(r) };
        }
    };

    let ctx = BodyFilter::new(NginxHost { request: r }, settings);
    let ctx = unsafe { Pool::from_ngx_pool(req.pool) }.allocate(ctx);
    if ctx.is_null() {
        return NGX_ERROR as ngx_int_t;
    }
    unsafe { *req.ctx.add(module().ctx_index) = ctx.cast() };

    let h: *mut ngx_table_elt_t = unsafe { ngx_list_push(&mut req.headers_out.headers) }.cast();
    let Some(h) = (unsafe { h.as_mut() }) else {
        return NGX_ERROR as ngx_int_t;
    };
    h.hash = 1;
    h.next = ptr::null_mut();
    h.key = ngx_string!("Content-Encoding");
    h.value = ngx_string!("br");
    h.lowcase_key = c"content-encoding".as_ptr().cast_mut().cast();
    req.headers_out.content_encoding = h;

    unsafe { ngx_mbrotli_r_prepare_encoded(r) };

    unsafe { next_header_filter(r) }
}

/// Body filter: feeds the response through the encoder.
unsafe extern "C" fn body_filter(r: *mut ngx_http_request_t, input: *mut ngx_chain_t) -> ngx_int_t {
    let ctx = unsafe { request_ctx(r) };

    if ctx.is_null() || unsafe { (*ctx).is_done() } || unsafe { ngx_mbrotli_r_header_only(r) } != 0
    {
        return unsafe { next_body_filter(r, input) };
    }

    ngx_log_debug_mask!(
        DebugMask::Http,
        unsafe { (*(*r).connection).log },
        "http brotli filter"
    );

    unsafe { (*ctx).filter(input) }
}

/// The NGINX side of [`Host`], for one request.
struct NginxHost {
    request: *mut ngx_http_request_t,
}

impl NginxHost {
    fn pool(&self) -> *mut ngx_pool_t {
        unsafe { (*self.request).pool }
    }

    fn log(&self) -> *mut ngx_log_t {
        unsafe { (*(*self.request).connection).log }
    }
}

impl Host for NginxHost {
    unsafe fn chain_add_copy(
        &mut self,
        chain: &mut *mut ngx_chain_t,
        input: *mut ngx_chain_t,
    ) -> bool {
        unsafe { ngx_chain_add_copy(self.pool(), chain, input) == NGX_OK as ngx_int_t }
    }

    unsafe fn update_chains(
        &mut self,
        free: &mut *mut ngx_chain_t,
        busy: &mut *mut ngx_chain_t,
        out: &mut *mut ngx_chain_t,
    ) {
        unsafe { ngx_chain_update_chains(self.pool(), free, busy, out, buf_tag()) };
    }

    fn alloc_chain_link(&mut self) -> *mut ngx_chain_t {
        unsafe { ngx_alloc_chain_link(self.pool()) }
    }

    unsafe fn free_chain(&mut self, cl: *mut ngx_chain_t) {
        // ngx_free_chain()
        let pool = self.pool();
        unsafe {
            (*cl).next = (*pool).chain;
            (*pool).chain = cl;
        }
    }

    fn create_temp_buf(&mut self, size: usize) -> *mut ngx_buf_t {
        let b = unsafe { ngx_create_temp_buf(self.pool(), size) };
        if !b.is_null() {
            unsafe {
                (*b).tag = buf_tag();
                ngx_mbrotli_buf_set_recycled(b);
            }
        }
        b
    }

    fn calloc_buf(&mut self) -> *mut ngx_buf_t {
        unsafe { ngx_pcalloc(self.pool(), size_of::<ngx_buf_t>()) }.cast()
    }

    unsafe fn next_body_filter(&mut self, chain: *mut ngx_chain_t) -> ngx_int_t {
        unsafe { next_body_filter(self.request, chain) }
    }

    fn set_buffered(&mut self, on: bool) {
        unsafe {
            ngx_mbrotli_r_set_buffered(
                self.request,
                BROTLI_BUFFERED as ngx_uint_t,
                on as ngx_uint_t,
            )
        };
    }

    unsafe fn buf_flags(&self, b: *const ngx_buf_t) -> BufFlags {
        unsafe { buf_flags(b.cast_mut()) }
    }

    unsafe fn set_flush(&self, b: *mut ngx_buf_t) {
        unsafe { ngx_mbrotli_buf_set_flush(b) };
    }

    unsafe fn set_last_buf(&self, b: *mut ngx_buf_t) {
        unsafe { ngx_mbrotli_buf_set_last_buf(b) };
    }

    fn error(&self, args: fmt::Arguments<'_>) {
        ngx_log_error!(NGX_LOG_ALERT, self.log(), "{args}");
    }

    fn debug(&self, args: fmt::Arguments<'_>) {
        ngx_log_debug_mask!(DebugMask::Http, self.log(), "{args}");
    }
}

/// Resolves the location configuration for a response of `content_length`
/// bytes (`-1` if unknown).
fn settings(conf: &FilterConf, content_length: off_t) -> Result<Settings, ConfigError> {
    let quality = Quality::try_from(conf.quality as u8)?;
    let window = Window::standard(body::window_bits(content_length, conf.lg_win as u8))?;

    Ok(Settings {
        encoder: EncoderConfig::default()
            .with_quality(quality)
            .with_window(window),
        input_size: if content_length > 0 {
            InputSize::Exact(content_length as u64)
        } else {
            InputSize::Unknown
        },
        reuse_memory: conf.reuse_memory != 0,
        retention: conf.retention.unwrap_or(encoder_pool::DEFAULT_RETENTION),
        bufs_num: conf.bufs.num as usize,
        bufs_size: conf.bufs.size,
    })
}

unsafe extern "C" fn create_conf(cf: *mut ngx_conf_t) -> *mut c_void {
    let conf: *mut FilterConf = unsafe { ngx_palloc((*cf).pool, size_of::<FilterConf>()) }.cast();
    if !conf.is_null() {
        unsafe {
            conf.write(FilterConf {
                enable: UNSET,
                types: core::mem::zeroed(),
                types_keys: ptr::null_mut(),
                min_length: UNSET,
                bufs: ngx_bufs_t { num: 0, size: 0 },
                quality: UNSET,
                lg_win: UNSET_SIZE,
                reuse_memory: UNSET,
                retention: None,
            })
        };
    }
    conf.cast()
}

unsafe extern "C" fn merge_conf(
    cf: *mut ngx_conf_t,
    parent: *mut c_void,
    child: *mut c_void,
) -> *mut c_char {
    let prev = unsafe { &mut *parent.cast::<FilterConf>() };
    let conf = unsafe { &mut *child.cast::<FilterConf>() };

    fn merge<T: Copy + PartialEq>(value: &mut T, prev: T, unset: T, default: T) {
        if *value == unset {
            *value = if prev == unset { default } else { prev };
        }
    }

    merge(&mut conf.enable, prev.enable, UNSET, 0);
    merge(&mut conf.quality, prev.quality, UNSET, 6);
    merge(&mut conf.lg_win, prev.lg_win, UNSET_SIZE, 19);
    merge(&mut conf.min_length, prev.min_length, UNSET, 20);
    merge(&mut conf.reuse_memory, prev.reuse_memory, UNSET, 1);
    conf.retention = conf
        .retention
        .or(prev.retention)
        .or(Some(encoder_pool::DEFAULT_RETENTION));

    if conf.bufs.num == 0 {
        conf.bufs = if prev.bufs.num != 0 {
            prev.bufs
        } else {
            let pagesize = unsafe { ngx_pagesize };
            ngx_bufs_t {
                num: (128 * 1024 / pagesize) as ngx_int_t,
                size: pagesize,
            }
        };
    }

    let rv = unsafe {
        ngx_http_merge_types(
            cf,
            &mut conf.types_keys,
            &mut conf.types,
            &mut prev.types_keys,
            &mut prev.types,
            (&raw mut ngx_http_html_default_types).cast(),
        )
    };
    if !rv.is_null() {
        return NGX_CONF_ERROR;
    }

    NGX_CONF_OK
}

/// `brotli_retention_policy aggressive | current_config | release_all | bounded <size>`.
unsafe extern "C" fn set_retention_policy(
    cf: *mut ngx_conf_t,
    _cmd: *mut ngx_command_t,
    conf: *mut c_void,
) -> *mut c_char {
    let conf = unsafe { &mut *conf.cast::<FilterConf>() };
    if conf.retention.is_some() {
        return c"is duplicate".as_ptr().cast_mut();
    }

    let args: &mut [ngx_str_t] = unsafe { (*(*cf).args).as_slice_mut() };
    let (name, size) = match args {
        [_, name] => (*name, None),
        [_, name, size] => (*name, Some(size)),
        _ => return NGX_CONF_ERROR,
    };

    let policy = match (name.as_bytes(), size) {
        (b"aggressive", None) => RetentionPolicy::Aggressive,
        (b"current_config", None) => RetentionPolicy::CurrentConfig,
        (b"release_all", None) => RetentionPolicy::ReleaseAll,
        (b"bounded", Some(size)) => {
            let max_bytes = unsafe { ngx_parse_size(size) };
            if max_bytes == NGX_ERROR as isize {
                ngx_conf_log_error!(
                    NGX_LOG_EMERG,
                    cf,
                    "invalid size \"{size}\" in \"brotli_retention_policy\" directive"
                );
                return NGX_CONF_ERROR;
            }
            RetentionPolicy::Bounded {
                max_bytes: max_bytes as usize,
            }
        }
        (b"bounded", None) => {
            ngx_conf_log_error!(
                NGX_LOG_EMERG,
                cf,
                "\"bounded\" in \"brotli_retention_policy\" directive requires a size"
            );
            return NGX_CONF_ERROR;
        }
        (b"aggressive" | b"current_config" | b"release_all", Some(_)) => {
            ngx_conf_log_error!(
                NGX_LOG_EMERG,
                cf,
                "\"{name}\" in \"brotli_retention_policy\" directive does not take a size"
            );
            return NGX_CONF_ERROR;
        }
        _ => {
            ngx_conf_log_error!(
                NGX_LOG_EMERG,
                cf,
                "invalid value \"{name}\" in \"brotli_retention_policy\" directive, \
                 it must be \"aggressive\", \"current_config\", \"release_all\" \
                 or \"bounded <size>\""
            );
            return NGX_CONF_ERROR;
        }
    };

    conf.retention = Some(policy);
    NGX_CONF_OK
}

/// Translates `brotli_window` from a size into window bits.
unsafe extern "C" fn parse_window_bits(
    _cf: *mut ngx_conf_t,
    _post: *mut c_void,
    data: *mut c_void,
) -> *mut c_char {
    let value = unsafe { &mut *data.cast::<usize>() };

    match (MIN_WINDOW_BITS..=MAX_WINDOW_BITS).find(|bits| *value == 1 << bits) {
        Some(bits) => {
            *value = bits;
            NGX_CONF_OK
        }
        None => c"must be 1k, 2k, 4k, 8k, 16k, 32k, 64k, 128k, 256k, 512k, 1m, 2m, 4m, 8m or 16m"
            .as_ptr()
            .cast_mut(),
    }
}

unsafe extern "C" fn add_variables(cf: *mut ngx_conf_t) -> ngx_int_t {
    let mut name = ngx_string!("brotli_ratio");
    let var = unsafe { ngx_http_add_variable(cf, &mut name, 0) };
    let Some(var) = (unsafe { var.as_mut() }) else {
        return NGX_ERROR as ngx_int_t;
    };
    var.get_handler = Some(ratio_variable);
    NGX_OK as ngx_int_t
}

/// `$brotli_ratio`: the achieved compression ratio, e.g. `3.25`.
unsafe extern "C" fn ratio_variable(
    r: *mut ngx_http_request_t,
    v: *mut ngx_http_variable_value_t,
    _data: usize,
) -> ngx_int_t {
    let sizes = unsafe { request_ctx(r).as_ref() }.and_then(FilterCtx::sizes);
    let Some((bytes_in, bytes_out)) = sizes.filter(|&(_, out)| out != 0) else {
        unsafe { ngx_mbrotli_var_set_not_found(v) };
        return NGX_OK as ngx_int_t;
    };

    let ratio = ratio::format(bytes_in, bytes_out);
    let data: *mut u8 = unsafe { ngx_pnalloc((*r).pool, ratio.len()) }.cast();
    if data.is_null() {
        return NGX_ERROR as ngx_int_t;
    }
    unsafe {
        ptr::copy_nonoverlapping(ratio.as_ptr(), data, ratio.len());
        ngx_mbrotli_var_set(v, data, ratio.len());
    }

    NGX_OK as ngx_int_t
}

unsafe extern "C" fn filter_init(_cf: *mut ngx_conf_t) -> ngx_int_t {
    unsafe {
        NEXT_HEADER_FILTER = ngx_http_top_header_filter;
        ngx_http_top_header_filter = Some(header_filter);

        NEXT_BODY_FILTER = ngx_http_top_body_filter;
        ngx_http_top_body_filter = Some(body_filter);
    }
    NGX_OK as ngx_int_t
}
