//! Shared build-and-load helpers for the headless tests. The three wasmtime
//! host test suites (the round-trip contract test and the two `--dump` render
//! checks) each build the demo components and lift them before driving them;
//! this crate holds that machinery once.
//!
//! The tests share their built artifacts on disk and run on parallel threads,
//! so the build *and* load window is serialized per artifact file (the two
//! locks below): one test's lift cannot clobber a file while a sibling is
//! reading it.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Mutex;

/// The repository root: the parent of the `crates/` directory this crate
/// sits in.
pub fn waui_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

/// The lifted Rust component for a demo.
pub fn component_path(demo: &str) -> PathBuf {
    waui_root()
        .join("target/components")
        .join(format!("{demo}.component.wasm"))
}

/// The lifted AssemblyScript component for a demo.
pub fn component_path_as(demo: &str) -> PathBuf {
    waui_root()
        .join("target/components")
        .join(format!("{demo}-as.component.wasm"))
}

/// Build the component and lift it, so the test is self-contained. Cargo
/// names the artifact with `_` where the crate name has `-`.
pub fn build_component(demo: &str) {
    let root = waui_root();
    let wasm = root
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}{}", demo.replace('-', "_"), ".wasm"));
    let out = component_path(demo);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).expect("failed to create target/components");
    }

    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            demo,
            "--release",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .current_dir(&root)
        .status()
        .expect("failed to run cargo");
    assert!(status.success(), "failed to build {demo}");

    let status = Command::new("wasm-tools")
        .args([
            "component",
            "new",
            wasm.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ])
        .current_dir(&root)
        .status()
        .expect("failed to run wasm-tools");
    assert!(status.success(), "failed to lift {demo} into a component");
}

// The AssemblyScript build needs node + the local npm toolchain (asc, babel);
// without it the AS tests skip rather than fail.
pub fn as_toolchain_available() -> bool {
    waui_root().join("node_modules/.bin/asc").exists()
        && waui_root().join("node_modules/@babel/core").exists()
        && Command::new("node")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
}

// Serialize the Rust build+load window per artifact file (see the crate docs).
pub static RUST_BUILD_LOCK: Mutex<()> = Mutex::new(());
// Serialize the AssemblyScript build+load window per artifact file.
pub static AS_BUILD_LOCK: Mutex<()> = Mutex::new(());

/// Build the AssemblyScript component and lift it:
/// jsx (babel lowers the `.tsx` to `h()` calls — asc has no JSX parser)
/// -> asc (the incremental runtime) -> the `env.abort` patch -> export rename
/// -> component embed (the WIT) -> component new.
/// (Callers hold [`AS_BUILD_LOCK`] across the build *and* the load, so a
/// sibling test cannot lift over the file while we read it.)
pub fn build_component_as(demo: &str) {
    let root = waui_root();
    std::fs::create_dir_all(root.join("target/assembly"))
        .expect("failed to create target/assembly");
    std::fs::create_dir_all(root.join("target/components"))
        .expect("failed to create target/components");

    let src = format!("assembly/{demo}/index.tsx");
    let gen = format!("assembly/{demo}/index.gen.ts");
    let raw = root
        .join("target/assembly")
        .join(format!("{demo}.raw.wasm"));
    let core = root
        .join("target/assembly")
        .join(format!("{demo}.core.wasm"));
    let renamed = root.join("target/assembly").join(format!("{demo}-as.wasm"));
    let embedded = root
        .join("target/assembly")
        .join(format!("{demo}-as.embedded.wasm"));
    let out = component_path_as(demo);

    let mut steps: Vec<Command> = Vec::new();
    {
        let mut c = Command::new("node");
        c.args(["assembly/jsx.mjs", src.as_str(), gen.as_str()]);
        steps.push(c);
    }
    {
        let mut c = Command::new("npx");
        c.args(["asc", gen.as_str(), "--outFile", raw.to_str().unwrap()]);
        c.args([
            "-O",
            "--noAssert",
            "--runtime",
            "incremental",
            "--memoryBase",
            "65536",
            "--initialMemory",
            "2",
        ]);
        steps.push(c);
    }
    {
        // The incremental runtime's one import (`env.abort`, its OOM path)
        // is replaced with an in-module `unreachable` so the lifted
        // component is import-free again (see assembly/patch-env-abort.mjs).
        let mut c = Command::new("node");
        c.args([
            "assembly/patch-env-abort.mjs",
            raw.to_str().unwrap(),
            core.to_str().unwrap(),
        ]);
        steps.push(c);
    }
    {
        let mut c = Command::new("node");
        c.args([
            "assembly/rename-exports.mjs",
            core.to_str().unwrap(),
            renamed.to_str().unwrap(),
            "onclick=on-click",
            "ontext=on-text",
            "ontick=on-tick",
        ]);
        steps.push(c);
    }
    {
        let mut c = Command::new("wasm-tools");
        c.args([
            "component",
            "embed",
            "wit/",
            renamed.to_str().unwrap(),
            "--output",
            embedded.to_str().unwrap(),
        ]);
        steps.push(c);
    }
    {
        let mut c = Command::new("wasm-tools");
        c.args([
            "component",
            "new",
            embedded.to_str().unwrap(),
            "--output",
            out.to_str().unwrap(),
        ]);
        steps.push(c);
    }
    for step in &mut steps {
        step.current_dir(&root);
        let status = step.status().expect("failed to run a build tool");
        assert!(
            status.success(),
            "failed to build the AssemblyScript {demo}"
        );
    }
}
