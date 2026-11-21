# Command interface. Stubs fail until implemented, and run inside WSL2 or Ubuntu.
# EmberKV has no external services, so the lab and backup commands of the other projects do not apply.
.PHONY: help dev-local stop-dev test crash-test bench lint

help:
	@echo "  dev-local      Run the server on localhost with a disposable data directory"
	@echo "  stop-dev       Stop the local server cleanly, keeping its data"
	@echo "  test           Format check, lint and all unit and integration tests"
	@echo "  crash-test     Kill the server repeatedly during writes and check every confirmed write survives"
	@echo "  bench          Run the benchmark harness and save results to docs/benchmarks/"
	@echo "  lint           Run cargo fmt --check and cargo clippy"

# Run the server on localhost with a disposable data directory
dev-local:
	@echo "dev-local is not implemented yet" >&2; exit 1

# Stop the local server cleanly, keeping its data
stop-dev:
	@echo "stop-dev is not implemented yet" >&2; exit 1

# Format check, lint and all unit and integration tests
test: lint
	cargo test --workspace --locked

# Kill the server repeatedly during writes and check every confirmed write survives
crash-test:
	@echo "crash-test is not implemented yet" >&2; exit 1

# Run the benchmark harness and save results to docs/benchmarks/
bench:
	@echo "bench is not implemented yet" >&2; exit 1

# Run cargo fmt --check and cargo clippy
lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
