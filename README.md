# LoqWat

Local, private push-to-talk dictation for macOS (Apple Silicon) and Windows.
Speak into any text field; transcription runs on-device with whisper.cpp.

Status: early development, in daily use on the dev Mac. Dictation, insertion into the focused app, setup,
preferences and the model manager work; Windows support and packaging are not built yet.
See [PROCESS.md](PROCESS.md) for progress and decisions.

## Using it

Click into a text field, press **⌥ Space** (configurable), speak, and press it again (or release, in hold mode).
**Esc** cancels. The text is typed where your cursor was; if the target changed, it goes to a recovery window instead.

Say punctuation out loud: **comma**, **colon**, **semicolon**, **question mark**, **exclamation point**, **period**.
"period" only becomes "." at the end of a sentence, so "the lookback period ends Friday" stays as words. To end a
sentence with the word itself, say "period period".

Numbers: one through nine are spelled out and 10 and up are digits with commas (1,200). Money, percentages, decimals,
times, dates and labels (Rule 3, page 2) keep their digits, and years (2026) and IDs (Form 8300) get no commas.

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
  src/                  application code (format.rs: punctuation + number rules)
  examples/             dev/diagnostic tools (never bundled)
  build.rs              Tauri build + macOS clang runtime link fix
  tauri.conf.json       app identity, window, bundle config
PROCESS.md              development log, decisions, milestone status
```

Models, recordings and build outputs are never committed (see `.gitignore`).
