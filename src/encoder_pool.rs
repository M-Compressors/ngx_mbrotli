//! Per-worker cache of idle [`Compressor`]s.
//!
//! NGINX runs filters on a single thread per worker process, so a thread-local
//! free list lets consecutive responses reuse an encoder's window, match finder
//! and staging buffers instead of allocating them for every request
//! (`brotli_reuse_memory`). How much an idle encoder keeps allocated is its
//! retention policy (`brotli_retention_policy`).
//!
//! Parallel compressors (`brotli_thread_pool`) are kept in a list of their own:
//! what they keep between responses is their idle segment workers.

use core::cell::RefCell;

use mbrotli::compressor::parallel::{
    ParallelCompressor, ParallelConfig, ParallelConfigError, ParallelRetentionPolicy,
};
use mbrotli::{Compressor, ConfigError, EncoderConfig, RetentionPolicy};

/// How many idle compressors a worker keeps.
const MAX_IDLE: usize = 4;

/// The default retention policy: idle compressors keep every buffer.
///
/// A worker holds on to what its largest responses grew into, so none of it
/// is allocated again for later responses.
pub const DEFAULT_RETENTION: RetentionPolicy = RetentionPolicy::Aggressive;

thread_local! {
    /// Idle compressors, least recently released first.
    static IDLE: RefCell<Vec<Compressor>> = const { RefCell::new(Vec::new()) };

    /// Idle parallel compressors, least recently released first.
    static IDLE_PARALLEL: RefCell<Vec<ParallelCompressor>> = const { RefCell::new(Vec::new()) };
}

/// Builds a compressor that bypasses the cache.
pub fn build(config: EncoderConfig, retention: RetentionPolicy) -> Result<Compressor, ConfigError> {
    Compressor::builder(config)
        .with_retention(retention)
        .build()
}

/// Returns a compressor for `config` and `retention`, preferring an idle one.
///
/// A compressor's retention policy is fixed when it is built, so only idle
/// compressors with the same policy are reused; one built for another encoder
/// configuration is reconfigured.
pub fn acquire(
    config: EncoderConfig,
    retention: RetentionPolicy,
) -> Result<Compressor, ConfigError> {
    let cached = IDLE.with_borrow_mut(|idle| {
        let same_policy = |c: &Compressor| c.retention() == retention;
        let index = idle
            .iter()
            .rposition(|c| same_policy(c) && *c.config() == config)
            .or_else(|| idle.iter().rposition(same_policy))?;
        Some(idle.remove(index))
    });

    match cached {
        Some(mut compressor) => {
            if *compressor.config() != config {
                compressor.reconfigure(config)?;
            }
            Ok(compressor)
        }
        None => build(config, retention),
    }
}

/// Returns `compressor` to the cache, evicting the least recently released
/// idle compressor when the cache is full.
pub fn release(compressor: Compressor) {
    IDLE.with_borrow_mut(|idle| push_idle(idle, compressor));
}

/// Builds a parallel compressor that bypasses the cache.
pub fn build_parallel(
    config: EncoderConfig,
    parallel: ParallelConfig,
) -> Result<ParallelCompressor, ParallelConfigError> {
    ParallelCompressor::new(config, parallel)
}

/// Returns a parallel compressor for `config` and `parallel`, preferring an
/// idle one: with the same encoder configuration, it keeps its workers.
pub fn acquire_parallel(
    config: EncoderConfig,
    parallel: ParallelConfig,
) -> Result<ParallelCompressor, ParallelConfigError> {
    let cached = IDLE_PARALLEL.with_borrow_mut(|idle| {
        let index = idle
            .iter()
            .rposition(|c| *c.encoder_config() == config)
            .or_else(|| idle.len().checked_sub(1))?;
        Some(idle.remove(index))
    });

    match cached {
        Some(mut compressor) => {
            if *compressor.encoder_config() != config {
                compressor.reconfigure(config)?;
            }
            if *compressor.parallel_config() != parallel {
                compressor.reconfigure_parallel(parallel);
            }
            Ok(compressor)
        }
        None => build_parallel(config, parallel),
    }
}

/// Returns `compressor` to the cache, keeping as much of its workers'
/// memory as `retention` allows.
pub fn release_parallel(mut compressor: ParallelCompressor, retention: RetentionPolicy) {
    compressor.trim(match retention {
        RetentionPolicy::Aggressive => ParallelRetentionPolicy::Aggressive,
        RetentionPolicy::CurrentConfig => ParallelRetentionPolicy::CurrentPlan,
        RetentionPolicy::Bounded { max_bytes } => ParallelRetentionPolicy::Bounded { max_bytes },
        RetentionPolicy::ReleaseAll => ParallelRetentionPolicy::ReleaseAll,
    });
    IDLE_PARALLEL.with_borrow_mut(|idle| push_idle(idle, compressor));
}

fn push_idle<T>(idle: &mut Vec<T>, compressor: T) {
    if idle.len() >= MAX_IDLE {
        idle.remove(0);
    }
    idle.push(compressor);
}

#[cfg(test)]
mod tests {
    use mbrotli::{EncoderConfig, Operation, Quality, RetentionPolicy, Window};

    use super::{DEFAULT_RETENTION, IDLE, MAX_IDLE, acquire, release};

    fn config(quality: Quality, bits: u8) -> EncoderConfig {
        EncoderConfig::default()
            .with_quality(quality)
            .with_window(Window::standard(bits).unwrap())
    }

    fn compress(config: EncoderConfig, input: &[u8]) -> Vec<u8> {
        let mut session = acquire(config, DEFAULT_RETENTION)
            .unwrap()
            .into_session(Default::default())
            .unwrap();
        let mut out = vec![0; 1 << 16];
        let progress = session.process(input, &mut out, Operation::Finish).unwrap();
        out.truncate(progress.produced);
        release(session.into_compressor());
        out
    }

    fn idle_policies() -> Vec<RetentionPolicy> {
        IDLE.with_borrow(|idle| idle.iter().map(|c| c.retention()).collect())
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
            .map(|_| acquire(config(Quality::Q1, 16), DEFAULT_RETENTION).unwrap())
            .collect();
        held.into_iter().for_each(release);

        assert_eq!(idle_policies().len(), MAX_IDLE);
    }

    #[test]
    fn compressors_keep_the_requested_retention_policy() {
        let q1 = config(Quality::Q1, 16);
        release(acquire(q1, RetentionPolicy::ReleaseAll).unwrap());

        // An idle compressor with another policy is not handed out...
        let aggressive = acquire(q1, RetentionPolicy::Aggressive).unwrap();
        assert_eq!(aggressive.retention(), RetentionPolicy::Aggressive);
        assert_eq!(idle_policies(), [RetentionPolicy::ReleaseAll]);

        // ...but the matching one is.
        let release_all = acquire(q1, RetentionPolicy::ReleaseAll).unwrap();
        assert_eq!(release_all.retention(), RetentionPolicy::ReleaseAll);
        assert!(idle_policies().is_empty());
    }

    #[test]
    fn a_full_cache_evicts_the_least_recently_released() {
        let q1 = config(Quality::Q1, 16);
        release(acquire(q1, RetentionPolicy::ReleaseAll).unwrap());

        let held: Vec<_> = (0..MAX_IDLE)
            .map(|_| acquire(q1, RetentionPolicy::Aggressive).unwrap())
            .collect();
        held.into_iter().for_each(release);

        assert_eq!(idle_policies(), [RetentionPolicy::Aggressive; MAX_IDLE]);
    }
}
