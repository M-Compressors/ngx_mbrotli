//! `Accept-Encoding` negotiation shared by the filter and static modules.

#[cfg(not(test))]
use ngx::ffi::ngx_http_request_t;

#[cfg(not(test))]
use crate::bitfields::ngx_mbrotli_r_claim_encoding;

/// Returns `true` when the client accepts the `br` content coding.
///
/// Follows google/ngx_brotli: the first `br` token decides, and it is rejected
/// only when its quality value is zero (`q=0`, `q=0.`, `q=0.0`, ... `q=0.000`).
pub fn accepts_brotli(header: &[u8]) -> bool {
    let mut from = 0;

    while let Some(offset) = find_br(&header[from..]) {
        let start = from + offset;
        let end = start + 2;
        from = end;

        let before = if start == 0 { b' ' } else { header[start - 1] };
        let after = header.get(end).copied().unwrap_or(b' ');

        if before != b',' && !is_space(before) {
            continue;
        }
        if after != b',' && after != b';' && !is_space(after) {
            continue;
        }

        return !has_zero_quality(&header[end..]);
    }

    false
}

/// Checks the eligibility of `r` for a `br` response and, when it is eligible,
/// tells the gzip modules to stay out of the way.
///
/// # Safety
///
/// `r` must point to a valid request.
#[cfg(not(test))]
pub unsafe fn check_request(r: *mut ngx_http_request_t) -> bool {
    let r = unsafe { &mut *r };

    if !core::ptr::eq(r, r.main) {
        return false;
    }

    let Some(entry) = (unsafe { r.headers_in.accept_encoding.as_ref() }) else {
        return false;
    };

    if !accepts_brotli(entry.value.as_bytes()) {
        return false;
    }

    unsafe { ngx_mbrotli_r_claim_encoding(r) };
    true
}

fn find_br(haystack: &[u8]) -> Option<usize> {
    haystack
        .windows(2)
        .position(|w| w[0].eq_ignore_ascii_case(&b'b') && w[1].eq_ignore_ascii_case(&b'r'))
}

fn is_space(c: u8) -> bool {
    c == b' ' || c == b'\t'
}

/// Parses an optional `;q=<value>` parameter and reports whether it is zero.
fn has_zero_quality(rest: &[u8]) -> bool {
    let mut it = rest.iter().copied().peekable();
    let skip_spaces = |it: &mut core::iter::Peekable<_>| {
        while it.next_if(|&c| is_space(c)).is_some() {}
    };

    skip_spaces(&mut it);
    if it.next() != Some(b';') {
        return false;
    }
    skip_spaces(&mut it);
    if !matches!(it.next(), Some(b'q' | b'Q')) {
        return false;
    }
    skip_spaces(&mut it);
    if it.next() != Some(b'=') {
        return false;
    }
    skip_spaces(&mut it);
    if it.next() != Some(b'0') {
        return false;
    }
    if it.next() != Some(b'.') {
        return true;
    }

    for _ in 0..3 {
        match it.next() {
            Some(b'0') => {}
            Some(b'1'..=b'9') => return false,
            _ => return true,
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::accepts_brotli;

    #[test]
    fn negotiation() {
        let accepted: &[&[u8]] = &[
            b"br",
            b"BR",
            b"gzip, deflate, br",
            b"br, gzip",
            b"gzip,br",
            b"gzip, br;q=1",
            b"gzip, br ; q = 0.5",
            b"br;q=0.001",
            b"br;q=1.0",
            b"br;level=5",
            b"brotli, br",
        ];
        for header in accepted {
            assert!(
                accepts_brotli(header),
                "{}",
                String::from_utf8_lossy(header)
            );
        }

        let rejected: &[&[u8]] = &[
            b"",
            b"gzip",
            b"b",
            b"brotli",
            b"xbr",
            b"gzip, br;q=0",
            b"br;q=0.",
            b"br;q=0.0",
            b"br;q=0.00",
            b"br;q=0.000",
            b"br; q=0, gzip",
            b"br;q=0.0000",
        ];
        for header in rejected {
            assert!(
                !accepts_brotli(header),
                "{}",
                String::from_utf8_lossy(header)
            );
        }
    }
}
