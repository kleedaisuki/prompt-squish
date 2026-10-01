//! Separate requested-allocation/peak-live measurements, never latency results.

#[path = "../../../scripts/perf/mechanism_support.rs"]
mod support;
mod workloads;

#[global_allocator]
static ALLOCATOR: support::TrackingAllocator = support::TrackingAllocator;

/// Count only operations on already-built, semantically checked fixtures.
fn main() {
    support::verify_allocation_accounting();
    workloads::run(workloads::Mode::Allocations);
}
