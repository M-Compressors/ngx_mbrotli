//! Formatting of the `$brotli_ratio` variable.

/// Formats `bytes_in / bytes_out` with two decimals, rounding half up.
pub fn format(bytes_in: u64, bytes_out: u64) -> String {
    let mut int = bytes_in / bytes_out;
    let mut frac = bytes_in * 100 / bytes_out % 100;

    if bytes_in * 1000 / bytes_out % 10 > 4 {
        frac += 1;
        if frac > 99 {
            int += 1;
            frac = 0;
        }
    }

    format!("{int}.{frac:02}")
}

#[cfg(test)]
mod tests {
    use super::format;

    #[test]
    fn rounds_half_up_to_two_decimals() {
        assert_eq!(format(100, 100), "1.00");
        assert_eq!(format(17000, 8000), "2.13");
        assert_eq!(format(1999, 1000), "2.00");
        assert_eq!(format(1, 3), "0.33");
        assert_eq!(format(2, 3), "0.67");
    }
}
