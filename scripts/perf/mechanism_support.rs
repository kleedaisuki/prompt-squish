//! Shared, dependency-free support for explicit in-process mechanism benchmarks.
//!
//! Include with `#[path = "../../../scripts/perf/mechanism_support.rs"] mod support`.
//! Latency executables must not install `TrackingAllocator`; allocation executables
//! install it as their global allocator and never report its timings as latency.
//! Fixture construction and semantic assertions belong outside these functions.

#![allow(dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Bounded sampling configuration; environment overrides are useful for hosted CI.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Number of independently emitted samples per mechanism/dimension combination.
    pub samples: usize,
    /// Number of untimed operation invocations before adaptation or allocation counts.
    pub warmup: usize,
    /// Target duration of a latency batch; setup and output serialization are excluded.
    pub target_ns: u64,
    /// Hard bound on repeated operations in an individual latency batch.
    pub max_iterations: usize,
    /// Fixed repetitions in an allocation sample; peak live bytes are not divided by it.
    pub allocation_iterations: usize,
}

impl Config {
    /// Read positive bounded controls without silently accepting invalid CI settings.
    pub fn from_env() -> Self {
        if smoke_requested() {
            return Self {
                samples: 1,
                warmup: 1,
                target_ns: 1,
                max_iterations: 1,
                allocation_iterations: 1,
            };
        }
        Self {
            samples: setting("MECHANISM_SAMPLES", 7, 100),
            warmup: setting("MECHANISM_WARMUP", 2, 100),
            target_ns: setting("MECHANISM_TARGET_US", 10_000, 1_000_000) as u64 * 1_000,
            max_iterations: setting("MECHANISM_MAX_ITERATIONS", 1_024, 1_000_000),
            allocation_iterations: setting("MECHANISM_ALLOCATION_ITERATIONS", 1, 10_000),
        }
    }
}

/// Debug-profile executions always exercise tiny fixtures, never the full suite.
///
/// Cargo does not reliably pass `--test` to harness-free bench executables during
/// all-target testing. Debug assertions identify those ordinary test-profile runs;
/// shipping-profile bench builds have debug assertions disabled and collect samples.
pub fn smoke_requested() -> bool {
    cfg!(debug_assertions) || std::env::args().any(|argument| argument == "--test")
}

/// Parse a single positive environment setting before timing begins.
fn setting(name: &str, default: usize, maximum: usize) -> usize {
    match std::env::var(name) {
        Ok(value) => {
            let value: usize = value.parse().expect("invalid mechanism benchmark setting");
            assert!(value > 0 && value <= maximum, "out-of-range setting {name}");
            value
        }
        Err(std::env::VarError::NotPresent) => default,
        Err(error) => panic!("invalid setting {name}: {error}"),
    }
}

/// Execute and destroy each returned value inside the measured region.
fn batch<T>(operation: &mut impl FnMut() -> T, iterations: usize) -> Duration {
    let start = Instant::now();
    for _ in 0..iterations {
        let output = black_box(operation());
        drop(black_box(output));
    }
    start.elapsed()
}

/// Emit latency-only observations using the uninstrumented process allocator.
///
/// Ownership costs of the returned result, including its destruction, are included.
/// A borrowed/reference-returning control must be labeled explicitly by its owner;
/// it does not represent an API-compatible replacement for an owned result.
pub fn latency<T>(
    suite: &str,
    workload: &str,
    mechanism: &str,
    dimensions: &[(&str, u64)],
    mut operation: impl FnMut() -> T,
) {
    let config = Config::from_env();
    for _ in 0..config.warmup {
        drop(black_box(operation()));
    }
    let mut iterations = 1;
    loop {
        let elapsed = batch(&mut operation, iterations).as_nanos();
        if elapsed >= u128::from(config.target_ns) || iterations == config.max_iterations {
            break;
        }
        iterations = iterations.saturating_mul(2).min(config.max_iterations);
    }
    for sample in 0..config.samples {
        let elapsed = batch(&mut operation, iterations).as_nanos();
        let mut record = header(
            suite, "latency", workload, mechanism, dimensions, sample, iterations,
        );
        record.push_str(&format!(",\"elapsed_ns\":{elapsed}}}"));
        println!("{record}");
    }
}

/// Exclude per-operation input preparation and returned-output destruction.
///
/// Each operation has its own Instant interval, summed per batch. This avoids
/// holding many large prepared inputs simultaneously but adds a clock-read floor;
/// owners should include a similarly segmented no-op control for very small work.
/// Destruction of consumed owned inputs is part of the API call and stays included.
pub fn latency_prepared<I, T>(
    suite: &str,
    workload: &str,
    mechanism: &str,
    dimensions: &[(&str, u64)],
    mut prepare: impl FnMut() -> I,
    mut operation: impl FnMut(I) -> T,
) {
    let config = Config::from_env();
    for _ in 0..config.warmup {
        drop(black_box(operation(prepare())));
    }
    let mut iterations = 1;
    loop {
        let elapsed = prepared_batch(&mut prepare, &mut operation, iterations);
        if elapsed >= u128::from(config.target_ns) || iterations == config.max_iterations {
            break;
        }
        iterations = iterations.saturating_mul(2).min(config.max_iterations);
    }
    for sample in 0..config.samples {
        let elapsed = prepared_batch(&mut prepare, &mut operation, iterations);
        let mut record = header(
            suite, "latency", workload, mechanism, dimensions, sample, iterations,
        );
        record = record.replace("\"drop_included\":true", "\"drop_included\":false");
        record.push_str(&format!(
            ",\"timing\":\"segmented_operation\",\"setup_excluded\":true,\"elapsed_ns\":{elapsed}}}"
        ));
        println!("{record}");
    }
}

/// Sum operation-only intervals while dropping prepared inputs/outputs individually.
fn prepared_batch<I, T>(
    prepare: &mut impl FnMut() -> I,
    operation: &mut impl FnMut(I) -> T,
    iterations: usize,
) -> u128 {
    let mut elapsed = 0;
    for _ in 0..iterations {
        let input = prepare();
        let start = Instant::now();
        let output = black_box(operation(black_box(input)));
        elapsed += start.elapsed().as_nanos();
        drop(black_box(output));
    }
    elapsed
}

/// Allocator used only by the allocation executable, never production or latency.
///
/// All successful allocation lifetimes are accounted even outside measurement, so
/// pre-existing fixture memory is subtracted from the sample's live-byte peak.
pub struct TrackingAllocator;

/// Check allocation/reallocation/peak accounting before any benchmark measurements.
///
/// This small fixture uses explicit calls to the wrapper, works in both binaries,
/// and never makes huge allocations merely to force an OS failure path.
pub fn verify_allocation_accounting() {
    // Harness-free main functions execute this debug smoke-selection check even
    // when Cargo does not generate/run a conventional Rust test harness.
    #[cfg(debug_assertions)]
    {
        assert!(smoke_requested());
        let config = Config::from_env();
        assert_eq!(config.samples, 1);
        assert_eq!(config.warmup, 1);
        assert_eq!(config.max_iterations, 1);
        assert_eq!(config.allocation_iterations, 1);
    }
    let original = Layout::from_size_align(32, 8).unwrap();
    let grown = Layout::from_size_align(64, 8).unwrap();
    let shrunk = Layout::from_size_align(24, 8).unwrap();
    let zeroed = Layout::from_size_align(16, 8).unwrap();
    let scope = Scope::start();
    // SAFETY: Layouts are valid; each successful realloc supplies the following
    // allocation's matching Layout, and each final pointer is deallocated once.
    unsafe {
        let pointer = TrackingAllocator.alloc(original);
        assert!(!pointer.is_null());
        let pointer = TrackingAllocator.realloc(pointer, original, grown.size());
        assert!(!pointer.is_null());
        let extra = TrackingAllocator.alloc_zeroed(zeroed);
        assert!(!extra.is_null());
        let pointer = TrackingAllocator.realloc(pointer, grown, shrunk.size());
        assert!(!pointer.is_null());
        TrackingAllocator.dealloc(extra, zeroed);
        TrackingAllocator.dealloc(pointer, shrunk);
    }
    let observed = (
        CALLS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
        DEALLOCS.load(Ordering::Relaxed),
        REALLOCS.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed).saturating_sub(scope.baseline),
        LIVE.load(Ordering::Relaxed),
    );
    let baseline = scope.baseline;
    drop(scope);
    assert_eq!(observed, (4, 136, 2, 2, 80, baseline));
}

/// Logical requested bytes currently live across all successful wrapper allocations.
static LIVE: AtomicU64 = AtomicU64::new(0);
/// Enables per-sample counters only after their baseline and zeroed traffic are ready.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Exclusive sample ownership; distinct from enabling instrumentation during setup.
static MEASURING: AtomicBool = AtomicBool::new(false);
/// Successful allocation requests, including successful reallocations.
static CALLS: AtomicU64 = AtomicU64::new(0);
/// Complete requested sizes, not copied bytes or positive realloc growth alone.
static BYTES: AtomicU64 = AtomicU64::new(0);
/// Explicit dealloc calls during the sample.
static DEALLOCS: AtomicU64 = AtomicU64::new(0);
/// Successful realloc requests during the sample.
static REALLOCS: AtomicU64 = AtomicU64::new(0);
/// Maximum observed logical live gauge, seeded from the sample's entry baseline.
static PEAK: AtomicU64 = AtomicU64::new(0);

/// Count an allocation without allocating, taking locks, formatting or panicking.
fn allocated(bytes: usize) {
    let live = LIVE.fetch_add(bytes as u64, Ordering::Relaxed) + bytes as u64;
    if ACTIVE.load(Ordering::Relaxed) {
        CALLS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(bytes as u64, Ordering::Relaxed);
        PEAK.fetch_max(live, Ordering::Relaxed);
    }
}

// SAFETY: Every pointer/Layout is forwarded unchanged to System, including failure
// semantics. Accounting uses only atomics, never recursively allocates, and does not
// inspect payload memory. The benchmark owns and quiesces all threads at boundaries.
unsafe impl GlobalAlloc for TrackingAllocator {
    /// Forward allocation; failed requests do not create a counted live allocation.
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: GlobalAlloc's caller supplies a valid Layout; System owns the result.
        let result = unsafe { System.alloc(layout) };
        if !result.is_null() {
            allocated(layout.size());
        }
        result
    }

    /// Preserve System's zeroing semantics while counting a successful request once.
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: The same valid Layout is forwarded to the underlying allocator.
        let result = unsafe { System.alloc_zeroed(layout) };
        if !result.is_null() {
            allocated(layout.size());
        }
        result
    }

    /// Remove the old live size after forwarding its matching pointer/Layout.
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: GlobalAlloc guarantees pointer/Layout identify a live allocation.
        unsafe { System.dealloc(pointer, layout) };
        LIVE.fetch_sub(layout.size() as u64, Ordering::Relaxed);
        if ACTIVE.load(Ordering::Relaxed) {
            DEALLOCS.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Count successful realloc as a request for the complete new size, not its delta.
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: Pointer/Layout/new_size satisfy GlobalAlloc's realloc contract.
        let result = unsafe { System.realloc(pointer, layout, new_size) };
        if result.is_null() {
            return result;
        }
        let live = if new_size >= layout.size() {
            let growth = (new_size - layout.size()) as u64;
            LIVE.fetch_add(growth, Ordering::Relaxed) + growth
        } else {
            let shrink = (layout.size() - new_size) as u64;
            LIVE.fetch_sub(shrink, Ordering::Relaxed) - shrink
        };
        if ACTIVE.load(Ordering::Relaxed) {
            CALLS.fetch_add(1, Ordering::Relaxed);
            REALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(new_size as u64, Ordering::Relaxed);
            PEAK.fetch_max(live, Ordering::Relaxed);
        }
        result
    }
}

/// Single process-wide counting interval, with an unwind-safe disable guard.
struct Scope {
    /// Existing live requested bytes, never reset when starting a new sample.
    baseline: u64,
}

impl Scope {
    /// Reset counters while benchmark-owned threads are quiescent.
    fn start() -> Self {
        assert!(
            MEASURING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok(),
            "overlapping allocation samples"
        );
        let baseline = LIVE.load(Ordering::SeqCst);
        CALLS.store(0, Ordering::Relaxed);
        BYTES.store(0, Ordering::Relaxed);
        DEALLOCS.store(0, Ordering::Relaxed);
        REALLOCS.store(0, Ordering::Relaxed);
        PEAK.store(baseline, Ordering::Relaxed);
        assert!(
            !ACTIVE.swap(true, Ordering::SeqCst),
            "overlapping allocation samples"
        );
        Self { baseline }
    }
}

impl Drop for Scope {
    /// Disable accounting even if a measured operation unwinds.
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::SeqCst);
        MEASURING.store(false, Ordering::SeqCst);
    }
}

/// Emit allocation-only samples; do not mix counting-allocator timing with latency.
///
/// Callers must quiesce their own workers before entering and before returning from
/// each operation. Concurrent allocations inside a joined operation are counted
/// atomically for the entire process, not attributed to a particular thread.
pub fn allocations<T>(
    suite: &str,
    workload: &str,
    mechanism: &str,
    dimensions: &[(&str, u64)],
    mut operation: impl FnMut() -> T,
) {
    let config = Config::from_env();
    for _ in 0..config.warmup {
        drop(black_box(operation()));
    }
    for sample in 0..config.samples {
        let scope = Scope::start();
        for _ in 0..config.allocation_iterations {
            drop(black_box(operation()));
        }
        // Returning outputs were dropped before stopping; printing happens afterward.
        let peak = PEAK.load(Ordering::SeqCst).saturating_sub(scope.baseline);
        let retained = LIVE.load(Ordering::SeqCst).saturating_sub(scope.baseline);
        let calls = CALLS.load(Ordering::Relaxed);
        let bytes = BYTES.load(Ordering::Relaxed);
        let deallocations = DEALLOCS.load(Ordering::Relaxed);
        let reallocations = REALLOCS.load(Ordering::Relaxed);
        drop(scope);
        let mut record = header(
            suite,
            "allocations",
            workload,
            mechanism,
            dimensions,
            sample,
            config.allocation_iterations,
        );
        record.push_str(&format!(
            ",\"allocation_calls\":{calls},\"allocated_bytes\":{bytes},\"deallocation_calls\":{deallocations},\"reallocation_calls\":{reallocations},\"peak_live_delta_bytes\":{peak},\"retained_live_delta_bytes\":{retained}}}",
        ));
        println!("{record}");
    }
}

/// Count only the owned API call, excluding input preparation and result destruction.
///
/// One prepared operation per sample is deliberate: summing peak live deltas across
/// sequential operations would not be a meaningful peak. Inputs are built before
/// the baseline gauge, outputs are dropped after disabling the counting interval.
pub fn allocations_prepared<I, T>(
    suite: &str,
    workload: &str,
    mechanism: &str,
    dimensions: &[(&str, u64)],
    mut prepare: impl FnMut() -> I,
    mut operation: impl FnMut(I) -> T,
) {
    let config = Config::from_env();
    for _ in 0..config.warmup {
        drop(black_box(operation(prepare())));
    }
    for sample in 0..config.samples {
        let input = prepare();
        let scope = Scope::start();
        let output = black_box(operation(black_box(input)));
        let peak = PEAK.load(Ordering::SeqCst).saturating_sub(scope.baseline);
        let retained = LIVE.load(Ordering::SeqCst).saturating_sub(scope.baseline);
        let calls = CALLS.load(Ordering::Relaxed);
        let bytes = BYTES.load(Ordering::Relaxed);
        let deallocations = DEALLOCS.load(Ordering::Relaxed);
        let reallocations = REALLOCS.load(Ordering::Relaxed);
        drop(scope);
        drop(black_box(output));
        let mut record = header(
            suite,
            "allocations",
            workload,
            mechanism,
            dimensions,
            sample,
            1,
        );
        record = record.replace("\"drop_included\":true", "\"drop_included\":false");
        record.push_str(&format!(
            ",\"setup_excluded\":true,\"allocation_calls\":{calls},\"allocated_bytes\":{bytes},\"deallocation_calls\":{deallocations},\"reallocation_calls\":{reallocations},\"peak_live_delta_bytes\":{peak},\"retained_live_delta_bytes\":{retained}}}",
        ));
        println!("{record}");
    }
}

/// Serialize trusted labels and dynamic dimensions using JSON-compatible escaping.
fn header(
    suite: &str,
    mode: &str,
    workload: &str,
    mechanism: &str,
    dimensions: &[(&str, u64)],
    sample: usize,
    iterations: usize,
) -> String {
    let mut result = String::from("{\"schema\":\"xmlsquish.mechanism.v1\"");
    for (name, value) in [
        ("suite", suite),
        ("mode", mode),
        ("workload", workload),
        ("mechanism", mechanism),
    ] {
        result.push_str(&format!(",{}:{}", quoted(name), quoted(value)));
    }
    result.push_str(&format!(
        ",\"sample\":{sample},\"iterations\":{iterations},\"drop_included\":true,\"dimensions\":{{"
    ));
    for (index, (name, value)) in dimensions.iter().enumerate() {
        if index > 0 {
            result.push(',');
        }
        result.push_str(&format!("{}:{value}", quoted(name)));
    }
    result.push('}');
    result
}

/// Escape control characters without relying on Rust Debug's non-JSON escapes.
fn quoted(value: &str) -> String {
    let mut result = String::from("\"");
    for character in value.chars() {
        match character {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            character if character <= '\u{1f}' => {
                result.push_str(&format!("\\u{:04x}", character as u32))
            }
            character => result.push(character),
        }
    }
    result.push('"');
    result
}
