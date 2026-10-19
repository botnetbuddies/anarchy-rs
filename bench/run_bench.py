#!/usr/bin/env python3
"""Phase 2 benchmark driver.

Runs the same invocations through the original Ruby tool and the Rust rewrite,
using `hyperfine` for timing (min/mean/max wall time and peak RSS), and prints a
Markdown table suitable for pasting into PERFORMANCE.md.

Notes
-----
* The original can only exercise the substitution engine on Ruby < 3.2 (or with
  the pre-3.2 `Object#=~` compatibility shim), because modern Ruby removed
  `Object#=~` and the script relies on it. Cases tagged `shim` therefore run
  Ruby with `tests/parity/oracle_compat.rb` preloaded.
* `-m 5` truncation cases are deliberately unfair in Rust's favour: Rust
  enumerates lazily and stops after the requested number of substitutions, while
  Ruby materialises the full product and only then slices it. They are included
  because they show the user-visible latency of the common `-m` workflow.
* Threads: Rust uses all available cores above a size threshold. Pass
  `--threads 1` for an apples-to-apples single-core comparison.

Usage
-----
    python3 bench/run_bench.py                # everything (~10 min; the Ruby side dominates)
    python3 bench/run_bench.py --quick        # skip the 100k and --auto cases
    python3 bench/run_bench.py --threads 1
    python3 bench/run_bench.py --json out.json

Rows are printed (and the JSON rewritten) as each case completes, so an
interrupted sweep still leaves usable results.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "bench"
DATA = BENCH / "data"
RUST = ROOT / "target" / "x86_64-unknown-linux-musl" / "release" / "username-anarchy-rs"
RUBY_SCRIPT = ROOT / "username-anarchy" / "username-anarchy"
SHIM = ROOT / "tests" / "parity" / "oracle_compat.rb"
FB = ROOT / "username-anarchy" / "names" / "facebook"


def ensure_data() -> None:
    DATA.mkdir(parents=True, exist_ok=True)
    giv = (FB / "firstnames-top1000.txt").read_text().splitlines()
    fam = (FB / "lastnames-top1000.txt").read_text().splitlines()
    for n, name in (
        (10_000, "names-10k.txt"),
        (20_000, "names-20k.txt"),
        (30_000, "names-30k.txt"),
        (100_000, "names-100k.txt"),
    ):
        path = DATA / name
        if path.exists() and sum(1 for _ in path.open()) == n:
            continue
        lines = [f"{giv[(i // 1000) % len(giv)]} {fam[i % len(fam)]}" for i in range(n)]
        path.write_text("\n".join(lines) + "\n")
        print(f"wrote {path.relative_to(ROOT)} ({n} rows)", file=sys.stderr)


def rust_cmd(args: str) -> str:
    return f"{RUST} {args}" if args else str(RUST)


def ruby_cmd(args: str, shim: bool = False) -> str:
    pre = f"ruby -r {SHIM} " if shim else "ruby "
    return f"{pre}{RUBY_SCRIPT} {args}".strip()


def run_hyperfine(cmd_a: str, cmd_b: str, runs: int, warmup: int) -> dict:
    """Time two commands, return hyperfine's parsed JSON for both."""
    jf = BENCH / ".hf.json"
    argv = [
        "hyperfine",
        "--style", "none",
        "--warmup", str(warmup),
        "--runs", str(runs),
        "--export-json", str(jf),
        cmd_a,
        cmd_b,
    ]
    proc = subprocess.run(argv, cwd=ROOT, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        raise SystemExit(f"hyperfine failed: {proc.stderr.decode(errors='replace')[:500]}")
    data = json.loads(jf.read_text())
    jf.unlink()
    return data


def fmt_s(seconds: float) -> str:
    if seconds < 0.001:
        return f"{seconds * 1e6:.0f} µs"
    if seconds < 1.0:
        return f"{seconds * 1e3:.1f} ms"
    return f"{seconds:.2f} s"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--quick", action="store_true", help="skip the slow 100k/auto cases")
    ap.add_argument("--prepare-data", action="store_true", help="generate benchmark inputs and exit")
    ap.add_argument("--threads", type=int, default=None, help="Rust worker threads")
    ap.add_argument("--json", type=Path, help="also write raw results as JSON")
    args = ap.parse_args()

    if args.prepare_data:
        ensure_data()
        return 0

    if not RUST.exists():
        raise SystemExit(f"missing {RUST}; run `cargo build --release` first")
    if not shutil.which("hyperfine"):
        raise SystemExit("hyperfine not found (pacman -S hyperfine)")

    ensure_data()
    env_prefix = ""
    if args.threads:
        env_prefix = f"env USERNAME_ANARCHY_THREADS={args.threads} "

    n10 = DATA / "names-10k.txt"
    n20 = DATA / "names-20k.txt"
    n30 = DATA / "names-30k.txt"
    n100 = DATA / "names-100k.txt"
    g1k = FB / "firstnames-top1000.txt"
    f1k = FB / "lastnames-top1000.txt"
    g100 = FB / "firstnames-top100.txt"
    f100 = FB / "lastnames-top100.txt"

    # (label, rust args, ruby args, shim?, runs, warmup)
    cases = [
        ("tiny invocation (`anna key`)", "anna key", "anna key", False, 20, 3),
        ("10k-row input file", f"-i {n10}", f"-i {n10}", False, 5, 1),
        ("20k-row input file", f"-i {n20}", f"-i {n20}", False, 2, 0),
        ("30k-row input file", f"-i {n30}", f"-i {n30}", False, 2, 0),
        ("100k-row input file", f"-i {n100}", f"-i {n100}", False, 2, 0),
        # Streamed in Rust; the original materializes all 10^6 people (and
        # re-reads the given-names file once per family).
        ("auto cross-product (1M people)",
         f"-a --given-names {g1k} --family-names {f1k} -f first -m 1",
         f"-a --given-names {g1k} --family-names {f1k} -f first -m 1",
         False, 2, 0),
        ("1M substitutions (full output)",
         f"-s on --given-names {g1k} --family-names {f1k} -f firstlast ''",
         f"-s on --given-names {g1k} --family-names {f1k} -f firstlast ''",
         True, 3, 1),
        ("1M substitutions, `-m 5`",
         f"-s on --given-names {g1k} --family-names {f1k} -m 5 -f firstlast ''",
         f"-s on --given-names {g1k} --family-names {f1k} -m 5 -f firstlast ''",
         True, 5, 1),
        ("custom `-F` (strict), 10k combinations",
         f"-s on --given-names {g100} --family-names {f100} -F '%givenname.%surname' ''",
         f"-s on --given-names {g100} --family-names {f100} -F '%givenname.%surname' ''",
         True, 5, 1),
    ]
    if args.quick:
        cases = [c for c in cases if "100k" not in c[0] and "cross-product" not in c[0]]

    rows = []
    raw = []
    hdr = ("Case", "Rust (min)", "Ruby (min)", "Speedup", "Rust RSS MiB", "Ruby RSS MiB")
    print(f"| {hdr[0]} | {hdr[1]} | {hdr[2]} | {hdr[3]} | {hdr[4]} | {hdr[5]} |", flush=True)
    print("|---|---:|---:|---:|---:|---:|", flush=True)
    for label, rargs, bargs, shim, runs, warmup in cases:
        print(f"  benchmarking: {label} ...", file=sys.stderr)
        a = env_prefix + rust_cmd(rargs)
        b = ruby_cmd(bargs, shim)
        data = run_hyperfine(a, b, runs, warmup)
        rust_r, ruby_r = data["results"][0], data["results"][1]
        rs = rust_r["summary"]
        rb = ruby_r["summary"]
        rmin = rs["time_wall_clock"]["min"]
        bmin = rb["time_wall_clock"]["min"]
        speed = bmin / rmin if rmin else float("inf")
        rows.append(
            (
                label,
                fmt_s(rmin),
                fmt_s(bmin),
                f"{speed:,.0f}×",
                f"{rs['memory_peak_resident']['max'] / (1024 * 1024):.1f}",
                f"{rb['memory_peak_resident']['max'] / (1024 * 1024):.1f}",
            )
        )
        # Print each completed row immediately: the Ruby side makes a full sweep
        # take minutes, and an interrupt should not throw away finished cases.
        print(f"| {rows[-1][0]} | {rows[-1][1]} | {rows[-1][2]} | {rows[-1][3]} | {rows[-1][4]} | {rows[-1][5]} |", flush=True)
        raw.append(
            {
                "label": label,
                "rust": {
                    "min": rmin,
                    "mean": rs["time_wall_clock"]["mean"],
                    "max": rs["time_wall_clock"]["max"],
                    "rss_bytes": rs["memory_peak_resident"]["max"],
                },
                "ruby": {
                    "min": bmin,
                    "mean": rb["time_wall_clock"]["mean"],
                    "max": rb["time_wall_clock"]["max"],
                    "rss_bytes": rb["memory_peak_resident"]["max"],
                },
                "speedup": speed,
            }
        )
        if args.json:
            # Keep completed cases even if the sweep is interrupted.
            args.json.write_text(json.dumps(raw, indent=2) + "\n")

    if args.json:
        print(f"\nraw results written to {args.json}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
