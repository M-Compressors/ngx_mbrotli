//! `ngx_http_brotli_filter_module`: on-the-fly Brotli compression of responses.
//!
//! Directives and behaviour follow google/ngx_brotli; output buffering follows
//! the stock gzip filter, so `brotli_buffers` is honoured rather than ignored.

use core::ffi::{c_char, c_void};
use core::{ptr, slice};

use mbrotli::{
    EncodeError, EncoderConfig, EncoderSessionOwned, EncoderStatus, InputSize, Operation, Quality,
    StreamConfig, Window,
};
use ngx::core::{NGX_CONF_ERROR, NGX_CONF_OK, Pool};
use ngx::ffi::{
    NGX_AGAIN, NGX_CONF_1MORE, NGX_CONF_FLAG, NGX_CONF_TAKE1, NGX_CONF_TAKE2, NGX_ERROR,
    NGX_HTTP_FORBIDDEN, NGX_HTTP_GZIP_BUFFERED, NGX_HTTP_LIF_CONF, NGX_HTTP_LOC_CONF,
    NGX_HTTP_LOC_CONF_OFFSET, NGX_HTTP_MAIN_CONF, NGX_HTTP_MODULE, NGX_HTTP_NOT_FOUND, NGX_HTTP_OK,
    NGX_HTTP_SRV_CONF, NGX_LOG_ALERT, NGX_OK, ngx_alloc_chain_link, ngx_array_t, ngx_buf_t,
    ngx_bufs_t, ngx_chain_add_copy, ngx_chain_t, ngx_chain_update_chains, ngx_command_t,
    ngx_conf_check_num_bounds, ngx_conf_num_bounds_t, ngx_conf_post_t, ngx_conf_set_bufs_slot,
    ngx_conf_set_flag_slot, ngx_conf_set_num_slot, ngx_conf_set_size_slot, ngx_conf_t,
    ngx_create_temp_buf, ngx_flag_t, ngx_hash_t, ngx_http_add_variable,
    ngx_http_html_default_types, ngx_http_merge_types, ngx_http_module_t,
    ngx_http_output_body_filter_pt, ngx_http_output_header_filter_pt, ngx_http_request_t,
    ngx_http_test_content_type, ngx_http_top_body_filter, ngx_http_top_header_filter,
    ngx_http_types_slot, ngx_http_variable_value_t, ngx_int_t, ngx_list_push, ngx_module_t,
    ngx_pagesize, ngx_pcalloc, ngx_pnalloc, ngx_pool_t, ngx_table_elt_t, ngx_uint_t, off_t,
};
use ngx::{ngx_log_debug_mask, ngx_log_error, ngx_string};

use crate::accept_encoding;
use crate::bitfields::{
    BufFlags, ngx_mbrotli_buf_set_flush, ngx_mbrotli_buf_set_last_buf,
    ngx_mbrotli_buf_set_recycled, ngx_mbrotli_r_header_only, ngx_mbrotli_r_prepare_encoded,
    ngx_mbrotli_r_set_buffered, ngx_mbrotli_r_set_gzip_vary, ngx_mbrotli_var_set,
    ngx_mbrotli_var_set_not_found,
};
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

static mut COMMANDS: [ngx_command_t; 7] = [
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

    let ctx = unsafe { Pool::from_ngx_pool(req.pool) }.allocate(FilterCtx::new(r, content_length));
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

    if ctx.is_null() || unsafe { (*ctx).done } || unsafe { ngx_mbrotli_r_header_only(r) } != 0 {
        return unsafe { next_body_filter(r, input) };
    }

    ngx_log_debug_mask!(
        DebugMask::Http,
        unsafe { (*(*r).connection).log },
        "http brotli filter"
    );

    let ctx = unsafe { &mut *ctx };
    match unsafe { ctx.filter(input) } {
        Ok(rc) => rc,
        Err(Failed) => {
            ctx.fail();
            NGX_ERROR as ngx_int_t
        }
    }
}

/// An error that has already been logged; the response must be aborted.
struct Failed;

/// What the filter loop should do after a step.
#[derive(Copy, Clone, Eq, PartialEq)]
enum Step {
    /// The step completed; carry on with the next one.
    Ok,
    /// Repeat the inner loop.
    Again,
    /// Nothing more can be done in the inner loop.
    Declined,
}

/// Per-request state, allocated from the request pool and dropped by its
/// cleanup handler.
struct FilterCtx {
    request: *mut ngx_http_request_t,
    session: Option<EncoderSessionOwned>,
    content_length: off_t,

    /// Input not yet handed to the encoder.
    input: *mut ngx_chain_t,
    /// The input buffer being consumed; null once it has been drained.
    ///
    /// A drained buffer belongs to the filter that produced it again and may be
    /// refilled with later data, so it must never be read after that point.
    in_buf: *mut ngx_buf_t,

    free: *mut ngx_chain_t,
    busy: *mut ngx_chain_t,
    out: *mut ngx_chain_t,
    out_tail: *mut ngx_chain_t,
    /// The output buffer being filled; null once it has been queued.
    out_buf: *mut ngx_buf_t,
    bufs: ngx_int_t,

    operation: Operation,
    /// The encoder holds output that did not fit the last buffer.
    redo: bool,
    nomem: bool,
    done: bool,
    success: bool,

    bytes_in: u64,
    bytes_out: u64,
}

impl Drop for FilterCtx {
    fn drop(&mut self) {
        self.release_encoder();
    }
}

impl FilterCtx {
    fn new(request: *mut ngx_http_request_t, content_length: off_t) -> Self {
        Self {
            request,
            session: None,
            content_length,
            input: ptr::null_mut(),
            in_buf: ptr::null_mut(),
            free: ptr::null_mut(),
            busy: ptr::null_mut(),
            out: ptr::null_mut(),
            out_tail: ptr::null_mut(),
            out_buf: ptr::null_mut(),
            bufs: 0,
            operation: Operation::Process,
            redo: false,
            nomem: false,
            done: false,
            success: false,
            bytes_in: 0,
            bytes_out: 0,
        }
    }

    fn pool(&self) -> *mut ngx_pool_t {
        unsafe { (*self.request).pool }
    }

    fn log(&self) -> *mut ngx::ffi::ngx_log_t {
        unsafe { (*(*self.request).connection).log }
    }

    fn set_buffered(&self, on: bool) {
        unsafe {
            ngx_mbrotli_r_set_buffered(
                self.request,
                BROTLI_BUFFERED as ngx_uint_t,
                on as ngx_uint_t,
            )
        };
    }

    fn release_encoder(&mut self) {
        if let Some(session) = self.session.take() {
            encoder_pool::release(session.into_compressor());
        }
    }

    fn fail(&mut self) {
        self.done = true;
        self.release_encoder();
    }

    unsafe fn filter(&mut self, input: *mut ngx_chain_t) -> Result<ngx_int_t, Failed> {
        let r = self.request;

        if self.session.is_none() {
            self.start()?;
        }

        if !input.is_null() {
            if unsafe { ngx_chain_add_copy(self.pool(), &mut self.input, input) } != NGX_OK as _ {
                return Err(Failed);
            }
            self.set_buffered(true);
        }

        let mut flush = if self.nomem {
            // Push busy buffers downstream to get some of them back.
            if unsafe { next_body_filter(r, ptr::null_mut()) } == NGX_ERROR as ngx_int_t {
                return Err(Failed);
            }
            let mut cl = ptr::null_mut();
            unsafe {
                ngx_chain_update_chains(
                    self.pool(),
                    &mut self.free,
                    &mut self.busy,
                    &mut cl,
                    buf_tag(),
                )
            };
            self.nomem = false;
            false
        } else {
            !self.busy.is_null()
        };

        loop {
            // Cycle while there is input for the encoder and room for output.
            loop {
                match self.add_data()? {
                    Step::Declined => break,
                    Step::Again => continue,
                    Step::Ok => {}
                }

                if !self.get_buf()? {
                    break;
                }

                if self.compress()? == Step::Ok {
                    break;
                }
            }

            if self.out.is_null() && !flush {
                return Ok(if self.busy.is_null() {
                    NGX_OK as ngx_int_t
                } else {
                    NGX_AGAIN as ngx_int_t
                });
            }

            let rc = unsafe { next_body_filter(r, self.out) };
            if rc == NGX_ERROR as ngx_int_t {
                return Err(Failed);
            }

            unsafe {
                ngx_chain_update_chains(
                    self.pool(),
                    &mut self.free,
                    &mut self.busy,
                    &mut self.out,
                    buf_tag(),
                )
            };
            self.out_tail = ptr::null_mut();
            self.nomem = false;
            flush = false;

            if self.done {
                return Ok(rc);
            }
        }
    }

    fn start(&mut self) -> Result<(), Failed> {
        let conf = unsafe { loc_conf(self.request) };

        // Shrink the window for responses known to be smaller than it.
        let window_bits = if self.content_length > 0 {
            let mut bits = MIN_WINDOW_BITS;
            while bits < conf.lg_win && self.content_length > (1 << bits) {
                bits += 1;
            }
            bits
        } else {
            conf.lg_win
        };

        let compressor = Quality::try_from(conf.quality as u8)
            .and_then(|quality| Ok((quality, Window::standard(window_bits as u8)?)))
            .map(|(quality, window)| {
                EncoderConfig::default()
                    .with_quality(quality)
                    .with_window(window)
            })
            .and_then(encoder_pool::acquire)
            .map_err(|err| {
                ngx_log_error!(
                    NGX_LOG_ALERT,
                    self.log(),
                    "brotli encoder config failed: {err}"
                );
                Failed
            })?;

        let input_size = if self.content_length > 0 {
            InputSize::Exact(self.content_length as u64)
        } else {
            InputSize::Unknown
        };

        let session = compressor
            .into_session(StreamConfig::from(input_size))
            .map_err(|err| self.encode_error(&err))?;
        self.session = Some(session);

        ngx_log_debug_mask!(
            DebugMask::Http,
            self.log(),
            "brotli encoder initialized: lvl:{} win:{}",
            conf.quality,
            1usize << window_bits
        );

        Ok(())
    }

    fn encode_error(&self, err: &EncodeError) -> Failed {
        ngx_log_error!(NGX_LOG_ALERT, self.log(), "brotli encoder failed: {err}");
        Failed
    }

    /// Bytes of the current input buffer still waiting for the encoder.
    fn pending_input<'a>(&self) -> &'a [u8] {
        match unsafe { self.in_buf.as_ref() } {
            Some(b) if b.last > b.pos && unsafe { BufFlags::of(self.in_buf) }.in_memory() => unsafe {
                slice::from_raw_parts(b.pos, b.last.offset_from(b.pos) as usize)
            },
            _ => &[],
        }
    }

    /// Takes the next input buffer, if the encoder needs one.
    fn add_data(&mut self) -> Result<Step, Failed> {
        if !self.pending_input().is_empty() || self.operation != Operation::Process || self.redo {
            return Ok(Step::Ok);
        }

        let cl = self.input;
        if cl.is_null() {
            return Ok(Step::Declined);
        }

        unsafe {
            self.in_buf = (*cl).buf;
            self.input = (*cl).next;
            free_chain(self.pool(), cl);
        }

        let b = unsafe { &*self.in_buf };
        let flags = unsafe { BufFlags::of(self.in_buf) };

        if !flags.in_memory() && flags.in_file() && b.file_last > b.file_pos {
            ngx_log_error!(NGX_LOG_ALERT, self.log(), "brotli filter got a file buffer");
            return Err(Failed);
        }

        let empty = self.pending_input().is_empty();
        if empty {
            self.in_buf = ptr::null_mut();
        }

        if flags.last_buf() {
            self.operation = Operation::Finish;
        } else if flags.flush() {
            self.operation = Operation::Flush;
        } else if empty {
            return Ok(Step::Again);
        }

        Ok(Step::Ok)
    }

    /// Makes sure there is an output buffer with free space. Returns `false`
    /// when all `brotli_buffers` are busy downstream.
    fn get_buf(&mut self) -> Result<bool, Failed> {
        if let Some(b) = unsafe { self.out_buf.as_ref() }
            && b.last < b.end
        {
            return Ok(true);
        }

        if !self.free.is_null() {
            let cl = self.free;
            unsafe {
                self.out_buf = (*cl).buf;
                self.free = (*cl).next;
                free_chain(self.pool(), cl);
            }
            return Ok(true);
        }

        let conf = unsafe { loc_conf(self.request) };
        if self.bufs >= conf.bufs.num {
            self.nomem = true;
            return Ok(false);
        }

        let b = unsafe { ngx_create_temp_buf(self.pool(), conf.bufs.size) };
        let Some(b) = (unsafe { b.as_mut() }) else {
            return Err(Failed);
        };
        b.tag = buf_tag();
        unsafe { ngx_mbrotli_buf_set_recycled(b) };
        self.out_buf = b;
        self.bufs += 1;

        Ok(true)
    }

    /// Runs the encoder once over the current input and output buffers.
    fn compress(&mut self) -> Result<Step, Failed> {
        let input = self.pending_input();
        let input_len = input.len();
        let out_buf = unsafe { &mut *self.out_buf };
        let room = unsafe { out_buf.end.offset_from(out_buf.last) } as usize;
        let output = unsafe { slice::from_raw_parts_mut(out_buf.last, room) };

        let session = self.session.as_mut().ok_or(Failed)?;
        let progress = match session.process(input, output, self.operation) {
            Ok(progress) => progress,
            Err(err) => return Err(self.encode_error(&err)),
        };

        if progress.consumed > 0 {
            unsafe { (*self.in_buf).pos = (*self.in_buf).pos.add(progress.consumed) };
            if progress.consumed == input_len {
                self.in_buf = ptr::null_mut();
            }
        }
        out_buf.last = unsafe { out_buf.last.add(progress.produced) };
        self.bytes_in += progress.consumed as u64;
        self.bytes_out += progress.produced as u64;

        ngx_log_debug_mask!(
            DebugMask::Http,
            self.log(),
            "brotli in:{} consumed:{} out:{} produced:{} op:{:?} status:{:?}",
            input_len,
            progress.consumed,
            room,
            progress.produced,
            self.operation,
            progress.status
        );

        if out_buf.last == out_buf.end && progress.status != EncoderStatus::Finished {
            self.queue_out_buf()?;
        }

        match progress.status {
            EncoderStatus::NeedsOutput => {
                if progress.produced == 0 {
                    ngx_log_error!(NGX_LOG_ALERT, self.log(), "brotli encoder made no progress");
                    return Err(Failed);
                }
                self.redo = true;
                Ok(Step::Again)
            }

            EncoderStatus::Finished => {
                self.redo = false;
                let b = self.take_out_buf();
                if b.is_null() {
                    return Err(Failed);
                }
                unsafe { ngx_mbrotli_buf_set_last_buf(b) };
                self.queue(b)?;

                self.done = true;
                self.success = true;
                self.release_encoder();
                self.set_buffered(false);
                Ok(Step::Ok)
            }

            EncoderStatus::NeedsInput => {
                self.redo = false;
                match self.operation {
                    Operation::Process => {
                        if progress.consumed == 0 && input_len != 0 {
                            ngx_log_error!(
                                NGX_LOG_ALERT,
                                self.log(),
                                "brotli encoder made no progress"
                            );
                            return Err(Failed);
                        }
                        Ok(Step::Again)
                    }

                    Operation::Flush => {
                        self.operation = Operation::Process;

                        let b = match unsafe { self.out_buf.as_ref() } {
                            Some(b) if b.last != b.pos => self.take_out_buf(),
                            // Nothing to send: a special buffer carries the flush.
                            _ => unsafe { ngx_pcalloc(self.pool(), size_of::<ngx_buf_t>()) }.cast(),
                        };
                        if b.is_null() {
                            return Err(Failed);
                        }
                        unsafe { ngx_mbrotli_buf_set_flush(b) };
                        self.queue(b)?;

                        self.set_buffered(false);
                        Ok(Step::Ok)
                    }

                    Operation::Finish => {
                        ngx_log_error!(
                            NGX_LOG_ALERT,
                            self.log(),
                            "brotli encoder did not finish the stream"
                        );
                        Err(Failed)
                    }
                }
            }
        }
    }

    /// Detaches the current output buffer, allocating an empty one if needed.
    fn take_out_buf(&mut self) -> *mut ngx_buf_t {
        let b = core::mem::replace(&mut self.out_buf, ptr::null_mut());
        if b.is_null() {
            unsafe { ngx_pcalloc(self.pool(), size_of::<ngx_buf_t>()) }.cast()
        } else {
            b
        }
    }

    fn queue_out_buf(&mut self) -> Result<(), Failed> {
        let b = self.take_out_buf();
        self.queue(b)
    }

    fn queue(&mut self, b: *mut ngx_buf_t) -> Result<(), Failed> {
        let cl = unsafe { ngx_alloc_chain_link(self.pool()) };
        if b.is_null() || cl.is_null() {
            return Err(Failed);
        }

        unsafe {
            (*cl).buf = b;
            (*cl).next = ptr::null_mut();
            match self.out_tail.as_mut() {
                Some(tail) => tail.next = cl,
                None => self.out = cl,
            }
        }
        self.out_tail = cl;

        Ok(())
    }
}

/// `ngx_free_chain()`: returns a chain link to the pool's free list.
unsafe fn free_chain(pool: *mut ngx_pool_t, cl: *mut ngx_chain_t) {
    unsafe {
        (*cl).next = (*pool).chain;
        (*pool).chain = cl;
    }
}

unsafe extern "C" fn create_conf(cf: *mut ngx_conf_t) -> *mut c_void {
    let conf: *mut FilterConf = unsafe { ngx_pcalloc((*cf).pool, size_of::<FilterConf>()) }.cast();
    if let Some(conf) = unsafe { conf.as_mut() } {
        // Zeroed: types, types_keys and bufs.
        conf.enable = UNSET;
        conf.quality = UNSET;
        conf.lg_win = UNSET_SIZE;
        conf.min_length = UNSET;
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
    let ctx = unsafe { request_ctx(r).as_ref() };
    let Some(ctx) = ctx.filter(|ctx| ctx.success && ctx.bytes_out != 0) else {
        unsafe { ngx_mbrotli_var_set_not_found(v) };
        return NGX_OK as ngx_int_t;
    };

    let ratio = ratio::format(ctx.bytes_in, ctx.bytes_out);
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
