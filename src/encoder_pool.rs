//! Per-worker cache of idle [`Compressor`]s.
//!
//! NGINX runs filters on a single thread per worker process, so a thread-local
//! free list lets consecutive responses reuse an encoder's window, match finder
//! and staging buffers instead of allocating them for every request
//! (`brotli_reuse_memory`). How many idle compressors a worker keeps is
//! `brotli_idle_encoders`, and how much each keeps allocated is its retention
//! policy (`brotli_retention_policy`).
//!
//! Parallel compressors (`brotli_thread_pool`) are kept in a list of their own:
//! what they keep between responses is their idle segment workers. A response
//! holds its compressor, workers included, until it has been sent, also while
//! its segments wait for a thread; so the cache also counts the compressors
//! with workers that responses hold, and keeps no more of them in all than
//! `brotli_parallel_compressors` allows.
//!
//! Both limits are set once per worker process, by [`configure`].

use core::cell::{Cell, RefCell};

use mbrotli::compressor::parallel::{
    ParallelCompressor, ParallelConfig, ParallelConfigError, ParallelRetentionPolicy,
};
use mbrotli::{Compressor, ConfigError, EncoderConfig, RetentionPolicy};

/// The default of `brotli_idle_encoders`: how many idle compressors a worker
/// keeps, in each of its two caches.
pub const DEFAULT_IDLE_ENCODERS: usize = 4;

/// The default of `brotli_parallel_compressors`: how many parallel compressors
/// keep workers, idle or held by a response.
pub const DEFAULT_PARALLEL_COMPRESSORS: usize = 4;

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

    /// Parallel compressors from the cache that keep workers, idle or held by
    /// a response.
    static PARALLEL_WITH_WORKERS: Cell<usize> = const { Cell::new(0) };

    /// `brotli_idle_encoders`.
    static IDLE_LIMIT: Cell<usize> = const { Cell::new(DEFAULT_IDLE_ENCODERS) };

    /// `brotli_parallel_compressors`.
    static PARALLEL_LIMIT: Cell<usize> = const { Cell::new(DEFAULT_PARALLEL_COMPRESSORS) };
}

/// Sets the cache limits of this thread (`brotli_idle_encoders` and
/// `brotli_parallel_compressors`), before it compresses anything.
pub fn configure(idle_encoders: usize, parallel_compressors: usize) {
    IDLE_LIMIT.set(idle_encoders);
    PARALLEL_LIMIT.set(parallel_compressors);
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

/// A parallel compressor's place among those that keep workers
/// ([`PARALLEL_WITH_WORKERS`]), given up when it is dropped.
#[must_use]
pub struct ParallelLease {
    counted: bool,
}

impl Drop for ParallelLease {
    fn drop(&mut self) {
        if self.counted {
            PARALLEL_WITH_WORKERS.set(PARALLEL_WITH_WORKERS.get() - 1);
        }
    }
}

/// Returns a parallel compressor for `config` and `parallel`, preferring an
/// idle one: with the same encoder configuration, it keeps its workers. The
/// lease goes back to [`release_parallel`] with it.
pub fn acquire_parallel(
    config: EncoderConfig,
    parallel: ParallelConfig,
) -> Result<(ParallelCompressor, ParallelLease), ParallelConfigError> {
    let cached = IDLE_PARALLEL.with_borrow_mut(|idle| {
        let index = idle
            .iter()
            .rposition(|c| *c.encoder_config() == config)
            .or_else(|| idle.len().checked_sub(1))?;
        Some(idle.remove(index))
    });

    let Some(mut compressor) = cached else {
        let compressor = build_parallel(config, parallel)?;
        return Ok((compressor, ParallelLease { counted: false }));
    };
    // An idle compressor with workers is counted already.
    let lease = ParallelLease {
        counted: compressor.retained_worker_count() > 0,
    };
    if *compressor.encoder_config() != config {
        compressor.reconfigure(config)?;
    }
    if *compressor.parallel_config() != parallel {
        compressor.reconfigure_parallel(parallel);
    }
    Ok((compressor, lease))
}

/// Returns `compressor` to the cache, keeping as much of its workers'
/// memory as `retention` allows, unless as many other compressors keep workers
/// as `brotli_parallel_compressors` allows.
pub fn release_parallel(
    mut compressor: ParallelCompressor,
    retention: RetentionPolicy,
    mut lease: ParallelLease,
) {
    compressor.trim(match retention {
        RetentionPolicy::Aggressive => ParallelRetentionPolicy::Aggressive,
        RetentionPolicy::CurrentConfig => ParallelRetentionPolicy::CurrentPlan,
        RetentionPolicy::Bounded { max_bytes } => ParallelRetentionPolicy::Bounded { max_bytes },
        RetentionPolicy::ReleaseAll => ParallelRetentionPolicy::ReleaseAll,
    });

    let keeps = compressor.retained_worker_count() > 0;
    match (lease.counted, keeps) {
        // Still counted; it moves to the cache below.
        (true, true) => lease.counted = false,
        (true, false) => {}
        (false, true) if PARALLEL_WITH_WORKERS.get() < PARALLEL_LIMIT.get() => {
            PARALLEL_WITH_WORKERS.set(PARALLEL_WITH_WORKERS.get() + 1);
        }
        (false, true) => compressor.trim(ParallelRetentionPolicy::ReleaseAll),
        (false, false) => {}
    }
    drop(lease);

    let counted = |c: &ParallelCompressor| c.retained_worker_count() > 0;
    let limit = IDLE_LIMIT.get();
    if limit == 0 {
        if counted(&compressor) {
            PARALLEL_WITH_WORKERS.set(PARALLEL_WITH_WORKERS.get() - 1);
        }
        return;
    }

    IDLE_PARALLEL.with_borrow_mut(|idle| {
        if idle.len() >= limit {
            // Compressors without workers are cheap to build again: they go
            // first, and one is not kept in place of one with workers.
            let evicted = match idle.iter().position(|c| !counted(c)) {
                Some(index) => idle.remove(index),
                None if !counted(&compressor) => return,
                None => idle.remove(0),
            };
            if counted(&evicted) {
                PARALLEL_WITH_WORKERS.set(PARALLEL_WITH_WORKERS.get() - 1);
            }
        }
        idle.push(compressor);
    });
}

fn push_idle<T>(idle: &mut Vec<T>, compressor: T) {
    let limit = IDLE_LIMIT.get();
    if limit == 0 {
        return;
    }
    if idle.len() >= limit {
        idle.remove(0);
    }
    idle.push(compressor);
}

#[cfg(test)]
mod tests {
    use mbrotli::compressor::parallel::{
        ParallelCompressor, ParallelConfig, ParallelEncoderStatus, SegmentSize, TaskCount,
    };
    use mbrotli::{EncoderConfig, Operation, Quality, RetentionPolicy, Window};

    use super::{
        DEFAULT_IDLE_ENCODERS as MAX_IDLE, DEFAULT_RETENTION, IDLE, IDLE_PARALLEL,
        PARALLEL_WITH_WORKERS, ParallelLease, acquire, acquire_parallel, configure, release,
        release_parallel,
    };

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

    /// Takes a parallel compressor from the cache and compresses with it, so
    /// that it has a worker.
    fn busy_parallel(config: EncoderConfig) -> (ParallelCompressor, ParallelLease) {
        let parallel = ParallelConfig::from(SegmentSize::try_from(64 << 10).unwrap())
            .with_max_retained_workers(2);
        let (compressor, lease) = acquire_parallel(config, parallel).unwrap();
        let (mut session, queue) = compressor.into_session(TaskCount::ONE).unwrap();
        let input = b"parallel parallel parallel".repeat(100);
        let mut out = vec![0; 1 << 16];
        let mut op = Operation::Finish;
        let mut input: &[u8] = &input;
        loop {
            let progress = session.process(input, &mut out, op).unwrap();
            input = &input[progress.consumed..];
            while let Some(task) = queue.try_recv() {
                task.run();
            }
            if progress.status == ParallelEncoderStatus::Finished {
                break;
            }
            op = Operation::Finish;
        }
        let compressor = session.into_compressor();
        assert!(compressor.retained_worker_count() > 0);
        (compressor, lease)
    }

    fn parallel_with_workers() -> (usize, usize) {
        let idle = IDLE_PARALLEL.with_borrow(|idle| {
            idle.iter()
                .filter(|c| c.retained_worker_count() > 0)
                .count()
        });
        (PARALLEL_WITH_WORKERS.get(), idle)
    }

    #[test]
    fn workers_are_kept_for_a_bounded_number_of_parallel_compressors() {
        let q5 = config(Quality::Q5, 16);
        let held: Vec<_> = (0..MAX_IDLE + 2).map(|_| busy_parallel(q5)).collect();
        for (compressor, lease) in held {
            release_parallel(compressor, RetentionPolicy::Aggressive, lease);
        }
        assert_eq!(parallel_with_workers(), (MAX_IDLE, MAX_IDLE));

        // Taken again, they are still counted; dropped, they are not.
        let mut held: Vec<_> = (0..MAX_IDLE + 2).map(|_| busy_parallel(q5)).collect();
        assert_eq!(parallel_with_workers(), (MAX_IDLE, 0));
        for (compressor, lease) in held.drain(..2) {
            release_parallel(compressor, RetentionPolicy::ReleaseAll, lease);
        }
        assert_eq!(parallel_with_workers(), (MAX_IDLE - 2, 0));
        drop(busy_parallel(q5));
        assert_eq!(parallel_with_workers(), (MAX_IDLE - 2, 0));
        drop(held);
        assert_eq!(parallel_with_workers(), (0, 0));
    }

    #[test]
    fn the_limits_are_configurable() {
        let q1 = config(Quality::Q1, 16);
        let q5 = config(Quality::Q5, 16);

        configure(2, 1);
        let held: Vec<_> = (0..5)
            .map(|_| acquire(q1, DEFAULT_RETENTION).unwrap())
            .collect();
        held.into_iter().for_each(release);
        assert_eq!(idle_policies().len(), 2);

        let held: Vec<_> = (0..3).map(|_| busy_parallel(q5)).collect();
        for (compressor, lease) in held {
            release_parallel(compressor, RetentionPolicy::Aggressive, lease);
        }
        assert_eq!(parallel_with_workers(), (1, 1));
        assert_eq!(IDLE_PARALLEL.with_borrow(Vec::len), 2);

        // Nothing is kept, and nothing stays counted.
        configure(0, 1);
        let (compressor, lease) = busy_parallel(q5);
        release_parallel(compressor, RetentionPolicy::Aggressive, lease);
        release(acquire(q1, DEFAULT_RETENTION).unwrap());
        assert_eq!(idle_policies().len(), 1);
        assert_eq!(parallel_with_workers(), (1, 1));
        IDLE_PARALLEL.with_borrow_mut(Vec::clear);
        PARALLEL_WITH_WORKERS.set(0);
        let (compressor, lease) = busy_parallel(q5);
        release_parallel(compressor, RetentionPolicy::Aggressive, lease);
        assert_eq!(parallel_with_workers(), (0, 0));
    }
}
