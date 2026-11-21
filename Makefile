# Command interface. Stubs fail until implemented, and run inside WSL2 or Ubuntu.
# EmberKV has no external services, so the lab and backup commands of the other projects do not apply.
# Port for dev-local; override with: make dev-local PORT=7400
PORT ?= 7379

.PHONY: help dev-local stop-dev test crash-test bench lint

help:
	@echo "  dev-local      Start the server on 127.0.0.1:$(PORT) in the background, data in data/dev"
	@echo "  stop-dev       Stop the local server cleanly, keeping its data"
	@echo "  test           Format check, lint and all unit and integration tests"
	@echo "  crash-test     Kill the server repeatedly during writes and check every confirmed write survives"
	@echo "  bench          Run the benchmark harness and save results to docs/benchmarks/"
	@echo "  lint           Run cargo fmt --check and cargo clippy"

# Start the server on 127.0.0.1:7379 in the background. Data in data/dev is kept between runs;
# delete that folder to start empty. Its output goes to data/dev.log.
dev-local:
	@if [ -f data/dev.pid ] && kill -0 $$(cat data/dev.pid) 2>/dev/null; then \
		echo "already running (process $$(cat data/dev.pid)); stop it with: make stop-dev"; exit 1; fi
	@mkdir -p data
	cargo build -q -p emberkv-server
	@./target/debug/emberkv --dir data/dev --port $(PORT) 2> data/dev.log & echo $$! > data/dev.pid
	@sleep 0.5; cat data/dev.log
	@if ! kill -0 $$(cat data/dev.pid) 2>/dev/null; then \
		rm -f data/dev.pid; echo "the server did not start (see the message above)"; exit 1; fi
	@echo "Running in the background. Talk to it with: redis-cli -p $(PORT)   Stop it with: make stop-dev"

# Stop the local server cleanly, keeping its data
stop-dev:
	@if [ -f data/dev.pid ] && kill -TERM $$(cat data/dev.pid) 2>/dev/null; then \
		while kill -0 $$(cat data/dev.pid) 2>/dev/null; do sleep 0.1; done; \
		rm data/dev.pid; tail -n 2 data/dev.log; \
	else rm -f data/dev.pid; echo "not running"; fi

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
