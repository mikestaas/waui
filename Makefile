WASM_TARGET ?= wasm32-unknown-unknown
COMPONENTS  := demo-counter demo-stopwatch

.PHONY: components run-cli run-stop-cli run-iced run-stop-iced test

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

# Windowed hosts (iced).
run-iced: components
	cargo run -p host-iced -- target/components/demo-counter.component.wasm

run-stop-iced: components
	cargo run -p host-iced -- target/components/demo-stopwatch.component.wasm

test:
	cargo test
