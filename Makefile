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
PGO_DIR := target/pgo-data
PGO_BIN := target/pgo/$(TARGET)/release/username-anarchy-rs

.PHONY: all build native dist pgo test selftest check bench clean

all: dist

build:
	cargo build --release

# Optional local build tuned for this exact CPU. Not used for `dist`: the
# shipped binary stays baseline x86-64 so it runs everywhere.
native:
	CARGO_TARGET_DIR=target/native RUSTFLAGS="-C target-cpu=native" cargo build --release
	@echo "built target/native/$(TARGET)/release/username-anarchy-rs"

# Optional profile-guided build. The training run uses the 100k-row input plus
# the 1M-substitution `-m 5` expansion, so both the row and expansion paths get
# branch weights. Not used for `dist`: PGO data is LLVM-version-specific, so the
# release artifact stays reproducible from a plain `make`.
pgo:
	python3 bench/run_bench.py --prepare-data
	rm -rf $(PGO_DIR) target/pgo
	mkdir -p $(PGO_DIR)
	CARGO_TARGET_DIR=target/pgo RUSTFLAGS="-Cprofile-generate=$(PWD)/$(PGO_DIR)" cargo build --release
	$(PGO_BIN) -i bench/data/names-100k.txt > /dev/null
	$(PGO_BIN) -s on --given-names username-anarchy/names/facebook/firstnames-top1000.txt \
		--family-names username-anarchy/names/facebook/lastnames-top1000.txt \
		-m 5 -f firstlast '' > /dev/null
	llvm-profdata merge -o $(PGO_DIR)/merged.profdata $(PGO_DIR)/*.profraw
	CARGO_TARGET_DIR=target/pgo \
		RUSTFLAGS="-Cprofile-use=$(PWD)/$(PGO_DIR)/merged.profdata -Cllvm-args=-pgo-warn-missing-function" \
		cargo build --release
	@echo "built $(PGO_BIN)"

check:
	cargo fmt --all -- --check
	cargo clippy --all-targets --release -- -D warnings
	cargo test
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
