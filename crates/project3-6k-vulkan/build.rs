use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-env-changed=VULKAN_SDK");
    let sdk_root = env::var_os("VULKAN_SDK")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("VULKAN_SDK must point to a Vulkan SDK installation"));
    let compiler = sdk_root
        .join("Bin")
        .join(if cfg!(windows) { "glslc.exe" } else { "glslc" });
    if !compiler.is_file() {
        panic!("could not find GLSL compiler at {}", compiler.display());
    }

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo sets this"));
    let output_dir = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets this"));
    for shader in ["forest.vert", "forest.frag"] {
        let source = manifest_dir.join("shaders").join(shader);
        let output = output_dir.join(format!("{shader}.spv"));
        println!("cargo:rerun-if-changed={}", source.display());
        let status = Command::new(&compiler)
            .arg("--target-env=vulkan1.0")
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .status()
            .unwrap_or_else(|error| {
                panic!("could not run {}: {error}", compiler.display());
            });
        if !status.success() {
            panic!("GLSL compilation failed for {}", source.display());
        }
    }
}
