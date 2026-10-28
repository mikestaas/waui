WASM_TARGET   ?= wasm32-unknown-unknown
COMPONENTS    := demo-counter demo-stopwatch
AS_COMPONENTS := demo-counter demo-stopwatch

.PHONY: components components-as run-cli run-stop-cli run-tui run-stop-tui run-iced run-stop-iced \
         run-cli-as run-stop-cli-as run-tui-as run-stop-tui-as run-iced-as run-stop-iced-as \
         web web-as web-test web-test-as fmt fmt-check hooks test

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

# Build the AssemblyScript demos and lift them.
#
# No wit-bindgen exists for AssemblyScript, so the lift is typed explicitly:
# the WIT in `wit/` is embedded into the module (`component embed`) and the
# result lifted (`component new`).
#
#   asc (incremental runtime — the ui/tree object tree needs a heap)
#   → patch-env-abort (the runtime's one import becomes an in-module
#     unreachable, so the module stays import-free)
#   → rename exports (world names with hyphens are not AS identifiers)
#   → component embed wit/ → component new
#
# Needs node + `npm install` (asc) and wabt on the PATH (the patch step).
components-as:
	@command -v node >/dev/null 2>&1 || { echo "components-as needs node/npm — run: npm install"; exit 1; }
	@test -x node_modules/.bin/asc || { echo "assemblyscript not installed — run: npm install"; exit 1; }
	@mkdir -p target/assembly target/components
	@for c in $(AS_COMPONENTS); do \
		echo "==> building $$c (assemblyscript)"; \
		npx asc assembly/$$c/index.ts --outFile target/assembly/$$c.raw.wasm \
			-O --noAssert --runtime incremental --memoryBase 65536 --initialMemory 2 || exit 1; \
		node assembly/patch-env-abort.mjs target/assembly/$$c.raw.wasm target/assembly/$$c.core.wasm || exit 1; \
		node assembly/rename-exports.mjs target/assembly/$$c.core.wasm target/assembly/$$c-as.wasm \
			onclick=on-click ontick=on-tick || exit 1; \
		wasm-tools component embed wit/ target/assembly/$$c-as.wasm \
			--output target/assembly/$$c-as.embedded.wasm || exit 1; \
		wasm-tools component new target/assembly/$$c-as.embedded.wasm \
			--output target/components/$$c-as.component.wasm || exit 1; \
	done
	@echo "wrote target/components/*-as.component.wasm (core modules in target/assembly/)"

# The same hosts, pointed at the AssemblyScript components.
run-cli-as: components-as
	cargo run -p host-cli -- target/components/demo-counter-as.component.wasm

run-stop-cli-as: components-as
	cargo run -p host-cli -- target/components/demo-stopwatch-as.component.wasm

run-tui-as: components-as
	cargo run -p host-tui -- target/components/demo-counter-as.component.wasm

run-stop-tui-as: components-as
	cargo run -p host-tui -- target/components/demo-stopwatch-as.component.wasm

run-iced-as: components-as
	cargo run -p host-iced -- target/components/demo-counter-as.component.wasm

run-stop-iced-as: components-as
	cargo run -p host-iced -- target/components/demo-stopwatch-as.component.wasm

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

# The same self-contained pages for the AssemblyScript components. The web
# host runs the pre-lift core module, so it consumes the renamed modules
# straight from target/assembly/ — glue.js and app.js are untouched.
web-as: components-as
	@mkdir -p target/web
	@for c in $(AS_COMPONENTS); do \
		node web/build.mjs target/assembly/$$c-as.wasm target/web/$$c-as.html || exit 1; \
	done
	@echo "wrote target/web/*-as.html — open them directly (no server needed)"

# Headless check of the web host's ABI layer (Node).
web-test: components
	node web/selftest.mjs

# The same selftest against the AssemblyScript core modules.
web-test-as: components-as
	node web/selftest.mjs target/assembly/demo-counter-as.wasm target/assembly/demo-stopwatch-as.wasm

# Format all sources: the Rust workspace with rustfmt, and the
# AssemblyScript (assembly/) + web (web/) trees with prettier. The
# prettier scope is .prettierignore; the assemblyscript-prettier
# plugin parses the `@external` imports that plain prettier rejects.
fmt:
	@cargo fmt
	@test -x node_modules/.bin/prettier || { echo "prettier not installed — run: npm install"; exit 1; }
	npx prettier --write .

# `fmt` without writing: fails when any file would change. The
# pre-commit hook (`.githooks`, `make hooks`) gates on this.
fmt-check:
	@cargo fmt --check
	@test -x node_modules/.bin/prettier || { echo "prettier not installed — run: npm install"; exit 1; }
	npx prettier --check .

# Point git at the shared hooks in .githooks/ (idempotent).
hooks:
	git config core.hooksPath .githooks
	@echo "git hooks path: $$(git config core.hooksPath)"

test:
	cargo test
