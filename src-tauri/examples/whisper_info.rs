//! Prints the bundled whisper.cpp version and the backends it was compiled with.
//! Used to confirm Metal is enabled on Apple Silicon builds.

fn main() {
    println!("whisper.cpp version: {}", whisper_rs::get_whisper_version());
    println!("system info: {}", whisper_rs::print_system_info());
    println!(
        "build target: {}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
}
