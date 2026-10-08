# Fuzz targets

`cargo +nightly fuzz run <target>` with cargo-fuzz. Every parser of untrusted
bytes gets a target (docs/SPEC.md section 14.5); the nightly workflow runs
each for ten minutes. Targets: the syntax parsers, JSON into the data model
and DAG-CBOR into the data model.
