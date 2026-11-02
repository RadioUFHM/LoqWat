# LoqWat

Local, private push-to-talk dictation for macOS (Apple Silicon) and Windows.
Speak into any text field; transcription runs on-device with whisper.cpp.

Status: early development — see [PROCESS.md](PROCESS.md) for progress and decisions.

## Stack

- [Tauri 2](https://v2.tauri.app/) desktop shell, Rust application logic
- Minimal TypeScript UI (Vite, no framework)
- [whisper.cpp](https://github.com/ggml-org/whisper.cpp) via [`whisper-rs`](https://codeberg.org/tazz4843/whisper-rs) — Metal on Apple Silicon, CPU on Windows

## Development setup (macOS)

Requirements: Xcode Command Line Tools, Rust (via [rustup](https://rustup.rs); version pinned in
`rust-toolchain.toml`), CMake, Node.js ≥ 20.

```bash
xcode-select --install          # if not already installed
brew install cmake
npm install
```

## Common commands

```bash
npm run tauri dev                                   # run the app in development mode
npx tauri build --bundles app                       # release .app → src-tauri/target/release/bundle/macos/
cd src-tauri && cargo run --example whisper_info    # print whisper.cpp version + compiled backends
cd src-tauri && cargo clippy --all-targets          # lint
```

## Repository layout

```
index.html, src/        TypeScript UI (settings/status)
src-tauri/              Rust app (Tauri)
  src/                  application code
  examples/             dev/diagnostic tools (never bundled)
  build.rs              Tauri build + macOS clang runtime link fix
  tauri.conf.json       app identity, window, bundle config
PROCESS.md              development log, decisions, milestone status
```

Models, recordings and build outputs are never committed (see `.gitignore`).
