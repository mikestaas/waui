WASM_TARGET   ?= wasm32-unknown-unknown
COMPONENTS    := demo-counter demo-stopwatch demo-todo
AS_COMPONENTS := demo-counter demo-stopwatch demo-todo
WAT_COMPONENTS := demo-counter demo-stopwatch demo-todo

# The desktop's nine icons, in their demos' source trees (the registry's
# canonical paths): the site and the web pages copy them to their own flat
# icons/ set.
ICONS := crates/demo-counter/counter-rust.svg crates/demo-stopwatch/stopwatch-rust.svg \
	crates/demo-todo/todo-rust.svg \
	assembly/demo-counter/counter-as.svg assembly/demo-stopwatch/stopwatch-as.svg \
	assembly/demo-todo/todo-as.svg \
	wat/counter-wat.svg wat/stopwatch-wat.svg wat/todo-wat.svg

.PHONY: components components-as components-wat run-cli run-stop-cli run-todo-cli run-tui run-stop-tui run-todo-tui run-desktop-tui \
         run-iced run-stop-iced run-todo-iced run-desktop-iced run-cli-as run-stop-cli-as run-todo-cli-as \
         run-tui-as run-stop-tui-as run-todo-tui-as run-desktop-tui-as run-iced-as run-stop-iced-as run-todo-iced-as run-desktop-iced-as \
         run-cli-wat run-stop-cli-wat run-todo-cli-wat run-tui-wat run-stop-tui-wat run-todo-tui-wat run-desktop-tui-wat \
         run-iced-wat run-stop-iced-wat run-todo-iced-wat run-desktop-iced-wat \
         web web-as web-wat web-test web-test-as web-test-wat \
         host-tui-wasm host-tui-wasm-test host-iced-wasm host-iced-wasm-test \
         site ui-test e2e goldens screenshots fmt fmt-check hooks test

# Build the demo components and lift them into the component model. The
# desktop (demo-desktop) is the multi-component host's component: it is
# built explicitly, not in the demo loop, because the demo loop's variables
# name the standalone demos only.
components:
	@mkdir -p target/components
	@for c in $(COMPONENTS); do \
		echo "==> building $$c (wasm32)"; \
		cargo build -p $$c --release --target $(WASM_TARGET) || exit 1; \
		lib=$$(printf '%s' "$$c" | tr '-' '_'); \
		wasm-tools component new target/$(WASM_TARGET)/release/$$lib.wasm \
			--output target/components/$$c.component.wasm || exit 1; \
	done
	@echo "==> building demo-desktop (wasm32)"
	@cargo build -p demo-desktop --release --target $(WASM_TARGET) || exit 1
	@wasm-tools component new target/$(WASM_TARGET)/release/demo_desktop.wasm \
		--output target/components/demo-desktop.component.wasm || exit 1
	@echo "wrote target/components/*.component.wasm"

# Build the AssemblyScript demos and lift them. The pipeline (jsx → asc →
# patch-env-abort → rename-exports → component embed/new) is documented in
# assembly/README.md. Needs node + `npm install` (asc, babel) and wabt on the
# PATH (the patch step).
components-as:
	@command -v node >/dev/null 2>&1 || { echo "components-as needs node/npm — run: npm install"; exit 1; }
	@test -x node_modules/.bin/asc || { echo "assemblyscript not installed — run: npm install"; exit 1; }
	@test -d node_modules/@babel/core || { echo "babel not installed — run: npm install"; exit 1; }
	@mkdir -p target/assembly target/components
	@for c in $(AS_COMPONENTS); do \
		echo "==> building $$c (assemblyscript)"; \
		node assembly/jsx.mjs assembly/$$c/index.tsx assembly/$$c/index.gen.ts || exit 1; \
		npx asc assembly/$$c/index.gen.ts --outFile target/assembly/$$c.raw.wasm \
			-O --noAssert --runtime incremental --memoryBase 65536 --initialMemory 2 || exit 1; \
		node assembly/patch-env-abort.mjs target/assembly/$$c.raw.wasm target/assembly/$$c.core.wasm || exit 1; \
		node assembly/rename-exports.mjs target/assembly/$$c.core.wasm target/assembly/$$c-as.wasm \
			onclick=on-click ontext=on-text ontick=on-tick \
			ondoubleclick=on-double-click onwindowclose=on-window-close \
			onpointerdown=on-pointer-down onpointermove=on-pointer-move \
			onpointerup=on-pointer-up onkey=on-key || exit 1; \
		wasm-tools component embed wit/ target/assembly/$$c-as.wasm \
			--world app --output target/assembly/$$c-as.embedded.wasm || exit 1; \
		wasm-tools component new target/assembly/$$c-as.embedded.wasm \
			--output target/components/$$c-as.component.wasm || exit 1; \
	done
	# The desktop, the same pipeline (see the components target).
	@echo "==> building demo-desktop (assemblyscript)"
	@node assembly/jsx.mjs assembly/demo-desktop/index.tsx assembly/demo-desktop/index.gen.ts || exit 1
	@npx asc assembly/demo-desktop/index.gen.ts --outFile target/assembly/demo-desktop.raw.wasm \
		-O --noAssert --runtime incremental --memoryBase 65536 --initialMemory 2 || exit 1
	@node assembly/patch-env-abort.mjs target/assembly/demo-desktop.raw.wasm target/assembly/demo-desktop.core.wasm || exit 1
	@node assembly/rename-exports.mjs target/assembly/demo-desktop.core.wasm target/assembly/demo-desktop-as.wasm \
		onclick=on-click ontext=on-text ontick=on-tick \
		ondoubleclick=on-double-click onwindowclose=on-window-close \
		onpointerdown=on-pointer-down onpointermove=on-pointer-move \
		onpointerup=on-pointer-up onkey=on-key || exit 1
	@wasm-tools component embed wit/ target/assembly/demo-desktop-as.wasm \
		--world app --output target/assembly/demo-desktop-as.embedded.wasm || exit 1
	@wasm-tools component new target/assembly/demo-desktop-as.embedded.wasm \
		--output target/components/demo-desktop-as.component.wasm || exit 1
	@echo "wrote target/components/*-as.component.wasm (core modules in target/assembly/)"

# Build the hand-rolled WAT demos and lift them. One self-contained .wat
# file per demo (wat/): the WASM GC holds the state, and the canonical ABI
# is written by hand. The pipeline is wasm-tools parse (WAT -> core wasm),
# then the same component embed (the WIT) / component new lift as the AS
# demos. Needs wasm-tools on the PATH.
components-wat:
	@command -v wasm-tools >/dev/null 2>&1 || { echo "components-wat needs wasm-tools on the PATH"; exit 1; }
	@mkdir -p target/wat target/components
	@for c in $(WAT_COMPONENTS); do \
		echo "==> building $$c (wat)"; \
		wasm-tools parse wat/$$c.wat --output target/wat/$$c.wasm || exit 1; \
		wasm-tools component embed wit/ target/wat/$$c.wasm \
			--world app --output target/wat/$$c-wat.embedded.wasm || exit 1; \
		wasm-tools component new target/wat/$$c-wat.embedded.wasm \
			--output target/components/$$c-wat.component.wasm || exit 1; \
	done
	# The desktop, the same pipeline (see the components target).
	@echo "==> building demo-desktop (wat)"
	@wasm-tools parse wat/demo-desktop.wat --output target/wat/demo-desktop.wasm || exit 1
	@wasm-tools component embed wit/ target/wat/demo-desktop.wasm \
		--world app --output target/wat/demo-desktop-wat.embedded.wasm || exit 1
	@wasm-tools component new target/wat/demo-desktop-wat.embedded.wasm \
		--output target/components/demo-desktop-wat.component.wasm || exit 1
	@echo "wrote target/components/*-wat.component.wasm (core modules in target/wat/)"

# The same hosts, pointed at the WAT components.
run-cli-wat: components-wat
	cargo run -p host-cli -- target/components/demo-counter-wat.component.wasm

run-stop-cli-wat: components-wat
	cargo run -p host-cli -- target/components/demo-stopwatch-wat.component.wasm

run-todo-cli-wat: components-wat
	cargo run -p host-cli -- target/components/demo-todo-wat.component.wasm

run-tui-wat: components-wat
	cargo run -p host-tui -- target/components/demo-counter-wat.component.wasm

run-stop-tui-wat: components-wat
	cargo run -p host-tui -- target/components/demo-stopwatch-wat.component.wasm

run-todo-tui-wat: components-wat
	cargo run -p host-tui -- target/components/demo-todo-wat.component.wasm

# The desktop targets read the registry manifest (registry.json): the nine
# entries (the three demos, three languages), so all producers build, and
# the host runs the producer's desktop app (the windowing component).
run-desktop-tui-wat: components components-as components-wat
	cargo run -p host-tui -- --registry registry.json target/components/demo-desktop-wat.component.wasm

run-iced-wat: components-wat
	cargo run -p host-iced -- target/components/demo-counter-wat.component.wasm

run-stop-iced-wat: components-wat
	cargo run -p host-iced -- target/components/demo-stopwatch-wat.component.wasm

run-todo-iced-wat: components-wat
	cargo run -p host-iced -- target/components/demo-todo-wat.component.wasm

run-desktop-iced-wat: components components-as components-wat
	cargo run -p host-iced -- --registry registry.json target/components/demo-desktop-wat.component.wasm

# The same hosts, pointed at the AssemblyScript components.
run-cli-as: components-as
	cargo run -p host-cli -- target/components/demo-counter-as.component.wasm

run-stop-cli-as: components-as
	cargo run -p host-cli -- target/components/demo-stopwatch-as.component.wasm

run-todo-cli-as: components-as
	cargo run -p host-cli -- target/components/demo-todo-as.component.wasm

run-tui-as: components-as
	cargo run -p host-tui -- target/components/demo-counter-as.component.wasm

run-stop-tui-as: components-as
	cargo run -p host-tui -- target/components/demo-stopwatch-as.component.wasm

run-todo-tui-as: components-as
	cargo run -p host-tui -- target/components/demo-todo-as.component.wasm

run-desktop-tui-as: components components-as components-wat
	cargo run -p host-tui -- --registry registry.json target/components/demo-desktop-as.component.wasm

run-iced-as: components-as
	cargo run -p host-iced -- target/components/demo-counter-as.component.wasm

run-stop-iced-as: components-as
	cargo run -p host-iced -- target/components/demo-stopwatch-as.component.wasm

run-todo-iced-as: components-as
	cargo run -p host-iced -- target/components/demo-todo-as.component.wasm

run-desktop-iced-as: components components-as components-wat
	cargo run -p host-iced -- --registry registry.json target/components/demo-desktop-as.component.wasm

# Terminal hosts: one component, two different renderers.
run-cli: components
	cargo run -p host-cli -- target/components/demo-counter.component.wasm

run-stop-cli: components
	cargo run -p host-cli -- target/components/demo-stopwatch.component.wasm

run-todo-cli: components
	cargo run -p host-cli -- target/components/demo-todo.component.wasm

run-tui: components
	cargo run -p host-tui -- target/components/demo-counter.component.wasm

run-stop-tui: components
	cargo run -p host-tui -- target/components/demo-stopwatch.component.wasm

run-todo-tui: components
	cargo run -p host-tui -- target/components/demo-todo.component.wasm

run-desktop-tui: components components-as components-wat
	cargo run -p host-tui -- --registry registry.json target/components/demo-desktop.component.wasm

# Windowed hosts (iced).
run-iced: components
	cargo run -p host-iced -- target/components/demo-counter.component.wasm

run-stop-iced: components
	cargo run -p host-iced -- target/components/demo-stopwatch.component.wasm

run-todo-iced: components
	cargo run -p host-iced -- target/components/demo-todo.component.wasm

run-desktop-iced: components components-as components-wat
	cargo run -p host-iced -- --registry registry.json target/components/demo-desktop.component.wasm

# Web host: one self-contained HTML page per component (the wasm module
# inlined as base64), plus the desktop page (the registry manifest's nine
# entries). The pages run straight from disk — no server, no npm. The
# desktop page's icons are the demos' own files, copied to target/web/icons/.
web: components components-as components-wat
	@mkdir -p target/web
	@rm -rf target/web/icons
	@mkdir -p target/web/icons
	@cp $(ICONS) target/web/icons/
	@for c in $(COMPONENTS); do \
		lib=$$(printf '%s' "$$c" | tr '-' '_'); \
		node web/build.mjs target/$(WASM_TARGET)/release/$$lib.wasm target/web/$$c.html || exit 1; \
	done
	@node web/build.mjs target/$(WASM_TARGET)/release/demo_desktop.wasm target/web/demo-desktop.html \
		--registry registry.json --wasm-target $(WASM_TARGET) || exit 1
	@echo "wrote target/web/*.html — open them directly (no server needed)"

# The same self-contained pages for the AssemblyScript components. The web
# host runs the pre-lift core module, so it consumes the renamed modules
# straight from target/assembly/ — glue.js and app.js are untouched.
web-as: components components-as components-wat
	@mkdir -p target/web
	@rm -rf target/web/icons
	@mkdir -p target/web/icons
	@cp $(ICONS) target/web/icons/
	@for c in $(AS_COMPONENTS); do \
		node web/build.mjs target/assembly/$$c-as.wasm target/web/$$c-as.html || exit 1; \
	done
	@node web/build.mjs target/assembly/demo-desktop-as.wasm target/web/demo-desktop-as.html \
		--registry registry.json --wasm-target $(WASM_TARGET) || exit 1
	@echo "wrote target/web/*-as.html — open them directly (no server needed)"

# The same self-contained pages for the WAT components. As with the AS
# pages, the web host runs the pre-lift core module (target/wat/).
web-wat: components components-as components-wat
	@mkdir -p target/web
	@rm -rf target/web/icons
	@mkdir -p target/web/icons
	@cp $(ICONS) target/web/icons/
	@for c in $(WAT_COMPONENTS); do \
		node web/build.mjs target/wat/$$c.wasm target/web/$$c-wat.html || exit 1; \
	done
	@node web/build.mjs target/wat/demo-desktop.wasm target/web/demo-desktop-wat.html \
		--registry registry.json --wasm-target $(WASM_TARGET) || exit 1
	@echo "wrote target/web/*-wat.html — open them directly (no server needed)"

# The GitHub Pages site: one index page — ?demo= and ?producer= select the
# module (counter|stopwatch|todo|desktop, rust|as|wat; the default demo is
# the desktop) — plus the twelve .wasm modules it fetches, the nine icons
# (the site's root icons/ set: the demos' own files), and the two browser
# host pages (tui/, iced/) with the twelve precompiled pulley32 artifacts
# they fetch (the shared components/ directory) and the registry.json at
# the root. The pages fetch, so the site needs a server (GitHub Pages, or
# python3 -m http.server).
# .github/workflows/site.yml deploys it on push to main.
site: components components-as components-wat host-tui-wasm host-iced-wasm
	# Rebuild the directory from scratch: the copies below are unconditional,
	# so a stale file from an older layout would otherwise survive.
	@rm -rf target/site
	@mkdir -p target/site/icons target/site/components target/site/tui/vendor target/site/iced
	@cp $(ICONS) target/site/icons/
	@for c in $(COMPONENTS); do \
		lib=$$(printf '%s' "$$c" | tr '-' '_'); \
		cp target/$(WASM_TARGET)/release/$$lib.wasm target/site/$$c.wasm || exit 1; \
	done
	@for c in $(AS_COMPONENTS); do \
		cp target/assembly/$$c-as.wasm target/site/$$c-as.wasm || exit 1; \
	done
	@for c in $(WAT_COMPONENTS); do \
		cp target/wat/$$c.wasm target/site/$$c-wat.wasm || exit 1; \
	done
	@cp target/$(WASM_TARGET)/release/demo_desktop.wasm target/site/demo-desktop.wasm || exit 1
	@cp target/assembly/demo-desktop-as.wasm target/site/demo-desktop-as.wasm || exit 1
	@cp target/wat/demo-desktop.wasm target/site/demo-desktop-wat.wasm || exit 1
	# The single index page: the module URL from ?demo=/?producer= (the iced
	# and tui host pages take the same pair), the registry inlined at build
	# time (applied by the page only for the desktop).
	@node web/build.mjs target/site/demo-desktop.wasm target/site/index.html \
		--url-params --name demos --registry registry.json || exit 1
	# The two browser host pages: each host's pkg (wasm-bindgen) and its
	# page (the tui page also takes the vendored xterm.js), the registry
	# manifest at the root (the root index page never fetches it), and the
	# twelve precompiled artifacts the pages fetch by ?demo= and ?producer=
	# (the artifacts are host-independent, so one shared components/
	# directory).
	@cp web/tui/index.html target/site/tui/
	@cp web/tui/vendor/xterm.js web/tui/vendor/xterm.css web/tui/vendor/LICENSE target/site/tui/vendor/
	@cp target/host-tui/pkg/host_tui.js target/host-tui/pkg/host_tui_bg.wasm target/site/tui/
	@cp web/iced/index.html target/site/iced/
	@cp target/host-iced/pkg/host_iced.js target/host-iced/pkg/host_iced_bg.wasm target/site/iced/
	@cp registry.json target/site/
	# The rust artifacts came with the host builds; precompile the other
	# two producers (the pages take all nine, plus the desktop's three).
	@for c in $(COMPONENTS) demo-desktop; do \
		for s in -as -wat; do \
			echo "==> precompiling $$c$$s for pulley32"; \
			cargo run -p host-precompile -- target/components/$$c$$s.component.wasm target/components/$$c$$s.cwasm || exit 1; \
		done; \
	done
	@for d in counter stopwatch todo desktop; do \
		for s in "" -as -wat; do \
			cp target/components/demo-$$d$$s.cwasm target/site/components/ || exit 1; \
		done; \
	done
	@echo "wrote target/site/ — serve it over http; .github/workflows/site.yml deploys it"

# Headless check of the web host's ABI layer (Node).
web-test: components
	node web/selftest.mjs

# The same selftest against the AssemblyScript core modules.
web-test-as: components-as
	node web/selftest.mjs target/assembly/demo-counter-as.wasm target/assembly/demo-stopwatch-as.wasm \
		target/assembly/demo-todo-as.wasm target/assembly/demo-desktop-as.wasm

# The same selftest against the WAT core modules: the byte-parity gate's
# third producer.
web-test-wat: components-wat
	node web/selftest.mjs target/wat/demo-counter.wasm target/wat/demo-stopwatch.wasm \
		target/wat/demo-todo.wasm target/wat/demo-desktop.wasm

# The browser TUI host: the wasmtime host as wasm (Pulley interpreter).
# Builds the cdylib (no crossterm, no native binary), runs wasm-bindgen,
# and precompiles the demo components natively for pulley32 — the artifacts
# the page's `run` deserializes. Needs wasm-bindgen on the PATH.
host-tui-wasm: components
	cargo build -p host-tui --no-default-features --release --target $(WASM_TARGET)
	wasm-bindgen target/$(WASM_TARGET)/release/host_tui.wasm --target web --out-dir target/host-tui/pkg
	@for c in $(COMPONENTS) demo-desktop; do \
		echo "==> precompiling $$c for pulley32"; \
		cargo run -p host-precompile -- target/components/$$c.component.wasm target/components/$$c.cwasm || exit 1; \
	done
	@echo "wrote target/host-tui/pkg/ and target/components/*.cwasm"

# The byte-parity gate: the wasm host's `dump` frame against the native
# `host-tui --dump` frame, per demo (Node). The desktop scenario opens a
# window, so the page's background fetch must have landed: site
# precompiles all twelve artifacts, which the selftest reads.
host-tui-wasm-test: site
	cargo build -p host-tui
	node web/tui/selftest.mjs

# The browser iced host: the wasmtime host as wasm (Pulley interpreter)
# with iced's Web runtime. Builds the cdylib (no native windowing
# backends), runs wasm-bindgen, and precompiles the demo components
# natively for pulley32 — the artifacts the page's `run` deserializes
# (the same ones the tui host takes; the artifact is host-independent).
# Needs wasm-bindgen on the PATH.
host-iced-wasm: components
	cargo build -p host-iced --no-default-features --release --target $(WASM_TARGET)
	wasm-bindgen target/$(WASM_TARGET)/release/host_iced.wasm --target web --out-dir target/host-iced/pkg
	@for c in $(COMPONENTS) demo-desktop; do \
		echo "==> precompiling $$c for pulley32"; \
		cargo run -p host-precompile -- target/components/$$c.component.wasm target/components/$$c.cwasm || exit 1; \
	done
	@echo "wrote target/host-iced/pkg/ and target/components/*.cwasm"

# The byte-parity gate: the wasm host's `dump` PNG against the native
# `host-iced --dump` PNG, per demo (Node). The desktop scenario opens a
# window, so the page's background fetch must have landed: site
# precompiles all twelve artifacts, which the selftest reads.
host-iced-wasm-test: site
	cargo build -p host-iced
	node web/iced/selftest.mjs

# The tree layer's diff channel, as a standalone module run via Node: the
# structural cases (add, remove, re-parent) plus the bootstrap and id-less
# rules — the AS twin of the Rust `component-ui` unit tests. The trees are
# written in JSX (lowered by assembly/jsx.mjs, as in the demos). Needs node +
# `npm install` (asc, babel) and wabt (the patch step).
ui-test:
	@test -x node_modules/.bin/asc || { echo "ui-test needs node/npm — run: npm install"; exit 1; }
	@test -d node_modules/@babel/core || { echo "babel not installed — run: npm install"; exit 1; }
	@mkdir -p target/assembly
	node assembly/jsx.mjs assembly/ui/tree.test.tsx assembly/ui/tree.test.gen.ts || exit 1; \
	npx asc assembly/ui/tree.test.gen.ts --outFile target/assembly/tree-test.raw.wasm \
		-O --noAssert --runtime incremental --memoryBase 65536 --initialMemory 2 || exit 1; \
	node assembly/patch-env-abort.mjs target/assembly/tree-test.raw.wasm target/assembly/tree-test.core.wasm || exit 1; \
	node assembly/ui/tree.test.mjs target/assembly/tree-test.core.wasm

# Browser E2E (Playwright, chromium): the site's index page (the nine demos
# and the desktop, by ?demo=/?producer=) and the two browser host pages,
# served from target/site/ (make site). Needs `npm install`, a
# one-time `npx playwright install chromium`, and wasm-bindgen on the PATH
# (the host pages' builds).
e2e: site
	@test -d node_modules/@playwright/test || { echo "e2e needs node/npm — run: npm install"; exit 1; }
	npx playwright test $(E2E_ARGS)

# Regenerate the goldens on the CI platform: dispatch the CI workflow with
# its update_snapshots input. The e2e screenshots must render on the runner
# (font rendering is platform-specific), so they cannot be generated
# locally; the run commits the changed goldens to the branch. Needs the gh
# CLI, authenticated.
goldens:
	@command -v gh >/dev/null 2>&1 || { echo "goldens needs the gh CLI (https://cli.github.com/), authenticated"; exit 1; }
	@gh workflow run ci.yml -f update_snapshots=true
	@echo "when green, the run commits the changed goldens to the branch — review the commit; a cancelled run commits nothing (re-dispatch)"

# The README's tui frames, rendered to PNGs via freeze. The web and iced
# frames link straight to their committed goldens. Needs: brew install
# charmbracelet/tap/freeze
screenshots:
	@mkdir -p docs/screenshots
	@./tools/tui-screenshots.sh

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
