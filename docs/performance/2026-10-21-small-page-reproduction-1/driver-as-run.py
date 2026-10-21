#!/usr/bin/env python3
"""Paired local HTTP throughput signal; use compiled, matched read_timing binaries."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import statistics
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument('--baseline', type=Path, required=True)
parser.add_argument('--candidate', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--pairs', type=int, default=5)
args = parser.parse_args()
assert args.pairs >= 3
args.output.mkdir(parents=True, exist_ok=False)

def snapshot():
    rows = []
    for line in subprocess.check_output(['ps', '-axo', 'pid=,pcpu=,comm='], text=True).splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) == 3:
            rows.append({'pid': int(fields[0]), 'cpu_percent': float(fields[1]), 'command': os.path.basename(fields[2])})
    return {'utc': datetime.datetime.now(datetime.timezone.utc).isoformat(), 'processes': sorted(rows, key=lambda row: row['cpu_percent'], reverse=True)[:10]}

meta = {'binaries': {}, 'pairs': [], 'decision_rule': 'RED if median paired candidate/baseline list throughput is below 0.98 and at least 80% of pairs are slower. Otherwise the short loop is inconclusive, not proof of parity.'}
for name in ['baseline', 'candidate']:
    path = getattr(args, name).resolve()
    meta['binaries'][name] = {'path': str(path), 'sha256': hashlib.sha256(path.read_bytes()).hexdigest()}

for pair in range(1, args.pairs + 1):
    result = {'pair': pair, 'runs': {}}
    order = ['baseline', 'candidate'] if pair % 2 else ['candidate', 'baseline']
    result['order'] = order
    for name in order:
        before = snapshot()
        started = time.monotonic()
        run = subprocess.run([meta['binaries'][name]['path'], 'read_requests_timed', '--exact', '--ignored', '--nocapture', '--test-threads=1'], text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        elapsed = time.monotonic() - started
        after = snapshot()
        text = run.stdout
        (args.output / f'pair-{pair:02}-{name}.txt').write_text(text)
        assert run.returncode == 0, (name, pair, text)
        match = re.search(r'^\|\s*listRecords 50, 8 at a time: rate\s*\|\s*(\d+)\s+req/s', text, re.M)
        assert match, text
        row = next(line for line in text.splitlines() if re.match(r'^\|\s*listRecords 50, 8 at a time\s*\|', line))
        columns = [column.strip() for column in row.split('|') if column.strip()]
        result['runs'][name] = {'rate': int(match.group(1)), 'p50_us': int(columns[1]), 'p99_us': int(columns[3]), 'elapsed_seconds': elapsed, 'before': before, 'after': after}
    result['candidate_ratio'] = result['runs']['candidate']['rate'] / result['runs']['baseline']['rate']
    meta['pairs'].append(result)
    (args.output / 'results.json').write_text(json.dumps(meta, indent=2) + '\n')
    print(f"Pair {pair}: baseline={result['runs']['baseline']['rate']} candidate={result['runs']['candidate']['rate']} req/s, change={(result['candidate_ratio']-1)*100:+.2f}%", flush=True)
ratios = [row['candidate_ratio'] for row in meta['pairs']]
median = statistics.median(ratios)
slower = sum(ratio < 1 for ratio in ratios)
red = median < 0.98 and slower / args.pairs >= 0.8
meta['verdict'] = {'red': red, 'median_ratio': median, 'slower_pairs': slower, 'pairs': args.pairs}
(args.output / 'results.json').write_text(json.dumps(meta, indent=2) + '\n')
print(f"{'RED' if red else 'INCONCLUSIVE'}: median paired change={(median-1)*100:+.2f}%; slower pairs={slower}/{args.pairs}", flush=True)
raise SystemExit(1 if red else 0)
