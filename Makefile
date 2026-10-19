# Reproducible builds, parity tests and benchmarks for the Rust rewrite.
#
#   make          static binary + datasets in dist/
#   make test     behavioural parity gate against the Ruby original
#   make check    fmt + clippy + parity self-test
#   make bench    Rust vs Ruby benchmark table
#   make clean

TARGET := x86_64-unknown-linux-musl
BUILD  := target/$(TARGET)/release/username-anarchy-rs
DIST   := dist/username-anarchy

.PHONY: all build dist test selftest check bench clean

all: dist

build:
	cargo build --release

check:
	cargo fmt --all -- --check
	cargo clippy --all-targets --release -- -D warnings
	$(MAKE) selftest

dist: build
	@mkdir -p dist
	@strip --strip-all -o $(DIST) $(BUILD)
	@rm -rf dist/names
	@cp -r username-anarchy/names dist/names
	@echo "built $(DIST)"
	@file $(DIST)

test:
	python3 tests/parity/run_parity.py

selftest:
	python3 tests/parity/run_parity.py --selftest

bench:
	python3 bench/run_bench.py

clean:
	cargo clean
	rm -rf dist
