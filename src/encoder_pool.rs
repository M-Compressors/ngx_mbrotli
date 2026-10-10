//! Per-worker cache of idle [`Compressor`]s.
//!
//! NGINX runs filters on a single thread per worker process, so a thread-local
//! free list lets consecutive responses reuse an encoder's window, match finder
//! and staging buffers instead of allocating them for every request.

use core::cell::RefCell;

use mbrotli::{Compressor, ConfigError, EncoderConfig, RetentionPolicy};

/// How many idle compressors a worker keeps.
const MAX_IDLE: usize = 4;

/// How much memory a single idle compressor may keep allocated.
///
/// Encoders whose working set grows past this (high qualities with large
/// windows) release it after every stream, as they would without the cache.
const MAX_RETAINED_BYTES: usize = 8 << 20;

thread_local! {
    static IDLE: RefCell<Vec<Compressor>> = const { RefCell::new(Vec::new()) };
}

/// Returns a compressor for `config`, preferring an idle one.
pub fn acquire(config: EncoderConfig) -> Result<Compressor, ConfigError> {
    let cached = IDLE.with_borrow_mut(|idle| {
        let index = idle
            .iter()
            .rposition(|c| *c.config() == config)
            .or_else(|| idle.len().checked_sub(1))?;
        Some(idle.swap_remove(index))
    });

    match cached {
        Some(mut compressor) => {
            if *compressor.config() != config {
                compressor.reconfigure(config)?;
            }
            Ok(compressor)
        }
        None => Compressor::builder(config)
            .with_retention(RetentionPolicy::Bounded {
                max_bytes: MAX_RETAINED_BYTES,
            })
            .build(),
    }
}

/// Returns `compressor` to the cache, or drops it when the cache is full.
pub fn release(compressor: Compressor) {
    IDLE.with_borrow_mut(|idle| {
        if idle.len() < MAX_IDLE {
            idle.push(compressor);
        }
    });
}

#[cfg(test)]
mod tests {
    use mbrotli::{EncoderConfig, Operation, Quality, Window};

    use super::{MAX_IDLE, acquire, release};

    fn config(quality: Quality, bits: u8) -> EncoderConfig {
        EncoderConfig::default()
            .with_quality(quality)
            .with_window(Window::standard(bits).unwrap())
    }

    fn compress(config: EncoderConfig, input: &[u8]) -> Vec<u8> {
        let mut session = acquire(config)
            .unwrap()
            .into_session(Default::default())
            .unwrap();
        let mut out = vec![0; 1 << 16];
        let progress = session.process(input, &mut out, Operation::Finish).unwrap();
        out.truncate(progress.produced);
        release(session.into_compressor());
        out
    }

    #[test]
    fn reused_compressors_produce_identical_streams() {
        let input = b"reuse reuse reuse reuse reuse".repeat(100);
        let q5 = config(Quality::Q5, 19);
        let q9 = config(Quality::Q9, 12);

        let first = compress(q5, &input);
        // Switching configuration reconfigures an idle compressor.
        let other = compress(q9, &input);
        assert_eq!(compress(q5, &input), first);
        assert_eq!(compress(q9, &input), other);
    }

    #[test]
    fn idle_list_is_bounded() {
        let held: Vec<_> = (0..MAX_IDLE + 3)
            .map(|_| acquire(config(Quality::Q1, 16)).unwrap())
            .collect();
        held.into_iter().for_each(release);

        super::IDLE.with_borrow(|idle| assert_eq!(idle.len(), MAX_IDLE));
    }
}
