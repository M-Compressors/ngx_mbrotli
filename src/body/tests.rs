//! Tests of the body filter against a model of the NGINX buffer API.
//!
//! The model follows the C implementation of every [`Host`] call, but is
//! stricter than NGINX where that turns a logic error into one Miri reports:
//!
//! - freed chain links are deallocated at once instead of being kept for reuse,
//!   so a link used after `ngx_free_chain()` is a use-after-free;
//! - output buffers are uninitialized, as `ngx_create_temp_buf()`'s are, so
//!   sending bytes the encoder never wrote is a read of uninitialized memory;
//! - upstream frees each input buffer and every chain link it passes as soon
//!   as the call returns and the buffer is drained, so holding on to either
//!   is a use-after-free.

use core::cell::RefCell;
use core::ffi::c_void;
use core::fmt;
use core::ptr;
use std::alloc::{Layout, alloc, dealloc};
use std::collections::{HashSet, VecDeque};

use mbrotli::{
    DecodeOperation, DecodeStreamConfig, DecoderConfig, DecoderStatus, Decompressor, EncoderConfig,
    InputSize, Quality, RetentionPolicy, Window,
};
use ngx::ffi::{NGX_AGAIN, NGX_ERROR, NGX_OK, ngx_buf_t, ngx_chain_t, ngx_int_t, off_t};

use super::{BodyFilter, BufFlags, Host, Settings, window_bits};

/// The request pool, the client and the connection, as the filter sees them.
struct Model {
    /// Its address tags the brotli module's buffers.
    tag: Box<u8>,
    /// Chain links allocated and not yet freed.
    links: HashSet<*mut ngx_chain_t>,
    /// Buffers and memory owned by the pool until the request ends.
    bufs: Vec<*mut ngx_buf_t>,
    memory: Vec<(*mut u8, Layout)>,
    temp_bufs: usize,

    /// Bytes the client takes per call to the next filter; `None` for all.
    budget: Option<usize>,
    /// Buffers passed to the next filter and not yet sent completely.
    pending: VecDeque<*mut ngx_buf_t>,
    /// What the client received, and where flushes and the end came.
    sent: Vec<u8>,
    flushes: Vec<usize>,
    last: Option<usize>,

    buffered: bool,
    errors: RefCell<Vec<String>>,
}

impl Model {
    fn new(budget: Option<usize>) -> Self {
        Self {
            tag: Box::new(0),
            links: HashSet::new(),
            bufs: Vec::new(),
            memory: Vec::new(),
            temp_bufs: 0,
            budget,
            pending: VecDeque::new(),
            sent: Vec::new(),
            flushes: Vec::new(),
            last: None,
            buffered: false,
            errors: RefCell::new(Vec::new()),
        }
    }

    fn tag(&self) -> *mut c_void {
        ptr::from_ref::<u8>(&self.tag).cast_mut().cast()
    }

    fn alloc_buf(&mut self) -> *mut ngx_buf_t {
        let b = Box::into_raw(Box::new(unsafe { core::mem::zeroed::<ngx_buf_t>() }));
        self.bufs.push(b);
        b
    }

    /// The write filter: sends what the client takes of the pending buffers.
    fn send(&mut self) -> ngx_int_t {
        let mut budget = self.budget.unwrap_or(usize::MAX);

        while let Some(&b) = self.pending.front() {
            let b = unsafe { &mut *b };
            let flags = unsafe { self.buf_flags(b) };

            if flags.in_memory {
                let size = unsafe { b.last.offset_from(b.pos) } as usize;
                let n = size.min(budget);
                self.sent
                    .extend_from_slice(unsafe { core::slice::from_raw_parts(b.pos, n) });
                b.pos = unsafe { b.pos.add(n) };
                budget -= n;
                if n < size {
                    break;
                }
            }

            if flags.flush {
                self.flushes.push(self.sent.len());
            }
            if flags.last_buf {
                self.last = Some(self.sent.len());
            }
            self.pending.pop_front();
        }

        if self.pending.is_empty() {
            NGX_OK as ngx_int_t
        } else {
            NGX_AGAIN as ngx_int_t
        }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        // The request pool is destroyed.
        for cl in self.links.drain() {
            drop(unsafe { Box::from_raw(cl) });
        }
        for b in self.bufs.drain(..) {
            drop(unsafe { Box::from_raw(b) });
        }
        for (p, layout) in self.memory.drain(..) {
            unsafe { dealloc(p, layout) };
        }
    }
}

impl Host for Model {
    unsafe fn chain_add_copy(
        &mut self,
        chain: &mut *mut ngx_chain_t,
        mut input: *mut ngx_chain_t,
    ) -> bool {
        let mut ll: *mut *mut ngx_chain_t = chain;
        unsafe {
            while !(*ll).is_null() {
                ll = &raw mut (**ll).next;
            }
            while !input.is_null() {
                let cl = self.alloc_chain_link();
                (*cl).buf = (*input).buf;
                *ll = cl;
                ll = &raw mut (*cl).next;
                input = (*input).next;
            }
            *ll = ptr::null_mut();
        }
        true
    }

    unsafe fn update_chains(
        &mut self,
        free: &mut *mut ngx_chain_t,
        busy: &mut *mut ngx_chain_t,
        out: &mut *mut ngx_chain_t,
    ) {
        unsafe {
            if !out.is_null() {
                if busy.is_null() {
                    *busy = *out;
                } else {
                    let mut cl = *busy;
                    while !(*cl).next.is_null() {
                        cl = (*cl).next;
                    }
                    (*cl).next = *out;
                }
                *out = ptr::null_mut();
            }

            while !busy.is_null() {
                let cl = *busy;
                let b = (*cl).buf;

                if (*b).tag != self.tag() {
                    *busy = (*cl).next;
                    self.free_chain(cl);
                    continue;
                }

                let size = if self.buf_flags(b).in_memory {
                    (*b).last.offset_from((*b).pos) as off_t
                } else {
                    (*b).file_last - (*b).file_pos
                };
                if size != 0 {
                    break;
                }

                (*b).pos = (*b).start;
                (*b).last = (*b).start;

                *busy = (*cl).next;
                (*cl).next = *free;
                *free = cl;
            }
        }
    }

    fn alloc_chain_link(&mut self) -> *mut ngx_chain_t {
        let cl = Box::into_raw(Box::new(ngx_chain_t {
            buf: ptr::null_mut(),
            next: ptr::null_mut(),
        }));
        self.links.insert(cl);
        cl
    }

    unsafe fn free_chain(&mut self, cl: *mut ngx_chain_t) {
        assert!(self.links.remove(&cl), "freed a chain link it does not own");
        drop(unsafe { Box::from_raw(cl) });
    }

    fn create_temp_buf(&mut self, size: usize) -> *mut ngx_buf_t {
        let layout = Layout::array::<u8>(size).unwrap();
        let p = unsafe { alloc(layout) };
        assert!(!p.is_null());
        self.memory.push((p, layout));
        self.temp_bufs += 1;

        let tag = self.tag();
        let b = unsafe { &mut *self.alloc_buf() };
        b.start = p;
        b.pos = p;
        b.last = p;
        b.end = unsafe { p.add(size) };
        b.set_temporary(1);
        b.set_recycled(1);
        b.tag = tag;
        b
    }

    fn calloc_buf(&mut self) -> *mut ngx_buf_t {
        self.alloc_buf()
    }

    unsafe fn next_body_filter(&mut self, mut chain: *mut ngx_chain_t) -> ngx_int_t {
        while !chain.is_null() {
            unsafe {
                self.pending.push_back((*chain).buf);
                chain = (*chain).next;
            }
        }
        self.send()
    }

    fn set_buffered(&mut self, on: bool) {
        self.buffered = on;
    }

    unsafe fn buf_flags(&self, b: *const ngx_buf_t) -> BufFlags {
        let b = unsafe { &*b };
        BufFlags {
            in_memory: b.temporary() != 0 || b.memory() != 0 || b.mmap() != 0,
            in_file: b.in_file() != 0,
            last_buf: b.last_buf() != 0,
            flush: b.flush() != 0,
        }
    }

    unsafe fn set_flush(&self, b: *mut ngx_buf_t) {
        unsafe { (*b).set_flush(1) };
    }

    unsafe fn set_last_buf(&self, b: *mut ngx_buf_t) {
        let b = unsafe { &mut *b };
        if b.pos == b.last {
            b.set_temporary(0);
        }
        b.set_last_buf(1);
    }

    fn error(&self, args: fmt::Arguments<'_>) {
        self.errors.borrow_mut().push(args.to_string());
    }

    fn debug(&self, _args: fmt::Arguments<'_>) {}
}

/// One buffer of a response body, as an upstream filter passes it.
#[derive(Copy, Clone)]
enum Part<'a> {
    Data(&'a [u8]),
    Flush(&'a [u8]),
    Last(&'a [u8]),
    /// A buffer that only refers to a file.
    File,
}

/// The filters above the brotli one, which own the body buffers.
#[derive(Default)]
struct Upstream {
    /// Buffers passed on and not yet drained, with the memory they point to.
    held: Vec<(*mut ngx_buf_t, *mut [u8])>,
}

impl Upstream {
    /// Passes `parts` to the filter in one chain, then frees the chain links
    /// and every buffer the filter has drained.
    fn send(&mut self, filter: &mut BodyFilter<Model>, parts: &[Part<'_>]) -> ngx_int_t {
        let mut links = Vec::new();
        for part in parts {
            let b = Box::into_raw(Box::new(unsafe { core::mem::zeroed::<ngx_buf_t>() }));
            let data = Box::into_raw(Box::<[u8]>::from(match part {
                Part::Data(d) | Part::Flush(d) | Part::Last(d) => *d,
                Part::File => &[],
            }));

            let buf = unsafe { &mut *b };
            let start = data.cast::<u8>();
            buf.start = start;
            buf.pos = start;
            buf.last = unsafe { start.add(data.len()) };
            buf.end = buf.last;
            match part {
                Part::Data(_) => buf.set_temporary(1),
                Part::Flush(d) | Part::Last(d) => {
                    buf.set_temporary(!d.is_empty() as u32);
                    if matches!(part, Part::Flush(_)) {
                        buf.set_flush(1);
                    } else {
                        buf.set_last_buf(1);
                    }
                }
                Part::File => {
                    buf.set_in_file(1);
                    buf.file_last = 100;
                }
            }
            self.held.push((b, data));

            links.push(Box::into_raw(Box::new(ngx_chain_t {
                buf: b,
                next: ptr::null_mut(),
            })));
        }
        for pair in links.windows(2) {
            unsafe { (*pair[0]).next = pair[1] };
        }

        let head = links.first().copied().unwrap_or(ptr::null_mut());
        let rc = unsafe { filter.filter(head) };

        for cl in links {
            drop(unsafe { Box::from_raw(cl) });
        }
        self.release_drained();
        rc
    }

    /// A write event: the filter is called again without new input.
    fn resume(&mut self, filter: &mut BodyFilter<Model>) -> ngx_int_t {
        let rc = if filter.is_done() {
            // The brotli filter passes calls through once it is done.
            unsafe { filter.host_mut().next_body_filter(ptr::null_mut()) }
        } else {
            unsafe { filter.filter(ptr::null_mut()) }
        };
        self.release_drained();
        rc
    }

    fn release_drained(&mut self) {
        self.held.retain(|&(b, data)| {
            let drained = unsafe { (*b).pos == (*b).last && (*b).file_pos == (*b).file_last };
            if drained {
                drop(unsafe { Box::from_raw(b) });
                drop(unsafe { Box::from_raw(data) });
            }
            !drained
        });
    }
}

impl Drop for Upstream {
    fn drop(&mut self) {
        for (b, data) in self.held.drain(..) {
            drop(unsafe { Box::from_raw(b) });
            drop(unsafe { Box::from_raw(data) });
        }
    }
}

fn settings(bufs_num: usize, bufs_size: usize) -> Settings {
    Settings {
        encoder: EncoderConfig::default()
            .with_quality(Quality::Q5)
            .with_window(Window::standard(16).unwrap()),
        input_size: InputSize::Unknown,
        reuse_memory: true,
        retention: RetentionPolicy::Bounded { max_bytes: 1 << 20 },
        bufs_num,
        bufs_size,
    }
}

fn filter(budget: Option<usize>, bufs_num: usize, bufs_size: usize) -> BodyFilter<Model> {
    BodyFilter::new(Model::new(budget), settings(bufs_num, bufs_size))
}

fn text(len: usize) -> Vec<u8> {
    let words = [
        "brotli ", "nginx ", "filter ", "chain ", "buffer ", "stream ",
    ];
    let mut seed = 7u32;
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        out.extend_from_slice(words[(seed >> 16) as usize % words.len()].as_bytes());
    }
    out.truncate(len);
    out
}

fn decode(stream: &[u8]) -> Vec<u8> {
    Decompressor::new(DecoderConfig::default())
        .unwrap()
        .decompress(stream)
        .unwrap()
}

/// Decodes the start of an unfinished stream, up to its last flush.
fn decode_prefix(stream: &[u8]) -> Vec<u8> {
    let mut decoder = Decompressor::new(DecoderConfig::default()).unwrap();
    let mut session = decoder.start(DecodeStreamConfig::default()).unwrap();
    let mut input = stream;
    let mut decoded = Vec::new();
    let mut out = vec![0; 1 << 16];
    loop {
        let progress = session
            .process(input, &mut out, DecodeOperation::Process)
            .unwrap();
        input = &input[progress.consumed..];
        decoded.extend_from_slice(&out[..progress.produced]);
        if progress.status == DecoderStatus::NeedsInput {
            return decoded;
        }
    }
}

const OK: ngx_int_t = NGX_OK as ngx_int_t;

#[test]
fn one_call_round_trip() {
    let (a, b, c) = (text(1500), text(700), text(300));
    let mut f = filter(None, 4, 4096);
    let mut up = Upstream::default();

    assert_eq!(
        up.send(&mut f, &[Part::Data(&a), Part::Data(&b), Part::Last(&c)]),
        OK
    );

    let model = f.host();
    assert!(f.is_done());
    assert_eq!(decode(&model.sent), [a, b, c].concat());
    assert_eq!(model.last, Some(model.sent.len()));
    assert!(!model.buffered);
    assert_eq!(f.sizes(), Some((2500, model.sent.len() as u64)));
    assert!(model.errors.borrow().is_empty());
}

#[test]
fn drained_input_is_never_touched_again() {
    // Upstream frees every buffer once the filter has drained it, as the SSI
    // filter recycles its own: reading one later is a use-after-free.
    let chunks: Vec<Vec<u8>> = (0..4).map(|i| text(400 + 100 * i)).collect();
    let mut f = filter(None, 4, 4096);
    let mut up = Upstream::default();

    for chunk in &chunks {
        assert_eq!(up.send(&mut f, &[Part::Data(chunk)]), OK);
        assert!(f.host().buffered);
    }
    assert_eq!(up.send(&mut f, &[Part::Last(&[])]), OK);

    assert_eq!(decode(&f.host().sent), chunks.concat());
    assert!(up.held.is_empty());
}

#[test]
fn slow_client_with_few_small_buffers() {
    let body = text(3000);
    let mut f = filter(Some(5), 2, 64);
    let mut up = Upstream::default();

    up.send(&mut f, &[Part::Data(&body)]);
    up.send(&mut f, &[Part::Last(&[])]);

    let mut calls = 0;
    while f.host().last.is_none() {
        calls += 1;
        assert!(calls < 100_000, "the response never completes");
        assert_ne!(up.resume(&mut f), NGX_ERROR as ngx_int_t);
    }

    let model = f.host();
    assert_eq!(model.temp_bufs, 2);
    assert_eq!(decode(&model.sent), body);
    assert!(up.held.is_empty());
}

#[test]
fn flush_makes_everything_so_far_decodable() {
    let (a, b) = (text(1200), text(800));
    let mut f = filter(None, 4, 4096);
    let mut up = Upstream::default();

    assert_eq!(up.send(&mut f, &[Part::Flush(&a)]), OK);
    let model = f.host();
    assert_eq!(model.flushes, [model.sent.len()]);
    assert_eq!(decode_prefix(&model.sent), a);
    assert!(!model.buffered);

    // A flush with nothing new still reaches the client.
    assert_eq!(up.send(&mut f, &[Part::Flush(&[])]), OK);
    assert_eq!(f.host().flushes.len(), 2);

    assert_eq!(up.send(&mut f, &[Part::Last(&b)]), OK);
    assert_eq!(decode(&f.host().sent), [a, b].concat());
}

#[test]
fn empty_and_zero_length_buffers() {
    let a = text(500);
    let mut f = filter(None, 4, 4096);
    let mut up = Upstream::default();

    let parts = [
        Part::Data(&[]),
        Part::Data(&a),
        Part::Data(&[]),
        Part::Last(&[]),
    ];
    assert_eq!(up.send(&mut f, &parts), OK);
    assert_eq!(decode(&f.host().sent), a);

    let mut f = filter(None, 4, 4096);
    assert_eq!(up.send(&mut f, &[Part::Last(&[])]), OK);
    assert!(decode(&f.host().sent).is_empty());
}

#[test]
fn file_buffers_are_rejected() {
    let mut f = filter(None, 4, 4096);
    let mut up = Upstream::default();

    assert_eq!(up.send(&mut f, &[Part::File]), NGX_ERROR as ngx_int_t);
    assert!(f.is_done());
    assert_eq!(f.sizes(), None);
    assert_eq!(
        *f.host().errors.borrow(),
        ["brotli filter got a file buffer"]
    );
}

#[test]
fn reused_encoders_produce_identical_responses() {
    let body = text(2000);
    let mut responses = Vec::new();
    for _ in 0..2 {
        let mut f = filter(None, 4, 4096);
        let mut up = Upstream::default();
        assert_eq!(up.send(&mut f, &[Part::Last(&body)]), OK);
        responses.push(f.host().sent.clone());
    }
    assert_eq!(responses[0], responses[1]);
    assert_eq!(decode(&responses[0]), body);
}

#[test]
fn window_follows_known_length() {
    assert_eq!(window_bits(-1, 19), 19);
    assert_eq!(window_bits(0, 19), 19);
    assert_eq!(window_bits(1, 19), 10);
    assert_eq!(window_bits(1024, 19), 10);
    assert_eq!(window_bits(1025, 19), 11);
    assert_eq!(window_bits(1 << 30, 19), 19);
    assert_eq!(window_bits(1 << 30, 24), 24);
}
