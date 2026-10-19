#!/usr/bin/env python3
"""Run recorded command-line regression cases without an upstream checkout."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
DEFAULT_BINARY = REPO / "target/x86_64-unknown-linux-musl/release/anarchy-rs"


def check_result(case, code, stdout):
    if case["kind"] == "error":
        return None if code > 0 else f"expected an error exit, got {code}"
    if code != 0:
        return f"expected exit 0, got {code}"
    if case["kind"] == "usage":
        if b"Usage: anarchy-rs " not in stdout:
            return "missing usage text"
        return None
    actual = hashlib.sha256(stdout).hexdigest()
    expected = case["stdout_sha256"]
    if actual != expected:
        return f"stdout SHA-256: expected {expected}, got {actual}"
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument("-k", "--filter", default="")
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f"executable not found: {binary}; run make build first")
    cases = json.loads((HERE / "cases.json").read_text())
    cases = [c for c in cases if args.filter in c["id"] + " " + shlex.join(c["args"])]
    if not cases:
        parser.error("no cases matched the filter")
    env = dict(os.environ, LC_ALL="C.UTF-8", USERNAME_ANARCHY_ROOT=str(REPO))
    failures = 0
    for case in cases:
        command = [str(binary), *case["args"]]
        try:
            result = subprocess.run(command, cwd=REPO, env=env, capture_output=True,
                                    timeout=30)
            detail = check_result(case, result.returncode, result.stdout)
            if detail and result.stderr:
                detail += "\n" + result.stderr.decode("utf-8", "replace")[:1000]
        except (OSError, subprocess.TimeoutExpired) as error:
            detail = str(error)
        if detail:
            failures += 1
            print(f"FAIL {case['id']}: {shlex.join(case['args'])}\n{detail}")
    print(f"CLI tests: {len(cases) - failures} passed, {failures} failed")
    return int(failures != 0)


if __name__ == "__main__":
    raise SystemExit(main())
