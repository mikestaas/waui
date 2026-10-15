WASM_TARGET ?= wasm32-unknown-unknown
COMPONENTS  := demo-counter demo-stopwatch

.PHONY: components run-cli run-stop-cli run-tui run-stop-tui run-iced run-stop-iced web web-test test

# Build the demo components and lift them into the component model.
components:
	@mkdir -p target/components
	@for c in $(COMPONENTS); do \
		echo "==> building $$c (wasm32)"; \
		cargo build -p $$c --release --target $(WASM_TARGET) || exit 1; \
		lib=$$(printf '%s' "$$c" | tr '-' '_'); \
		wasm-tools component new target/$(WASM_TARGET)/release/$$lib.wasm \
			--output target/components/$$c.component.wasm || exit 1; \
	done
	@echo "wrote target/components/*.component.wasm"

# Terminal hosts: one component, two different renderers.
run-cli: components
	cargo run -p host-cli -- target/components/demo-counter.component.wasm

run-stop-cli: components
	cargo run -p host-cli -- target/components/demo-stopwatch.component.wasm

run-tui: components
	cargo run -p host-tui -- target/components/demo-counter.component.wasm

run-stop-tui: components
	cargo run -p host-tui -- target/components/demo-stopwatch.component.wasm

# Windowed hosts (iced).
run-iced: components
	cargo run -p host-iced -- target/components/demo-counter.component.wasm

run-stop-iced: components
	cargo run -p host-iced -- target/components/demo-stopwatch.component.wasm

# Web host: one self-contained HTML page per component (the wasm module
# inlined as base64). The page runs straight from disk — no server, no npm.
web: components
	@mkdir -p target/web
	@for c in $(COMPONENTS); do \
		lib=$$(printf '%s' "$$c" | tr '-' '_'); \
		node web/build.mjs target/$(WASM_TARGET)/release/$$lib.wasm target/web/$$c.html || exit 1; \
	done
	@echo "wrote target/web/*.html — open them directly (no server needed)"

# Headless check of the web host's ABI layer (Node).
web-test: components
	node web/selftest.mjs

test:
	cargo test
