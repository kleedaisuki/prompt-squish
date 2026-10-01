//! Uninstrumented, filesystem-free measurements of shipping archive mechanisms.

#[path = "../../../scripts/perf/mechanism_support.rs"]
mod support;
mod workloads;

/// Keep allocation accounting out of the timing executable.
fn main() {
    support::verify_allocation_accounting();
    workloads::run(workloads::Mode::Latency);
}
