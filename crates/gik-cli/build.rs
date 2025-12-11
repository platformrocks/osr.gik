use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Get the short git hash
    // Priority: 1) git command, 2) GIT_HASH env var (from Docker), 3) "unknown"
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("GIT_HASH").ok().filter(|s| s != "unknown" && !s.is_empty()))
        .unwrap_or_else(|| "unknown".to_string());

    // Set the GIT_HASH environment variable for the build
    println!("cargo:rustc-env=GIT_HASH={}", git_hash);

    // Rebuild if the git HEAD changes
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads/");

    // Windows resource file for executable metadata
    // Compile for any Windows target (native or cross-compiled)
    let target = env::var("TARGET").unwrap();
    if target.contains("windows") {
        let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
        let resource_file = PathBuf::from(&manifest_dir)
            .join("resources")
            .join("windows")
            .join("gik.rc");

        if resource_file.exists() {
            // Tell cargo to compile and link the resource file
            println!("cargo:rerun-if-changed={}", resource_file.display());
            
            // For MinGW cross-compilation, ensure windres is configured
            if target.contains("gnu") {
                // Check if WINDRES is set, if not set it to the MinGW windres
                if env::var("WINDRES").is_err() {
                    let windres = format!("x86_64-w64-mingw32-windres");
                    env::set_var("WINDRES", &windres);
                    println!("cargo:warning=Setting WINDRES to {}", windres);
                }
            }
            
            // Use embed_resource crate to handle .rc compilation
            // This works with both MSVC and MinGW toolchains
            embed_resource::compile(&resource_file, embed_resource::NONE);
        }
    }
}
