//! Chooses the encoder mode from the response's `Content-Type`.
//!
//! Brotli's font mode changes the distance alphabet, which suits the tables
//! of an uncompressed TrueType or OpenType font and makes such responses
//! slightly smaller. Everything else is compressed in generic mode: text mode
//! does not change the encoder's output, and WOFF and WOFF2 fonts are already
//! compressed.

use mbrotli::CompressionMode;

/// MIME types of uncompressed fonts, lowercase.
const FONT_TYPES: &[&[u8]] = &[
    b"application/font-sfnt",
    b"application/vnd.ms-fontobject",
    b"application/x-font-opentype",
    b"application/x-font-otf",
    b"application/x-font-truetype",
    b"application/x-font-ttf",
    b"font/collection",
    b"font/eot",
    b"font/opentype",
    b"font/otf",
    b"font/sfnt",
    b"font/truetype",
    b"font/ttf",
];

/// Returns the mode for a response of `content_type`, which may carry
/// parameters such as `; charset=utf-8`.
pub fn compression_mode(content_type: &[u8]) -> CompressionMode {
    let end = content_type
        .iter()
        .position(|&c| c == b';')
        .unwrap_or(content_type.len());
    let media_type = content_type[..end].trim_ascii();

    if FONT_TYPES
        .iter()
        .any(|t| t.eq_ignore_ascii_case(media_type))
    {
        CompressionMode::Font
    } else {
        CompressionMode::Generic
    }
}

#[cfg(test)]
mod tests {
    use mbrotli::CompressionMode::{Font, Generic};

    use super::{FONT_TYPES, compression_mode};

    #[test]
    fn uncompressed_fonts_use_font_mode() {
        for t in FONT_TYPES {
            assert_eq!(compression_mode(t), Font, "{}", t.escape_ascii());
        }
    }

    #[test]
    fn parameters_and_case_are_ignored() {
        assert_eq!(compression_mode(b"Font/TTF"), Font);
        assert_eq!(compression_mode(b"font/otf; charset=binary"), Font);
        assert_eq!(compression_mode(b" font/ttf ;x=y"), Font);
    }

    #[test]
    fn everything_else_uses_generic_mode() {
        for t in [
            &b""[..],
            b"text/html",
            b"text/html; charset=utf-8",
            b"application/javascript",
            b"image/svg+xml",
            b"font/woff",
            b"font/woff2",
            b"application/font-woff",
            b"font/ttfx",
            b"font/tt",
            b"xfont/ttf",
        ] {
            assert_eq!(compression_mode(t), Generic, "{}", t.escape_ascii());
        }
    }
}
