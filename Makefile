TARGET := x86_64-unknown-linux-musl
BUILD := target/$(TARGET)/release/anarchy-rs
DIST := dist/anarchy-rs

.PHONY: all build dist test check clean

all: dist

build:
	cargo build --release --locked --target $(TARGET)

dist: build
	rm -rf dist
	@mkdir -p dist
	strip --strip-all -o $(DIST) $(BUILD)
	cp -r names dist/names
	cp README.md LICENSE dist/
	cp -r docs dist/docs
	@echo "built $(DIST)"

test: build
	cargo test --locked --target $(TARGET)
	python3 -m unittest discover -s tests/cli -p 'test_*.py'
	python3 tests/cli/run.py --binary $(BUILD)

check:
	cargo fmt --all -- --check
	cargo clippy --all-targets --release --locked -- -D warnings
	RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items --locked
	$(MAKE) test

clean:
	cargo clean
	rm -rf dist
