#!/usr/bin/env python3
"""Compare libtermy and libghostty-vt vt_library_bench executables.

Build both executables from scripts/vt-library-bench/vt_library_bench.c before
running this script. Each case runs in a separate process, alternating which
library goes first per pair; no builds or other benchmarks should overlap.
Ratios are libtermy/libghostty-vt throughput, so values above 1.0 mean libtermy
is faster. This is a comparison report, not a gate.
"""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import statistics
import subprocess
import sys


CASES = ("plain", "styled", "unicode", "varied-unicode", "combining", "varied-combining", "fragmented")
ENGINES = {"termy": "libtermy", "ghostty": "libghostty-vt"}


def executable(path):
    path = path.resolve()
    if not path.is_file() or not os.access(path, os.X_OK):
        raise argparse.ArgumentTypeError(f"not an executable file: {path}")
    return path


def fingerprint(path):
    digest = hashlib.sha256()
    with path.open("rb") as binary:
        for block in iter(lambda: binary.read(1024 * 1024), b""):
            digest.update(block)
    return {"path": str(path), "sha256": digest.hexdigest()}


def measure(binary, label, case, pair, order, args):
    command = [str(binary), str(args.mib), case]
    if args.frame:
        command.append("--frame")
    result = subprocess.run(
        command, capture_output=True, text=True, encoding="utf-8", errors="replace",
        timeout=args.timeout, check=False,
    )
    if result.returncode:
        raise RuntimeError(
            f"{label}/{case}/pair {pair}: exit {result.returncode}\n"
            f"{result.stdout[-4000:]}{result.stderr[-4000:]}"
        )
    engines = re.findall(r"^engine: ([a-z-]+)$", result.stdout, re.MULTILINE)
    if engines != [ENGINES[label]]:
        raise RuntimeError(f"{label}/{case}/pair {pair}: expected engine {ENGINES[label]}, got {engines}")
    pattern = rf"^{re.escape(case)}: ([0-9.]+) MiB/s; ([0-9]+) bytes; ([0-9.]+) seconds$"
    matches = re.findall(pattern, result.stdout, re.MULTILINE)
    rss = re.findall(r"^peak rss: (-?[0-9]+) KiB$", result.stdout, re.MULTILINE)
    if len(matches) != 1 or len(rss) != 1:
        raise RuntimeError(
            f"{label}/{case}/pair {pair}: expected one throughput record\n"
            f"{result.stdout[-4000:]}{result.stderr[-4000:]}"
        )
    throughput, byte_count, seconds = float(matches[0][0]), int(matches[0][1]), float(matches[0][2])
    if (not math.isfinite(throughput) or throughput <= 0 or
            not math.isfinite(seconds) or seconds <= 0 or byte_count < args.mib * 1024**2):
        raise RuntimeError(f"{label}/{case}/pair {pair}: invalid throughput record")
    return {
        "label": label, "case": case, "pair": pair, "order": order,
        "mib_per_second": throughput, "bytes": byte_count, "seconds": seconds,
        "peak_rss_kib": int(rss[0]), "stdout": result.stdout, "stderr": result.stderr,
    }


def summarize(samples):
    def stats(values):
        return {"median": statistics.median(values), "min": min(values), "max": max(values)}

    summaries = {}
    for case in CASES:
        by_label = {
            label: {sample["pair"]: sample for sample in samples
                    if sample["case"] == case and sample["label"] == label}
            for label in ENGINES
        }
        pairs = sorted(by_label["termy"].keys() & by_label["ghostty"].keys())
        if not pairs:
            continue
        throughput = {label: [by_label[label][pair]["mib_per_second"] for pair in pairs] for label in ENGINES}
        rss = {label: [by_label[label][pair]["peak_rss_kib"] for pair in pairs] for label in ENGINES}
        ratios = [termy / ghostty for termy, ghostty in zip(throughput["termy"], throughput["ghostty"])]
        summaries[case] = {
            "completed_pairs": len(pairs),
            **{f"{label}_mib_per_second": stats(throughput[label]) for label in ENGINES},
            **{f"{label}_peak_rss_kib": stats(rss[label]) for label in ENGINES},
            "paired_ratios": [{"pair": pair, "ratio": ratio} for pair, ratio in zip(pairs, ratios)],
            "ratio": stats(ratios),
        }
    return summaries


def markdown(report):
    mode = "feed + frame update per 64 KiB" if report["frame"] else "feed only"
    lines = [
        f"### {mode}: {report['status']}",
        "",
        f"{report['pairs']} alternating pairs per case at {report['mib']} MiB. "
        "Ratio is libtermy / libghostty-vt paired throughput; above 1.00x means libtermy is faster.",
        "",
        "| Case | libtermy MiB/s | libghostty-vt MiB/s | Median ratio | libtermy peak RSS | libghostty-vt peak RSS |",
        "| --- | ---: | ---: | ---: | ---: | ---: |",
    ]
    for case, result in report["summary"].items():
        lines.append(
            f"| {case} | {result['termy_mib_per_second']['median']:.1f} "
            f"| {result['ghostty_mib_per_second']['median']:.1f} "
            f"| {result['ratio']['median']:.2f}x "
            f"| {result['termy_peak_rss_kib']['median'] / 1024:.1f} MiB "
            f"| {result['ghostty_peak_rss_kib']['median'] / 1024:.1f} MiB |"
        )
    if report.get("error"):
        lines += ["", f"Error: `{report['error'].splitlines()[0]}`"]
    return "\n".join(lines) + "\n\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--termy", type=lambda value: executable(Path(value)), required=True)
    parser.add_argument("--ghostty", type=lambda value: executable(Path(value)), required=True)
    parser.add_argument("--mib", type=int, default=32)
    parser.add_argument("--pairs", type=int, default=6)
    parser.add_argument("--frame", action="store_true", help="pull a frame update after every 64 KiB block")
    parser.add_argument("--output", type=Path, help="JSON output file; defaults to stdout")
    parser.add_argument("--markdown", type=Path, help="append a Markdown table to this file")
    parser.add_argument("--timeout", type=float, default=300, help="seconds per executable/case (default: 300)")
    args = parser.parse_args()
    if not 1 <= args.mib <= 4096 or args.pairs < 1:
        parser.error("--mib must be in 1..=4096 and --pairs must be positive")
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error("--timeout must be finite and positive")
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)

    report = {
        "schema_version": 1, "started_at": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(), "mib": args.mib, "pairs": args.pairs,
        "frame": args.frame, "timeout_seconds": args.timeout,
        "termy": fingerprint(args.termy), "ghostty": fingerprint(args.ghostty),
        "samples": [], "summary": {}, "status": "running",
    }

    def save():
        report["summary"] = summarize(report["samples"])
        if args.output:
            args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    try:
        for case in CASES:
            for pair in range(1, args.pairs + 1):
                order = ("termy", "ghostty") if pair % 2 else ("ghostty", "termy")
                pair_samples = []
                for index, label in enumerate(order, start=1):
                    sample = measure(getattr(args, label), label, case, pair, index, args)
                    report["samples"].append(sample)
                    pair_samples.append(sample)
                    print(f"{case} pair {pair}/{args.pairs} {ENGINES[label]}: "
                          f"{sample['mib_per_second']:.3f} MiB/s", file=sys.stderr, flush=True)
                if pair_samples[0]["bytes"] != pair_samples[1]["bytes"]:
                    raise RuntimeError(f"{case}/pair {pair}: binaries processed different byte counts")
                save()
        report["status"] = "completed"
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        report["status"] = "error"
        report["error"] = str(error)
        print(report["error"], file=sys.stderr)
    finally:
        save()
        if not args.output:
            print(json.dumps(report, indent=2))
        if args.markdown:
            with args.markdown.open("a", encoding="utf-8") as output:
                output.write(markdown(report))
    for case, summary in report["summary"].items():
        print(f"{case}: libtermy/libghostty-vt median ratio {summary['ratio']['median']:.3f}x", file=sys.stderr)
    return 0 if report["status"] == "completed" else 1


if __name__ == "__main__":
    sys.exit(main())
