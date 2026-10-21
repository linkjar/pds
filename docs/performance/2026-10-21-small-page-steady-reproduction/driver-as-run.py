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
import random
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
    # Resample whole paired runs, never individual HTTP requests. The seed and
    # percentile method are fixed so repeated analysis produces the same result.
    rng = random.Random(20261021)
    draws = sorted(
        statistics.median(rng.choices(ratios, k=len(ratios)))
        for _ in range(20_000)
    )
    return draws[math.floor(0.025 * len(draws))], draws[math.floor(0.975 * len(draws))]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--reference", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pairs", type=int, default=7)
    args = parser.parse_args()
    assert args.pairs >= 7, "at least seven paired runs are required"
    args.output.mkdir(parents=True, exist_ok=False)
    meta = {
        "script_sha256": digest(Path(__file__)),
        "binaries": {},
        "pairs": [],
        "analysis": {
            "unit": "paired process-level candidate/reference throughput ratio",
            "interval": "percentile bootstrap of paired median; 20000 draws; seed20261021",
            "pass": "median >= 1.00 and two-sided95% interval lower bound >= 0.98",
            "red": "two-sided95% interval upper bound < 1.00",
            "otherwise": "inconclusive; uncertainty is not evidence of parity",
            "scope": "local engineering tolerances, not production SLOs",
        },
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
            assert preflight["records"] == "50"
            assert preflight["cursor"] == "r9950"
            assert preflight["workers"] == "10"
            if expected is None:
                expected = preflight
            assert preflight == expected, ("response mismatch", expected, preflight)
            stats = fields(run.stdout, "LIST_STEADY")
            assert stats["clients"] == "8" and stats["warmups"] == "100"
            assert float(stats["elapsed_s"]) >= 5
            assert int(stats["body_bytes"]) == int(stats["requests"]) * int(preflight["bytes"])
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
    ratios = [row["candidate_ratio"] for row in meta["pairs"]]
    median = statistics.median(ratios)
    lower, upper = interval(ratios)
    verdict = "PASS" if median >= 1 and lower >= 0.98 else "RED" if upper < 1 else "INCONCLUSIVE"
    meta["verdict"] = {
        "status": verdict,
        "median_ratio": median,
        "paired_median_95_percent_interval": [lower, upper],
        "slower_pairs": sum(ratio < 1 for ratio in ratios),
        "pairs": args.pairs,
    }
    (args.output / "results.json").write_text(json.dumps(meta, indent=2) + "\n")
    print(
        f"{verdict}: median paired change={(median - 1) * 100:+.2f}%; "
        f"paired95% interval=[{(lower - 1) * 100:+.2f}%, {(upper - 1) * 100:+.2f}%]",
        flush=True,
    )
    return {"PASS": 0, "RED": 1, "INCONCLUSIVE": 2}[verdict]


if __name__ == "__main__":
    raise SystemExit(main())
