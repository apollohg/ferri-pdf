//! Gives the WebAssembly module the stack the renderer's nesting limits assume.

/// Twice the 2 MiB stack the deepest accepted nesting is measured against.
const STACK_BYTES: usize = 4 * 1024 * 1024;
const WASM_FAMILY: &str = "wasm";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let wasm = std::env::var("CARGO_CFG_TARGET_FAMILY")
        .is_ok_and(|families| families.split(',').any(|family| family == WASM_FAMILY));
    if wasm {
        println!("cargo:rustc-link-arg-cdylib=-zstack-size={STACK_BYTES}");
    }
}
