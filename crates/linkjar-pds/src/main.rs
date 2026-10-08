//! The LinkJar PDS binary. Configuration, wiring, CLI and telemetry arrive
//! with the units in `docs/plan.md`; today it only reports its version.

#[allow(clippy::print_stdout)] // a CLI reports on stdout
fn main() {
    println!(
        "linkjar-pds {} (unit 0 skeleton)",
        env!("CARGO_PKG_VERSION")
    );
}
