use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        link_clang_runtime();
    }
    tauri_build::build()
}

/// ggml's Metal backend uses `@available(...)` checks, which compile to calls to
/// `__isPlatformVersionAtLeast` when the deployment target is older than the APIs
/// being checked. That symbol lives in clang's compiler-rt (`libclang_rt.osx.a`),
/// which rustc does not link by default, so link it explicitly.
fn link_clang_runtime() {
    let output = Command::new("clang")
        .arg("--print-resource-dir")
        .output()
        .expect("failed to run `clang --print-resource-dir`; are the Xcode Command Line Tools installed?");
    assert!(
        output.status.success(),
        "`clang --print-resource-dir` failed"
    );
    let resource_dir = String::from_utf8(output.stdout).expect("non-UTF-8 clang resource dir");
    // Passed as a trailing link arg (not `rustc-link-lib`) so it follows the
    // whisper-rs-sys archive on the linker command line for every binary.
    println!(
        "cargo:rustc-link-arg={}/lib/darwin/libclang_rt.osx.a",
        resource_dir.trim()
    );
}
