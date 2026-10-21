# Concurrent small-page reads: follow-up

The earlier 2.4–8.0% throughput drop did not reproduce in the longer controlled comparison. Across 21 paired runs, candidate throughput had a median change of **+0.89%** and an exact median interval of **−0.42% to +3.66%**. This passes the predeclared local tolerance. These data do not justify a production optimization or establish a general speedup.

No runtime code or dependencies changed during this investigation. The candidate application-source aggregate matches the previously measured final mixed-workload snapshot. Its security checks and mixed-load implementation remain unchanged. New files contain benchmark code, analysis, and evidence; a historical harness comment points readers to the focused measurement.

## Protocol and decision rule

Both versions used identical `list_read_timing.rs` and fixture helpers. Each process created 20,000 posts with its production SQLite schema. Eight clients requested 50-record pages over loopback, with 100 warmups per client, a shared start barrier, and a five-second measurement window. Ten Tokio workers matched this host's original default.

Preflight required HTTP 200, JSON content type, 50 records, cursor `r9950`, expected record contents, and matching response SHA-256 across versions. Every timed response checked status and consumed exactly 11,930 bytes. Timing excluded task creation, sample merging, and sorting. Warmup waits and requests had deadlines.

Pairs alternated reference/candidate order. The confirmatory cohort was fixed at 21 pairs before it started; it completed without early stopping. The analysis uses whole paired runs, not individual requests as independent samples. The gate required median candidate/reference throughput ≥1.00 and an exact median interval lower bound ≥0.98. An upper bound below 1.00 would trigger the regression verdict; other outcomes are inconclusive. These are local engineering tolerances, not production SLOs.

## Results

Changes are candidate/reference throughput differences. The cohorts remain separate.

| Cohort | Pairs | Median | Arithmetic mean | Full paired range | Outcome |
| --- | ---: | ---: | ---: | ---: | --- |
| Existing HTTP executables, original short rows | 5 | +2.17% | +1.33% | −1.29% to +2.31% | Drop not reproduced |
| Focused exploratory runs | 7 | −1.17% | −0.08% | −5.33% to +3.59% | Inconclusive |
| Focused confirmatory runs | 21 | +0.89% | +0.96% | −11.42% to +9.92% | Pass |

The seven-pair exact interval was −5.33% to +3.59%, with 98.4375% coverage. The confirmatory interval uses the sixth and sixteenth ordered ratios, giving 97.3396% coverage under independent, continuous paired observations. Eight of its 21 pairs were slower. All 42 confirmatory HTTP processes passed response and measurement checks.

Independent review corrected the exploratory bootstrap interval's overstated nominal 95% coverage before final analysis. The exact method was fixed before the confirmatory cohort. The driver also rejects nonfinite, zero, inconsistent, or incomplete measurements. A startup output-parser failure was corrected before the complete focused cohorts ran. Original driver copies and outputs remain alongside corrected analysis; all observations from the reported cohorts remain available.

## Limits and reproduction

Measurements ran on an Apple M1 Pro with 16 GiB memory, macOS 27.0, and Rust 1.99.0. Task-owned builds and other benchmarks stopped during timing. Unrelated desktop activity continued, including processes exceeding 200% CPU in snapshots. The full paired range shows substantial host variation. Serial host effects can violate the interval's independence assumption. This workload does not establish production capacity or performance under different hardware, records, concurrency, or network conditions.

Build matched release harnesses before timing, then run the [driver](compare-list-reads.py):

```sh
python3 docs/performance/compare-list-reads.py \
  --reference /tmp/linkjar-pds-small-page-controls-20261021/baseline_steady \
  --candidate /tmp/linkjar-pds-small-page-controls-20261021/before_steady \
  --output /tmp/list-read-confirmation --pairs 21
```

The baseline is commit `77bf09e2e4e94079c38b86e47fb2d84e6ed36bbc`. [Focused provenance](2026-10-21-small-page-steady-provenance.json) records measured source aggregates, identical harness/helper hashes, and copied executable hashes. [Preserved controls](2026-10-21-small-page-controls.json) identify the earlier HTTP and mixed binaries. Temporary executable paths require rebuilding if removed.

Raw outputs, host snapshots, and as-run drivers accompany the [initial five pairs](2026-10-21-small-page-reproduction-1/results.json), [exploratory seven pairs](2026-10-21-small-page-steady-reproduction/results.json), and [confirmatory 21 pairs](2026-10-21-small-page-steady-confirmatory/results.json). The [confirmation console output](2026-10-21-small-page-steady-confirmatory/console.txt) lists every paired ratio.
