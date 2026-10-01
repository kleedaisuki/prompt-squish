//! Allocation observations only; instrumented timings must never be interpreted as latency.
#[path = "support/core_suite.rs"]
mod core_suite;
#[path = "../../../scripts/perf/mechanism_support.rs"]
mod support;
#[global_allocator]
static ALLOCATOR: support::TrackingAllocator = support::TrackingAllocator;
fn main() {
    core_suite::run(core_suite::Mode::Allocations);
}
