# Performance measurements

Start with the [security and performance review](../security-review.md) for the changes and rollout limits.

- [Blob-reference index](record-blob-index.md): query gains, index size, and write cost.
- [Blob-reference checks and cached statements](blob-references.md): isolated SQL measurements and controls.
- [Mixed reads and legacy sessions](mixed-reads.md): large-page interference and connection pooling.
- [Small-page follow-up](small-page-followup.md): the current concurrent-list throughput conclusion.
- [Production-schema HTTP comparison](http-reads-production.md): earlier short measurements.
- [Historical HTTP comparison](http-reads.md): superseded fixture measurements.

Captured console output retains all measurement values. Before publication, trailing whitespace and extra blank lines at the end of text files were normalized for Git. Two early host snapshots reduced process executable paths to basenames; their publication notes identify that change. Timestamps, process IDs, CPU readings, response hashes, measurements, and as-run driver files are unchanged.
