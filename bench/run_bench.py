#!/usr/bin/env python3
"""Compare the Rust port with Ruby using Hyperfine.

Use --filter to select a workload, --quick to skip 100k rows and auto mode,
or --threads 1 for a serial Rust run. Completed results are printed immediately;
--json also saves them after each case. Hyperfine 1.x does not report peak RSS,
so its memory columns are left blank.
"""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "bench" / "data"
UPSTREAM = ROOT / "vendor" / "username-anarchy"
RUST = ROOT / "target" / "x86_64-unknown-linux-musl" / "release" / "username-anarchy-rs"
SHIM = ROOT / "tests" / "parity" / "oracle_compat.rb"
FB = UPSTREAM / "names" / "facebook"


@dataclass
class Case:
    label: str
    args: list[str]
    runs: int = 5
    warmup: int = 1
    shim: bool = False


def ensure_data() -> None:
    DATA.mkdir(parents=True, exist_ok=True)
    given = (FB / "firstnames-top1000.txt").read_text().splitlines()
    family = (FB / "lastnames-top1000.txt").read_text().splitlines()
    for count in (10_000, 20_000, 30_000, 100_000):
        path = DATA / f"names-{count // 1000}k.txt"
        if path.exists() and len(path.read_text().splitlines()) == count:
            continue
        rows = (f"{given[(i // 1000) % len(given)]} {family[i % len(family)]}"
                for i in range(count))
        path.write_text("\n".join(rows) + "\n")
        print(f"wrote {path.relative_to(ROOT)} ({count} rows)", file=sys.stderr)


def build_cases() -> list[Case]:
    cases = [Case("tiny invocation (anna key)", ["anna", "key"], runs=20, warmup=3)]
    for count in (10, 20, 30, 100):
        cases.append(Case(f"{count}k-row input file", ["-i", str(DATA / f"names-{count}k.txt")],
                          runs=5 if count == 10 else 2, warmup=1 if count == 10 else 0))
    dictionaries = ["--given-names", str(FB / "firstnames-top1000.txt"),
                    "--family-names", str(FB / "lastnames-top1000.txt")]
    cases += [
        Case("auto cross-product (1M people)",
             ["-a", *dictionaries, "-f", "first", "-m", "1"], runs=2, warmup=0),
        Case("1M substitutions (full output)",
             ["-s", "on", *dictionaries, "-f", "firstlast", ""], runs=3, shim=True),
        Case("1M substitutions, -m 5",
             ["-s", "on", *dictionaries, "-m", "5", "-f", "firstlast", ""], shim=True),
        Case("custom format, 10k combinations",
             ["-s", "on", "--given-names", str(FB / "firstnames-top100.txt"),
              "--family-names", str(FB / "lastnames-top100.txt"),
              "-F", "%givenname.%surname", ""], shim=True),
    ]
    return cases


def run_hyperfine(commands: list[list[str]], runs: int, warmup: int, env: dict) -> dict:
    with tempfile.TemporaryDirectory(prefix="anarchy-rs-bench-") as directory:
        report = Path(directory) / "results.json"
        argv = ["hyperfine", "-N", "--style", "none", "--warmup", str(warmup),
                "--runs", str(runs), "--export-json", str(report)]
        argv.extend(shlex.join(command) for command in commands)
        proc = subprocess.run(argv, cwd=ROOT, env=env, stdout=subprocess.DEVNULL,
                              stderr=subprocess.PIPE, text=True)
        if proc.returncode:
            raise SystemExit(f"hyperfine failed: {proc.stderr.strip()}")
        return json.loads(report.read_text())


def summarize(result: dict) -> dict:
    """Accept the timing-only 1.x export and the timing/memory 2.x export."""
    if "summary" in result:
        summary = result["summary"]
        timing = summary["time_wall_clock"]
        rss = summary.get("memory_peak_resident", {}).get("max")
    else:
        timing = result
        rss = None
    return {key: timing[key] for key in ("min", "mean", "max")} | {"rss_bytes": rss}


def fmt_s(seconds: float) -> str:
    if seconds < 0.001:
        return f"{seconds * 1e6:.0f} µs"
    if seconds < 1:
        return f"{seconds * 1e3:.1f} ms"
    return f"{seconds:.2f} s"


def fmt_rss(value: int | None) -> str:
    return "—" if value is None else f"{value / (1024 * 1024):.1f}"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--quick", action="store_true", help="skip 100k-row and auto workloads")
    parser.add_argument("--filter", help="run workloads whose names contain this text")
    parser.add_argument("--prepare-data", action="store_true", help="generate inputs and exit")
    parser.add_argument("--threads", type=int, help="Rust worker count")
    parser.add_argument("--json", type=Path, help="save results as JSON after each case")
    args = parser.parse_args()
    if args.threads is not None and args.threads < 1:
        parser.error("--threads must be positive")
    if args.prepare_data:
        ensure_data()
        return 0
    if not RUST.exists():
        parser.error(f"missing {RUST}; run make build first")
    if not shutil.which("hyperfine"):
        parser.error("hyperfine is not installed")

    cases = build_cases()
    if args.quick:
        cases = [case for case in cases if "100k" not in case.label
                 and not case.label.startswith("auto ")]
    if args.filter:
        cases = [case for case in cases if args.filter.casefold() in case.label.casefold()]
    if not cases:
        parser.error("no matching workloads")
    ensure_data()
    env = dict(os.environ)
    if args.threads:
        env["USERNAME_ANARCHY_THREADS"] = str(args.threads)

    print("| Case | Rust (min) | Ruby (min) | Speedup | Rust RSS MiB | Ruby RSS MiB |", flush=True)
    print("|---|---:|---:|---:|---:|---:|", flush=True)
    results = []
    for case in cases:
        print(f"benchmarking: {case.label}", file=sys.stderr)
        ruby = ["ruby"]
        if case.shim:
            ruby += ["-r", str(SHIM)]
        ruby += [str(UPSTREAM / "username-anarchy"), *case.args]
        data = run_hyperfine([[str(RUST), *case.args], ruby], case.runs, case.warmup, env)
        rust, original = (summarize(result) for result in data["results"])
        speedup = original["min"] / rust["min"]
        row = [case.label, fmt_s(rust["min"]), fmt_s(original["min"]), f"{speedup:,.0f}×",
               fmt_rss(rust["rss_bytes"]), fmt_rss(original["rss_bytes"])]
        print("| " + " | ".join(row) + " |", flush=True)
        results.append({"label": case.label, "rust": rust, "ruby": original, "speedup": speedup})
        if args.json:
            args.json.write_text(json.dumps(results, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
