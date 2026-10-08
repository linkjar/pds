//! Repository tasks. `cargo xtask codegen` generates `crates/pds-types/src/generated`
//! from `lexicons/`; `--check` fails when the checked-in output is stale.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("codegen") => {
            eprintln!("codegen: not implemented yet (unit 0 in progress)");
            std::process::exit(2);
        }
        _ => {
            eprintln!("usage: cargo xtask codegen [--check]");
            std::process::exit(2);
        }
    }
}
