//! In-process core latency measurements with the ordinary uninstrumented allocator.
#[path = "support/core_suite.rs"]
mod core_suite;
#[path = "../../../scripts/perf/mechanism_support.rs"]
mod support;
fn main() {
    core_suite::run(core_suite::Mode::Latency);
}
