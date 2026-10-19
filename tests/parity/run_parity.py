#!/usr/bin/env python3
"""Parity tests: Rust rewrite vs. the original Ruby oracle.

Runs the same invocation through three programs and compares stdout:

  * ruby          -- the original `username-anarchy` script, unmodified
  * ruby+compat   -- the original with the pre-Ruby-3.2 `Object#=~` shim, which
                     is the only way to exercise the substitution engine on a
                     modern Ruby (see oracle_compat.rb)
  * rust          -- the compiled rewrite

Rules
-----
* If stock Ruby exits 0, it is the oracle, and we additionally require that the
  compat oracle produces the same set (proving the shim is faithful).
* If stock Ruby crashes (non-zero exit), the compat oracle is used instead.
* If both oracles crash, the Rust binary must also fail (non-zero exit).
* Output sets are compared for generation; format listings are compared
  byte for byte. Serial/parallel ordering is checked by Rust unit tests.

Usage:
    python3 tests/parity/run_parity.py            # run everything
    python3 tests/parity/run_parity.py --selftest # verify the harness itself
    python3 tests/parity/run_parity.py -k substr  # filter by substring
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
RUBY_DIR = REPO / "vendor" / "username-anarchy"
RUBY_SCRIPT = RUBY_DIR / "username-anarchy"
COMPAT = HERE / "oracle_compat.rb"
FIX = HERE / "fixtures"

DEFAULT_RUST = REPO / "target/x86_64-unknown-linux-musl/release/username-anarchy-rs"
RUST_BIN = Path(os.environ.get("RUST_BIN", DEFAULT_RUST))

RUBY = os.environ.get("RUBY", "ruby")

G = str(FIX / "given.txt")
F = str(FIX / "family.txt")
FB = RUBY_DIR / "names" / "facebook"
FBG = str(FB / "firstnames-top100.txt")
FBF = str(FB / "lastnames-top100.txt")
GD = str(FIX / "given_dup.txt")
GEMPTY = str(FIX / "given_empty.txt")
GNONL = str(FIX / "given_nonl.txt")

STATIC_PLUGINS = [
    "first", "firstlast", "first.last", "firstlast[8]", "first[4]last[4]",
    "firstl", "f.last", "flast", "lfirst", "l.first", "lastf", "last",
    "last.f", "last.first", "FLast", "first1", "fl", "fmlast",
    "firstmiddlelast", "fml", "FL", "FirstLast", "First.Last", "Last",
]


def run(cmd, cwd, extra_env=None):
    env = dict(os.environ)
    # Match the encoding the tool is normally run under. With LC_ALL=C, Ruby
    # treats ARGV as ASCII-8BIT and character-based slicing (`s[0..0]`)
    # silently becomes byte-based, producing mangled non-ASCII output; the
    # Rust port operates on UTF-8 characters, which is Ruby's behaviour in any
    # UTF-8 locale.
    env["LC_ALL"] = "C.UTF-8"
    if extra_env:
        env.update(extra_env)
    p = subprocess.run(cmd, cwd=str(cwd), env=env,
                       stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return p.returncode, p.stdout, p.stderr


def run_ruby(args, compat=False):
    cmd = [RUBY]
    if compat:
        cmd += ["-r", str(COMPAT)]
    cmd += [str(RUBY_SCRIPT)] + args
    return run(cmd, RUBY_DIR)


def run_rust(args):
    return run([str(RUST_BIN)] + args, REPO,
               {"USERNAME_ANARCHY_ROOT": str(RUBY_DIR)})


def lines_set(b: bytes):
    return set(b.splitlines())


def normalise_recognise(b: bytes):
    """Mask the knowingly non-deterministic plugin-object rendering."""
    out = []
    for line in b.splitlines():
        i = line.find(b"[#<Plugin")
        if i != -1:
            line = line[:i] + b"[#<Plugin ...>]"
        out.append(line)
    return set(out)


# ---------------------------------------------------------------------------
# Cases
# ---------------------------------------------------------------------------
# kind:
#   names      -> set-compare stdout
#   listfmt    -> exact stdout compare
#   recognise  -> set-compare, masking plugin-object renders
#   usage      -> both exit 0, no candidate leakage required
#   error      -> rust must exit non-zero

def build_cases():
    C = []

    # Additional cases from the Go repository's broader compatibility matrix.
    C.append(("names", ["Mary", "Beth", "Key", "-F", "%%f"]))
    for case in ["true", "false"]:
        C.append(("names", ["Mary-Jane", "De La Cruz", "-F", "%%f", "-C", case]))
        C.append(("names", ["ΟΣ", "ΣΟΣ", "-C", case]))
    C.append(("listfmt", ["-v", "-l"]))
    C.append(("names", ["Anna", "Key", "-f", "first.last", "-@", "@ΟΣ.EXAMPLE"]))

    # --- small / common cases -------------------------------------------
    C += [("names", ["anna"]),
          ("names", ["anna", "key"]),
          ("names", ["anna", "boom", "key"]),
          ("names", ["O'Toole"]),
          ("names", ["von Grippenvud"]),
          ("names", ["Anna", "Key"]),
          ("usage", ["anna", "key", "extra", "ignored"]),
          ("names", ["ANNA", "BOOM", "KEY"]),
          ("names", ["de la", "cruz"]),
          ("names", [""]),
          ]

    # --- every static plugin, with 3 name shapes ------------------------
    for p in STATIC_PLUGINS:
        C.append(("names", ["-f", p, "anna", "key"]))
        C.append(("names", ["-f", p, "anna", "boom", "key"]))
        C.append(("names", ["-f", p, "anna"]))          # missing lastname
        C.append(("names", ["-f", p, "", "key"]))       # missing firstname

    # --- default plugin set --------------------------------------------
    C += [("names", ["anna", "key"]),
          ("names", ["-v", "anna", "key"]),
          ]

    # --- substitution ---------------------------------------------------
    sub = ["-s", "on", "--given-names", G, "--family-names", F]
    C.append(("names", sub + ["-f", "firstlast", "anna"]))
    C.append(("names", sub + ["-s", "on", "-m", "3", "-f", "firstlast", "anna"]))
    C.append(("names", sub + ["-m", "1", "-f", "firstlast", "anna"]))
    C.append(("names", sub + ["-f", "first.last", "anna"]))
    C.append(("names", sub + ["-f", "fmlast", "anna"]))
    C.append(("names", sub + ["-f", "fmlast", "anna", "key"]))
    C.append(("names", sub + ["-f", "firstmiddlelast", "anna"]))
    C.append(("names", sub + ["-f", "firstl", "anna"]))
    C.append(("names", sub + ["-f", "fl", "anna"]))
    C.append(("names", sub + ["-f", "fml", "anna"]))
    C.append(("names", sub + ["-f", "FL", "anna"]))
    C.append(("names", sub + ["-f", "FirstLast", "anna"]))
    C.append(("names", sub + ["-f", "first1", "anna"]))
    C.append(("names", sub + ["-f", "flast", "", "key"]))
    C.append(("names", sub + ["-f", "first.last", "anna", "", "key"]))
    C.append(("names", sub + ["-F", "a-b-k", "anna"]))
    C.append(("names", sub + ["-F", "%f-%l", "anna"]))
    C.append(("names", sub + ["-F", "%F.%L", "anna"]))
    C.append(("names", sub + ["-F", "anna%D", "anna"]))
    C.append(("names", sub + ["-F", "%givenname", "anna"]))
    C.append(("names", sub + ["-F", "%Givenname", "anna"]))
    C.append(("names", sub + ["-F", "a%l", "anna"]))
    C.append(("names", sub + ["-F", "%a-z", "anna"]))
    C.append(("names", sub + ["-F", "%A-Z", "anna"]))
    C.append(("names", sub + ["-F", "%DD", "anna"]))
    C.append(("names", sub + ["-F", "%D", "anna"]))
    C.append(("names", sub + ["-F", "%DDD", "anna"]))
    C.append(("names", sub + ["-F", "%a-z-%A-Z", "anna"]))
    C.append(("names", sub + ["-F", "x%givenname%D", "anna"]))
    # duplicate wordlist entries must not be de-duplicated before -m
    C.append(("names", ["-s", "on", "--given-names", GD, "--family-names", F,
                        "-m", "3", "-f", "firstlast", "key"]))
    # empty wordlist -> zero results
    C.append(("names", ["-s", "on", "--given-names", GEMPTY, "--family-names", F,
                        "-f", "firstlast", "anna"]))
    # default (facebook 10k) dictionary, truncated so it stays fast
    C.append(("names", ["-s", "on", "-m", "4", "-f", "firstlast", "anna"]))
    C.append(("names", ["-s", "on", "-m", "4", "-f", "last", "anna"]))

    # --- suffix / case --------------------------------------------------
    C += [("names", ["-@", "@EXAMPLE.COM", "anna", "key"]),
          ("names", ["-@", "", "anna", "key"]),
          ("names", ["--suffix", ".corp", "anna", "key"]),
          ("names", ["-C", "false", "-f", "FirstLast", "Anna", "Key"]),
          ("names", ["-C", "TRUE", "-f", "FirstLast", "Anna", "Key"]),
          ("names", ["-C", "false", "anna", "key"]),
          ("names", ["--case-insensitive", "false", "-@", "@X", "Anna", "Key"]),
          ]

    # --- custom -F ------------------------------------------------------
    C += [("names", ["-F", "%f.%l", "andrew", "horton"]),
          ("names", ["-F", "v-annakey", "andrew", "horton"]),
          ("names", ["-F", "anna.key", "andrew", "horton"]),
          ("names", ["-F", "ak", "andrew", "horton"]),
          ("names", ["-F", "ABK", "andrew", "boom", "horton"]),
          ("names", ["-F", '"%f.%l"', "andrew", "horton"]),
          ("names", ["-F", "annaboomkey", "andrew", "boom", "horton"]),
          ("names", ["-F", "key.anna", "andrew", "horton"]),
          ("names", ["-F", "v-%f%l", "-C", "false", "Andrew", "Horton"]),
          ]

    # --- input files ----------------------------------------------------
    for f in ["test-names.txt", "test-names2.txt", "test-names3.txt"]:
        C.append(("names", ["-i", str(RUBY_DIR / f)]))
        C.append(("names", ["-i", str(RUBY_DIR / f), "-f", "first.last"]))
        C.append(("names", ["-i", str(RUBY_DIR / f), "-v", "-f", "firstlast"]))
    for f in ["input_space.txt", "input_csv_header.txt", "input_tab.txt",
              "input_full.csv", "input_unknown.csv", "input_mixedcase_header.csv",
              "input_emptyfields.csv", "input_firstlastinitial.csv"]:
        C.append(("names", ["-i", str(FIX / f)]))
        C.append(("names", ["-i", str(FIX / f), "-f", "first.last"]))
        C.append(("names", ["-i", str(FIX / f), "-v", "-f", "fl"]))
    C.append(("names", ["-i", str(FIX / "input_full.csv"), "-f", "firstlast"]))
    C.append(("names", ["-i", str(FIX / "input_summary.csv")]))
    # Wide rows must not silently lose name columns after the stack buffer.
    C.append(("names", ["-i", str(FIX / "input_wide.csv"), "-f", "first.last"]))
    # Repeated headers use the final column, even when that value is empty.
    for plugin in ["firstlast", "last"]:
        C.append(("names", ["-i", str(FIX / "input_repeated_columns.csv"), "-f", plugin]))
    # File input is lowercased before character-based truncation, including
    # Unicode mappings that expand the encoded byte length (İ -> i + dot).
    for plugin in ["firstlast[8]", "first[4]last[4]", "first.last"]:
        for case in ["true", "false"]:
            C.append(("names", ["-i", str(FIX / "input_unicode.csv"),
                                "-f", plugin, "-C", case]))
    # input file + cmdline names together
    C.append(("names", ["-i", str(FIX / "input_space.txt"), "zed", "qux"]))

    # --- auto / country -------------------------------------------------
    C += [("names", ["-a", "--country", "france"]),
          ("names", ["-a", "--country", "uk"]),
          ("names", ["-a", "--country", "us"]),
          ("names", ["-a", "--country", "FRANCE", "-f", "first.last"]),
          ("names", ["--auto", "--country", "france", "-s", "on", "-m", "2",
                     "-f", "firstlast"]),
          ("names", ["-a", "--given-names", G, "--family-names", F]),
          ("names", ["-a", "--given-names", G, "--family-names", F,
                     "-f", "first.last", "-s", "on", "-m", "2"]),
          ("names", ["-a", "--given-names", G, "--family-names", F, "-v",
                     "-f", "firstlast"]),
          # `None` entries (the original's `strip!`-returns-nil quirk) and
          # verbose output on the streamed cross-product.
          ("names", ["-a", "--given-names", GNONL, "--family-names", F,
                     "-f", "firstlast", "-s", "on"]),
          ("names", ["-a", "--given-names", GNONL, "--family-names", F,
                     "-f", "FLast", "-s", "on"]),
          ("names", ["-a", "--given-names", G, "--family-names", F,
                     "-f", "FLast", "-s", "on", "-v"]),
          ]

    # --- list-formats ----------------------------------------------------
    C.append(("listfmt", ["-l"]))
    C.append(("listfmt", ["--list-formats"]))

    # --- recognise -------------------------------------------------------
    base = ["--given-names", G, "--family-names", F]
    for target in ["b.key", "annakey", "bobkey", "anna", "key", "carlsmith",
                   "nnakey", "bs", "abc"]:
        C.append(("recognise", base + ["--recognise", target]))
    C.append(("recognise", base + ["-v", "--recognise", "key"]))
    C.append(("recognise", base + ["-v", "--recognise", "carlsmith"]))
    # options after -r are ignored (original exits mid-parse)
    C.append(("recognise", base + ["--recognise", "b.key", "-v"]))

    # --- verbose ---------------------------------------------------------
    C.append(("names", ["-v", "-s", "on", "--given-names", G,
                        "--family-names", F, "-f", "firstlast", "anna"]))

    # --- all bundled datasets -------------------------------------------
    valid_countries = [
        "argentina", "austria", "belgium", "canada", "denmark", "france",
        "germany", "hungary", "india", "italy", "luxembourg", "netherlands",
        "newzealand", "norway", "poland", "serbia", "slovenia", "spain",
        "sweden", "switzerland", "uk", "us",
    ]
    for c in valid_countries:
        C.append(("names", ["-a", "--country", c]))
    C.append(("names", ["-a", "--country", "france", "-f", "first.last"]))
    C.append(("names", ["-a", "--country", "US", "-s", "on", "-m", "2", "-f", "firstlast"]))
    C.append(("names", ["-s", "on", "-m", "3", "--country", "facebook",
                        "-f", "firstlast", "anna"]))
    for c in ["australia", "china", "ireland", "japan", "atlantis"]:
        C.append(("error", ["-a", "--country", c]))

    # --- extra input-file shapes ----------------------------------------
    for f in ["input_crlf.csv", "input_nonl.csv", "input_oddspace.txt",
              "input_blanklines.txt"]:
        C.append(("names", ["-i", str(FIX / f)]))
        C.append(("names", ["-i", str(FIX / f), "-f", "first.last"]))
        C.append(("names", ["-i", str(FIX / f), "-v", "-f", "firstlast"]))

    # --- explicit expansion-ordering cases ------------------------------
    C.append(("names", ["-s", "on", "--given-names", GD, "--family-names", F,
                        "-m", "2", "-f", "fml", "anna"]))
    C.append(("names", ["-s", "on", "--given-names", G, "--family-names", F,
                        "-m", "5", "-f", "fmlast", ""]))
    C.append(("names", ["-s", "on", "--given-names", G, "--family-names", F,
                        "-m", "7", "-f", "fml", ""]))
    C.append(("names", ["-s", "on", "--given-names", G, "--family-names", F,
                        "-f", "fml", ""]))
    C.append(("names", ["-s", "on", "--given-names", G, "--family-names", F,
                        "-m", "11", "-f", "firstmiddlelast", ""]))
    C.append(("names", ["-s", "on", "--given-names", G, "--family-names", F,
                        "-m", "6", "-F", "%givenname-%surname", ""]))
    C.append(("names", ["-s", "on", "--given-names", G, "--family-names", F,
                        "-m", "9", "-F", "%A-Z%a-z", ""]))

    # --- seeded random fuzz ---------------------------------------------
    import random
    rng = random.Random(0xA11A)
    pool_first = ["anna", "Bob", "O'Toole", "de la", "x", "Zed", "", "m", "J.R."]
    pool_last = ["key", "smith", "von G", "O'Neil", "", "z"]
    pool_mid = ["boom", "", "m", "q"]
    fmt_opts = ["%f.%l", "annakey", "ak", "%F-%L", "a%l", "%givenname",
                "%DD", "x%Dy", "anna.key", "annaboomkey", "%a-z", "Z%F"]
    for _ in range(90):
        args = []
        if rng.random() < 0.55:
            k = rng.randint(1, 3)
            args += ["-f", ",".join(rng.sample(STATIC_PLUGINS, k))]
        if rng.random() < 0.6:
            args += ["-s", "on", "--given-names", G, "--family-names", F]
            if rng.random() < 0.7:
                args += ["-m", str(rng.choice([1, 2, 3, 5, 10, 26]))]
        if rng.random() < 0.25:
            args += ["-C", rng.choice(["true", "false"])]
        if rng.random() < 0.2:
            args += ["-@", rng.choice(["@x", ".corp", "0"])]
        if rng.random() < 0.2:
            args += ["-F", rng.choice(fmt_opts)]
        n = rng.choice([1, 2, 3])
        names = [rng.choice(pool_first), rng.choice(pool_last), rng.choice(pool_mid)][:n]
        C.append(("names", args + names))

    # --- select-format list coercion (OptionParser `Array`) -------------
    C += [("names", ["-f", "", "anna", "key"]),
          ("names", ["-f", ",", "anna", "key"]),
          ("names", ["-f", "first,", "anna", "key"]),
          ("names", ["-f", "first,,", "anna", "key"]),
          ("names", ["-f", "first,last", "anna", "key"]),
          ("error", ["-f", ",first", "anna", "key"]),
          ("error", ["-f", "first,,last", "anna", "key"]),
          ("error", ["-f", " first", "anna", "key"]),
          ("error", ["-f", "first, last", "anna", "key"]),
          ]

    # --- strict (custom -F) expansion with token reassembly -------------
    C += [("names", ["-s", "on", "--given-names", G, "--family-names", F,
                     "-F", "%givenname", ""]),
          ("names", ["-s", "on", "--given-names", G, "--family-names", F,
                     "-F", "%surname", ""]),
          ("names", ["-s", "on", "--given-names", G, "--family-names", F,
                     "-F", "%Givenname%Surname", ""]),
          ("names", ["-v", "-s", "on", "--given-names", G, "--family-names", F,
                     "-m", "3", "-F", "%givenname-%surname", ""]),
          ("names", ["-v", "-F", "%f%l", "anna", "key"]),
          ("names", ["-a", "-F", "anna.key", "-s", "on", "-m", "2",
                     "--given-names", G, "--family-names", F]),
          ("names", ["-F", "x%f%l%D", "anna", "key"]),
          ]

    # --- wordlist last-line-without-newline (original raises) -----------
    C += [("names", ["-s", "on", "--given-names", GNONL, "--family-names", F,
                     "-f", "firstlast", "anna"]),
          ("error", ["-s", "on", "--given-names", G, "--family-names",
                     str(FIX / "family_nonl.txt"), "-f", "firstlast", "anna"]),
          ("error", ["-s", "on", "--given-names", GNONL, "--family-names", F,
                     "-f", "first", ""]),
          ("error", ["-s", "on", "--given-names", G, "--family-names",
                     str(FIX / "family_nonl.txt"), "-f", "last", ""]),
          ("names", ["-s", "on", "--given-names", G, "--family-names", F,
                     "-f", "last", "anna"]),
          ]

    # --- undetectable delimiter -----------------------------------------
    C.append(("error", ["-i", str(FIX / "input_nodelim.txt")]))

    # --- long-option abbreviation & inline forms ------------------------
    C += [("names", ["--gi", G, "anna"]),
          ("names", ["--suf", ".x", "anna", "key"]),
          ("names", ["--case-", "true", "-f", "FirstLast", "Anna", "Key"]),
          ("listfmt", ["--list", "anna"]),
          ("names", ["--verb", "anna", "key"]),
          ("usage", ["--hel"]),
          ("names", ["--max", "2", "anna"]),
          ("names", ["--countr", "france", "-a"]),
          ("error", ["--f", "first", "anna", "key"]),
          ("error", ["--auto=false"]),
          ("error", ["--no-auto", "--country", "france", "anna", "key"]),
          ("error", ["--formatsss", "x"]),
          ]

    # --- multiple -F / custom plugin registration -----------------------
    C += [("names", ["-F", "annakey", "-F", "%f-%l", "anna", "key"]),
          ("names", ["-F", "%f.%l", "-f", "custom", "anna", "key"]),
          ("error", ["-f", "custom", "-F", "%f.%l", "anna", "key"]),
          ]

    # --- Ruby `Integer()` numeric forms for -m --------------------------
    C += [("names", ["-s", "on", "--given-names", FBG, "--family-names", FBF,
                     "-m", "0x10", "-f", "firstlast", ""]),
          ("names", ["-s", "on", "--given-names", FBG, "--family-names", FBF,
                     "-m", "010", "-f", "firstlast", ""]),
          ("names", ["-s", "on", "--given-names", FBG, "--family-names", FBF,
                     "-m", "1_0", "-f", "firstlast", ""]),
          ]

    # --- case folding and the streamed --auto path -----------------------
    # Uppercase templates run through upcase_it; with -C true the original
    # downcases the candidate before printing. The resolved-lower fast path
    # must produce the same bytes, including when fields are missing (the
    # sentinel) and when a suffix is appended.
    for fmt in ["FLast", "FL", "FirstLast", "First.Last", "Last"]:
        C.append(("names", ["-a", "--given-names", G, "--family-names", F,
                            "-f", fmt, "-s", "on"]))
    C += [("names", ["Anna", "Key"]),
          ("names", ["-C", "false", "Anna", "Key"]),
          ("names", ["-C", "false", "-f", "FLast", "Anna", "Key"]),
          ("names", ["-C", "false", "-f", "firstlast", "Anna", "Key"]),
          ("names", ["-C", "false", "-f", "FirstLast", "Anna", "Key"]),
          ("names", ["-C", "false", "-a", "--given-names", G, "--family-names", F]),
          ("names", ["-C", "false", "-a", "--given-names", G, "--family-names", F,
                     "-f", "FLast", "-s", "on"]),
          ("names", ["-C", "false", "-@", "@EXAMPLE.COM", "-f", "FLast", "Anna", "Key"]),
          ("names", ["-@", "@EXAMPLE.COM", "-f", "FLast", "Anna", "Key"]),
          ("names", ["-@", "@example.com", "-f", "First.Last", "Anna", "Key"]),
          # Non-ASCII: the composed upcase->downcase is not plain downcase
          # (ß -> SS -> ss), so both paths must match Ruby exactly.
          ("names", ["-f", "FLast", "ß", "Key"]),
          ("names", ["-f", "FirstLast", "ß", "Key"]),
          ("names", ["-f", "First.Last", "Élodie", "Key"]),
          ("names", ["-C", "false", "-f", "FLast", "ß", "Key"]),
          ("names", ["-C", "false", "-f", "First.Last", "Élodie", "Key"]),
          ("names", ["-f", "firstlast[8]", "ß", "Key"]),
          ]

    # --- usage / error paths --------------------------------------------
    C += [("usage", []),
          ("usage", ["a", "b", "c", "d"]),
          ("usage", ["--help"]),
          ("error", ["-i", "/nonexistent/nope.txt"]),
          ("error", ["-f", "nosuch", "anna", "key"]),
          ("error", ["-s", "bogus", "anna", "key"]),
          ("error", ["-C", "maybe", "anna", "key"]),
          ("error", ["-m", "0", "anna", "key"]),
          ("error", ["-m", "-3", "anna", "key"]),
          ("error", ["-m", "abc", "anna", "key"]),
          ("error", ["--badopt"]),
          ("error", ["-Z"]),
          ("error", ["-C"]),
          ("error", ["-a", "--country", "china"]),
          ("error", ["-a", "--country", "atlantis"]),
          ("error", ["--bare-delim", "x"]),
          ]
    return C


# ---------------------------------------------------------------------------
# Runner
# ---------------------------------------------------------------------------

class Missing:
    pass


def classify(case_kind, rust, oracle, oracle_name):
    """Return (ok, detail)."""
    r_rc, r_out, r_err = rust
    o_rc, o_out, o_err = oracle
    if case_kind == "error":
        if r_rc == 0:
            return False, f"expected non-zero exit, got 0 (oracle {oracle_name} rc={o_rc})"
        return True, "ok"
    if case_kind == "usage":
        if r_rc != 0:
            return False, f"expected exit 0, got {r_rc}"
        if b"Usage:" not in r_out:
            return False, "usage text missing"
        return True, "ok"
    if o_rc != 0:
        return False, f"oracle {oracle_name} failed with rc={o_rc}: {o_err[-200:]!r}"
    if r_rc != 0:
        return False, f"rust exited {r_rc}: {r_err[-200:]!r}"
    if case_kind == "listfmt":
        if r_out != o_out:
            return False, _diff(r_out, o_out)
        return True, "ok"
    if case_kind == "recognise":
        rs, os_ = normalise_recognise(r_out), normalise_recognise(o_out)
    else:
        rs, os_ = lines_set(r_out), lines_set(o_out)
    if rs != os_:
        return False, _diff_lines(rs, os_)
    return True, "ok"


def _diff(a: bytes, b: bytes) -> str:
    import difflib
    al = a.decode("utf-8", "replace").splitlines()
    bl = b.decode("utf-8", "replace").splitlines()
    d = list(difflib.unified_diff(bl, al, "oracle", "rust", lineterm=""))
    return "\n".join(d[:40])


def _diff_lines(rs: set, os_: set) -> str:
    only_rust = sorted(rs - os_)[:15]
    only_oracle = sorted(os_ - rs)[:15]
    return (f"missing from rust ({len(os_ - rs)}): {only_oracle}\n"
            f"extra in rust ({len(rs - os_)}): {only_rust}")


def only_only(a, b):
    return sorted(a - b)[:15]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("-k", "--filter", default=None)
    ap.add_argument("--selftest", action="store_true")
    ap.add_argument("--verbose", action="store_true")
    args = ap.parse_args()

    if not RUBY_SCRIPT.exists():
        print(f"FATAL: ruby oracle not found at {RUBY_SCRIPT}", file=sys.stderr)
        return 2
    if not RUST_BIN.exists():
        print(f"FATAL: rust binary not found at {RUST_BIN}\n"
              f"       build with: cargo build --release", file=sys.stderr)
        return 2

    if args.selftest:
        return selftest()

    cases = build_cases()
    if args.filter:
        cases = [c for c in cases if args.filter in " ".join(c[1])]

    npass = nfail = nskip = 0
    failures = []
    for idx, (kind, argv) in enumerate(cases):
        stock = run_ruby(argv, compat=False)
        compat = run_ruby(argv, compat=True)
        rust = run_rust(argv)

        if kind == "error":
            oracle, oname = stock, "ruby"
        elif stock[0] == 0:
            oracle, oname = stock, "ruby"
            # The shim must agree wherever stock Ruby succeeds.
            if compat[0] == 0 and kind in ("names",) and b"\x1b[" not in stock[1]:
                a = lines_set(stock[1])
                b = lines_set(compat[1])
                if a != b:
                    failures.append((argv, "SHIM MISMATCH", _diff_lines(b, a)))
                    nfail += 1
                    continue
        elif compat[0] == 0:
            oracle, oname = compat, "ruby+compat"
        else:
            if rust[0] != 0:
                npass += 1
                continue
            failures.append((argv, "both oracles failed but rust succeeded",
                             f"stock={stock[2][-150:]!r} compat={compat[2][-150:]!r}"))
            nfail += 1
            continue

        ok, detail = classify(kind, rust, oracle, oname)
        if ok:
            npass += 1
            if args.verbose:
                print(f"PASS [{kind}] {' '.join(argv)}")
        else:
            nfail += 1
            failures.append((argv, f"[{kind}] oracle={oname}", detail))

    print(f"\n{'=' * 70}")
    print(f"parity: {npass} passed, {nfail} failed ({len(cases)} cases)")
    if failures:
        print(f"{'=' * 70}\nFAILURES:\n")
        for argv, why, detail in failures:
            print(f"- {' '.join(argv)}")
            print(f"    {why}")
            for ln in detail.splitlines()[:25]:
                print(f"      {ln}")
            print()
    return 1 if failures else 0


# ---------------------------------------------------------------------------
# Self-test: prove the harness can actually detect divergence
# ---------------------------------------------------------------------------

def selftest():
    print("selftest: synthetic comparisons")
    ok = True

    # exact match must pass
    if not classify("names", (0, b"a\nb\nc\n", b""), (0, b"c\nb\na\n", b""), "x")[0]:
        print("  FAIL: identical sets reported as mismatch")
        ok = False
    # differing set must fail
    if classify("names", (0, b"a\nb\n", b""), (0, b"a\nz\n", b""), "x")[0]:
        print("  FAIL: differing sets reported as match")
        ok = False
    # ordering-only difference must pass (set semantics)
    if not classify("names", (0, b"z\na\n", b""), (0, b"a\nz\n", b""), "x")[0]:
        print("  FAIL: ordering-only diff reported as mismatch")
        ok = False
    # error kind must require non-zero
    if classify("error", (0, b"", b""), (1, b"", b""), "x")[0]:
        print("  FAIL: rc=0 accepted for an error case")
        ok = False
    if not classify("error", (1, b"", b""), (1, b"", b""), "x")[0]:
        print("  FAIL: rc=1 rejected for an error case")
        ok = False

    # end-to-end: the Redis self-consistency (stock vs compat) plumbing.
    print("selftest: real oracle self-consistency")
    for argv in [["anna", "key"], ["-f", "firstlast", "anna"],
                 ["-i", str(FIX / "input_full.csv")]]:
        s = run_ruby(argv, False)
        c = run_ruby(argv, True)
        if s[0] != 0 or c[0] != 0 or lines_set(s[1]) != lines_set(c[1]):
            print(f"  FAIL: stock/compat disagree on {argv}")
            ok = False

    # Deliberately broken candidate must be caught.
    print("selftest: broken-candidate detection")
    bad = subprocess.run(
        ["bash", "-c",
         f'"{RUST_BIN}" "$@" 2>/dev/null; echo ZZZ-BOGUS-LINE',
         "_", "anna", "key"],
        cwd=str(REPO), stdout=subprocess.PIPE)
    oracle = run_rust(["anna", "key"])
    ok2, _ = classify("names", (0, bad.stdout, b""), oracle, "x")
    if ok2:
        print("  FAIL: injected bogus line was not detected")
        ok = False

    print("selftest:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
