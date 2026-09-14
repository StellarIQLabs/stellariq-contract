.PHONY: build test fmt clippy clean

PACKAGE := stellariq-router

build:
	stellar contract build --package $(PACKAGE)

test:
	cargo test --package $(PACKAGE)

fmt:
	cargo fmt --package $(PACKAGE)

clippy:
	cargo clippy --package $(PACKAGE) --all-targets -- -D warnings

clean:
	cargo clean --package $(PACKAGE)
	rm -rf target/wasm32v1-none/release/$(PACKAGE)*.wasm
