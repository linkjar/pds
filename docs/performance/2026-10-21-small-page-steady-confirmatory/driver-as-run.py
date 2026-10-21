#!/usr/bin/env python3
"""Compare compiled list_read_timing binaries using paired local runs.

This is a diagnostic for a controlled local experiment, not a production SLO
or a CI performance assertion. Build first and stop other project workloads.
"""

import argparse
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import statistics
import subprocess
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def snapshot():
    rows = []
    output = subprocess.check_output(["ps", "-axo", "pid=,pcpu=,comm="], text=True)
    for line in output.splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) == 3:
            rows.append({
                "pid": int(fields[0]),
                "cpu_percent": float(fields[1]),
                "command": os.path.basename(fields[2]),
            })
    return {
        "utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "processes": sorted(rows, key=lambda row: row["cpu_percent"], reverse=True)[:10],
    }


def fields(output, prefix):
    marker = prefix + " "
    lines = [line.split(marker, 1)[1] for line in output.splitlines() if marker in line]
    assert len(lines) == 1, (prefix, output)
    return dict(field.split("=", 1) for field in lines[0].split())


def interval(ratios):
    # Exact order-statistic interval for the population median of independent,
    # continuous paired ratios. Choose the narrowest symmetric interval whose
    # binomial coverage is at least95%; at n=7 this is min..max (98.4375%).
    count = len(ratios)
    selected, coverage = 1, 1 - 2 / (2 ** count)
    for index in range(1, (count + 1) // 2 + 1):
        tail = sum(math.comb(count, j) for j in range(index)) / (2 ** count)
        level = 1 - 2 * tail
        if level < 0.95:
            break
        selected, coverage = index, level
    ordered = sorted(ratios)
    return ordered[selected - 1], ordered[count - selected], coverage, selected


def validate(preflight, stats):
    assert preflight["records"] == "50"
    assert preflight["cursor"] == "r9950"
    assert preflight["workers"] == "10"
    assert len(bytes.fromhex(preflight["sha256"])) == 32
    size = int(preflight["bytes"])
    requests = int(stats["requests"])
    total_bytes = int(stats["body_bytes"])
    elapsed, rate = float(stats["elapsed_s"]), float(stats["req_per_s"])
    assert size > 0 and requests > 0 and total_bytes > 0
    assert math.isfinite(elapsed) and elapsed >= 5
    assert math.isfinite(rate) and rate > 0
    assert stats["clients"] == "8" and stats["warmups"] == "100"
    assert total_bytes == requests * size
    # The harness prints elapsed seconds to6decimals and rate to3decimals.
    tolerance = 0.0005 + requests * 0.0000005 / (elapsed * (elapsed - 0.0000005))
    assert math.isclose(rate, requests / elapsed, rel_tol=0, abs_tol=tolerance + 1e-9)
    assert 0 <= int(stats["p50_us"]) <= int(stats["p95_us"]) <= int(stats["p99_us"])


def validate_cohort(meta):
    planned = meta.get("planned_pairs", 7)  # Explicit support for the original seven-pair cohort.
    assert type(planned) is int and planned >= 7
    assert len(meta["pairs"]) == planned, "the complete predeclared cohort is required"
    assert [row["pair"] for row in meta["pairs"]] == list(range(1, planned + 1))
    assert all(set(row["runs"]) == {"reference", "candidate"} for row in meta["pairs"])


def analyze(meta, output):
    validate_cohort(meta)
    ratios = [row["candidate_ratio"] for row in meta["pairs"]]
    assert len(ratios) >= 7 and all(math.isfinite(ratio) and ratio > 0 for ratio in ratios)
    median = statistics.median(ratios)
    lower, upper, coverage, order = interval(ratios)
    verdict = "PASS" if median >= 1 and lower >= 0.98 else "RED" if upper < 1 else "INCONCLUSIVE"
    meta["analysis"] = {
        "unit": "paired process-level candidate/reference throughput ratio",
        "interval": "exact binomial/order-statistic median interval, at least95% coverage",
        "assumption": "independent continuous paired ratios; serial host effects can violate independence",
        "pass": "median >= 1.00 and exact interval lower bound >= 0.98",
        "red": "exact interval upper bound < 1.00",
        "otherwise": "inconclusive; uncertainty is not evidence of parity",
        "scope": "local engineering tolerances, not production SLOs",
    }
    meta["analysis_script_sha256"] = digest(Path(__file__))
    meta["verdict"] = {
        "status": verdict,
        "median_ratio": median,
        "arithmetic_mean_ratio": statistics.fmean(ratios),
        "minimum_ratio": min(ratios),
        "maximum_ratio": max(ratios),
        "exact_median_interval": [lower, upper],
        "coverage": coverage,
        "order_statistic_1based": order,
        "slower_pairs": sum(ratio < 1 for ratio in ratios),
        "pairs": len(ratios),
    }
    (output / "results.json").write_text(json.dumps(meta, indent=2) + "\n")
    print(
        f"{verdict}: median paired change={(median - 1) * 100:+.2f}%; "
        f"exact{coverage * 100:.4f}% median interval="
        f"[{(lower - 1) * 100:+.2f}%, {(upper - 1) * 100:+.2f}%]; "
        f"mean change={(statistics.fmean(ratios) - 1) * 100:+.2f}%; "
        f"range=[{(min(ratios) - 1) * 100:+.2f}%, {(max(ratios) - 1) * 100:+.2f}%]",
        flush=True,
    )
    return {"PASS": 0, "RED": 1, "INCONCLUSIVE": 2}[verdict]


def reanalyze(output):
    meta = json.loads((output / "results.json").read_text())
    validate_cohort(meta)
    expected = None
    for pair in meta["pairs"]:
        for label, run in pair["runs"].items():
            raw = output / f"pair-{pair['pair']:02}-{label}.txt"
            text = raw.read_text()
            preflight, stats = fields(text, "LIST_PREFLIGHT"), fields(text, "LIST_STEADY")
            validate(preflight, stats)
            assert run["exit_code"] == 0
            assert run["preflight"] == preflight and run["stats"] == stats
            if expected is None:
                expected = preflight
            assert preflight == expected
        pair["candidate_ratio"] = float(pair["runs"]["candidate"]["stats"]["req_per_s"]) / float(pair["runs"]["reference"]["stats"]["req_per_s"])
    if (output / "results-as-run.json").exists():
        meta["analysis_revision"] = "Revalidated unchanged raw runs with finite/positive/rounding checks and exact median interval. The original small-n bootstrap interval was exploratory and its nominal95% label overstated coverage; retained in results-as-run.json and driver-as-run.py. Independent review requested this correction before the final results were analyzed."
    return analyze(meta, output)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path)
    parser.add_argument("--candidate", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--reanalyze", type=Path)
    parser.add_argument("--pairs", type=int, default=7)
    args = parser.parse_args()
    if args.reanalyze:
        return reanalyze(args.reanalyze)
    assert args.reference and args.candidate and args.output
    assert args.pairs >= 7, "at least seven paired runs are required"
    args.output.mkdir(parents=True, exist_ok=False)
    (args.output / "driver-as-run.py").write_bytes(Path(__file__).read_bytes())
    meta = {
        "script_sha256": digest(Path(__file__)),
        "planned_pairs": args.pairs,
        "analysis_plan": {
            "interval": "exact symmetric binomial median interval with coverage at least95%",
            "pass": "median ratio >= 1.00 and lower interval bound >= 0.98",
            "red": "upper interval bound < 1.00",
            "otherwise": "INCONCLUSIVE",
            "stopping": "complete every planned pair; stop early only on invalid measurement or HTTP failure",
        },
        "binaries": {},
        "pairs": [],
    }
    for label in ["reference", "candidate"]:
        path = getattr(args, label).resolve()
        meta["binaries"][label] = {"path": str(path), "sha256": digest(path)}

    expected = None
    for pair in range(1, args.pairs + 1):
        order = ["reference", "candidate"] if pair % 2 else ["candidate", "reference"]
        result = {"pair": pair, "order": order, "runs": {}}
        for label in order:
            executable = Path(meta["binaries"][label]["path"])
            assert digest(executable) == meta["binaries"][label]["sha256"]
            before = snapshot()
            started = time.monotonic()
            run = subprocess.run(
                [str(executable), "list_records_steady_timed", "--exact", "--ignored",
                 "--nocapture", "--test-threads=1"],
                text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=90,
            )
            elapsed = time.monotonic() - started
            after = snapshot()
            raw = args.output / f"pair-{pair:02}-{label}.txt"
            raw.write_text(run.stdout)
            assert run.returncode == 0, (label, pair, str(raw), run.stdout)
            preflight = fields(run.stdout, "LIST_PREFLIGHT")
            if expected is None:
                expected = preflight
            assert preflight == expected, ("response mismatch", expected, preflight)
            stats = fields(run.stdout, "LIST_STEADY")
            validate(preflight, stats)
            result["runs"][label] = {
                "preflight": preflight,
                "stats": stats,
                "exit_code": run.returncode,
                "process_elapsed_seconds": elapsed,
                "before": before,
                "after": after,
            }
        reference = float(result["runs"]["reference"]["stats"]["req_per_s"])
        candidate = float(result["runs"]["candidate"]["stats"]["req_per_s"])
        result["candidate_ratio"] = candidate / reference
        meta["pairs"].append(result)
        (args.output / "results.json").write_text(json.dumps(meta, indent=2) + "\n")
        print(
            f"Pair {pair}: reference={reference:.3f} candidate={candidate:.3f} req/s, "
            f"change={(candidate / reference - 1) * 100:+.2f}%",
            flush=True,
        )
    return analyze(meta, args.output)


if __name__ == "__main__":
    raise SystemExit(main())
