//! The build-time half of the browser hosts' precompile/deserialize design:
//! compile a waui component natively (Cranelift) for the pulley32 target and
//! write the serialized artifact the browser host deserializes. The browser
//! never compiles (no executable memory in wasm): it only interprets the
//! artifact on the Pulley interpreter.
//!
//!   host-precompile <component.wasm> <artifact.cwasm>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 2 {
        eprintln!("usage: host-precompile <component.wasm> <artifact.cwasm>");
        std::process::exit(2);
    }
    let bytes = std::fs::read(&args[0])?;

    // The shared pulley32 config the browser host deserializes against,
    // plus `wasm_threads(false)`: the full native build has the `threads`
    // feature, and the artifact records the resolved value, which must
    // match the browser (threads off by construction there).
    let mut config = host_shared::pulley_config();
    config.wasm_threads(false);
    let engine = wasmtime::Engine::new(&config)?;

    let artifact = engine.precompile_component(&bytes)?;
    std::fs::write(&args[1], &artifact)?;
    println!("wrote {} ({} bytes)", args[1], artifact.len());
    Ok(())
}
