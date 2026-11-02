# LoqWat — Development Process Log

Running record of decisions, environment, progress, and issues.
Newest log entries at the bottom of each section. Dates are absolute (YYYY-MM-DD).

---

## 1. Project summary

**LoqWat** — local, private dictation. Click a text field in any desktop app, press a
global shortcut, speak, stop; LoqWat transcribes on-device (whisper.cpp) and inserts
the text. No accounts, cloud, analytics, or transcript storage.

- Primary target: macOS, Apple Silicon (MacBook Pro 14" 2021, M1 Pro, 16 GB).
- Secondary target: Windows (friend's machine; hardware TBD) — architecture-ready, built after Mac works.
- Source spec: build plan pasted at project start (originally titled "Local Dictation"; renamed LoqWat — all naming uses LoqWat).

## 2. Environment

| Item | Value | Verified |
|---|---|---|
| Dev host OS | macOS 27.0.1 (build 26A434), arm64 | 2026-11-02 |
| SDK | macOS 27.0 (Command Line Tools only; no full Xcode) | 2026-11-02 |
| git | 2.54.0 | 2026-11-02 |
| gh CLI | 2.92.0, logged in as RadioUFHM | 2026-11-02 |
| Node / npm | 25.9.0 / 11.12.1 | 2026-11-02 |
| Homebrew | 5.1.11 | 2026-11-02 |
| CMake | 4.4.5 (Homebrew) | 2026-11-02 |
| Rust | 1.99.1 stable, aarch64-apple-darwin (rustup, minimal profile) | 2026-11-02 |

**Repository:** https://github.com/RadioUFHM/LoqWat (private), branch `main`.

### Host quirks
- Anaconda's `curl` (`/opt/anaconda3/bin/curl`) is broken (missing `libgssapi_krb5`) and
  shadows `/usr/bin/curl`. Workaround: put `/usr/bin` first in PATH for build commands.
  Fix: `conda update curl` or `conda install krb5`.
- `~/.bash_profile` is owned by root, so rustup couldn't edit it. Rust PATH must be added
  to `~/.zshrc` manually: `echo '. "$HOME/.cargo/env"' >> ~/.zshrc`. (Pending — user action.)

## 3. Key decisions

| Date | Decision | Rationale |
|---|---|---|
| 2026-11-02 | App name **LoqWat**; bundle ID `com.fcitradecraft.loqwat` | User instruction; stable identity keeps macOS permissions intact across builds |
| 2026-11-02 | Stack: Tauri 2 + Rust + minimal TypeScript UI + whisper.cpp | Per build spec |
| 2026-11-02 | GitHub repo is **private** | Personal utility; can be made public later |
| 2026-11-02 | Models, audio, build outputs, installers excluded via `.gitignore` | Spec §6/§8: never commit models, recordings, binaries |
| 2026-11-02 | Tauri **2.12.x** (not 3.0 alpha); `@tauri-apps/cli` + `api` pinned to 2.12.4 | Spec requires Tauri 2; 3.x is alpha only |
| 2026-11-02 | Whisper binding: **whisper-rs 0.16** (bundles whisper.cpp **1.8.3**), `metal` feature on macOS, CPU elsewhere | Maintained, Metal + Vulkan/CUDA features, built-in VAD API, builds whisper.cpp from source (no system install) |
| 2026-11-02 | UI: vanilla TypeScript + Vite (no framework) | Spec asks for minimal settings/status UI |
| 2026-11-02 | Rust toolchain pinned to 1.99.1 (`rust-toolchain.toml`); lockfiles `package-lock.json` + `src-tauri/Cargo.lock` committed | Reproducible builds |
| 2026-11-02 | macOS minimum version 13.0 (provisional) | Broad compatibility; revisit if a needed API requires newer |
| 2026-11-02 | `build.rs` links `libclang_rt.osx.a` as a trailing link arg | ggml Metal `@available` checks need `__isPlatformVersionAtLeast` when deployment target < 27 |
| 2026-11-02 | Diagnostic tools live in `src-tauri/examples/` | Tauri bundles every `[[bin]]` into the .app; examples are excluded |
| 2026-11-02 | whisper-rs-sys built at opt-level 3 even in dev profile | Dev-mode transcription timings stay meaningful |
| 2026-11-02 | Removed `tauri-plugin-opener`; strict CSP; `withGlobalTauri` off | Not needed; smaller attack surface |

## 4. Milestones

Status key: ⬜ not started · 🟨 in progress · ✅ done · 🧪 tested (on real OS) · ⛔ blocked

| # | Milestone | Spec § | Status |
|---|---|---|---|
| 0 | Repo + toolchain + Tauri scaffold | 1–2 | ✅ 🧪 release .app built and launched on dev Mac |
| 1 | Prove transcription (file → mic), benchmark models | 3 | ⬜ |
| 2 | Recording lifecycle / state machine / indicator | 4 | ⬜ |
| 3 | Mac text insertion + clipboard + recovery window | 5 | ⬜ |
| 4 | First-run setup, model manager, preferences | 6 | ⬜ |
| 5 | Formatting, vocabulary hints, privacy/resource bounds | 7–8 | ⬜ |
| 6 | Mac validation + packaging (.app/.dmg) | 9–10 | ⬜ |
| 7 | Windows support | 11 | ⬜ (needs friend's OS/CPU/RAM) |
| 8 | Final deliverables & report | 12 | ⬜ |

## 5. Open questions / blocked items

- Windows friend's OS version, CPU architecture (x64/ARM64), RAM, GPU — needed before Milestone 7.
- Code signing / notarization credentials — not available; Mac build will be labeled unsigned local build.
- App icons are Tauri's default placeholders — replace with LoqWat artwork before distribution.
- Metal shader library compiles at first engine init (~8.9 s measured). Check in Milestone 1 whether
  macOS caches it across launches; if not, warm the engine in the background at app start.

## 6. Test & benchmark results

_(Filled in as tests actually run. Only real-OS runs count as tested.)_

### 2026-11-02 — Milestone 0 build verification (MacBook Pro M1 Pro, macOS 27.0.1)
| Check | Result |
|---|---|
| `whisper_info` (debug) | whisper.cpp 1.8.3; Metal detected (Apple M1 Pro, MTLGPUFamilyApple7, unified memory, ~12.7 GB working set); CPU: NEON, ARM_FMA, FP16, DOTPROD, Accelerate |
| Metal library first init | 8.9 s (embedded library, compiled at runtime) |
| Release `.app` build | ✅ 4.34 MiB; arm64 Mach-O; `CFBundleIdentifier` = com.fcitradecraft.loqwat; `LSMinimumSystemVersion` 13.0 |
| Dynamic libs | System only (libc++, libSystem, libiconv, libobjc) — no Homebrew/dev deps |
| Release app launch | ✅ launched via `open`, stayed running; idle RSS ≈ 104 MB, CPU 0.0 %. Window contents not visually verified by Claude |
| `cargo clippy --all-targets`, `tsc --noEmit` | ✅ no warnings |

## 7. Session log

### 2026-11-02
- Inspected empty project folder and dev host.
- `git init -b main`; added `.gitignore` and `README.md`; initial commit `97c0f93`.
- Created private GitHub repo `RadioUFHM/LoqWat` via `gh repo create`; pushed `main`.
- Installed CMake 4.4.5 (Homebrew).
- Rust install: first attempt failed (Anaconda curl); second attempt with `/usr/bin/curl`
  installed rustup but couldn't edit root-owned `~/.bash_profile`; ran `rustup default stable` → Rust 1.99.1.
- Attempt to append cargo env to `~/.zshrc` blocked by Claude Code safety policy — left for user.
- Created this process log.

#### Milestone 0
- Scaffolded with `create-tauri-app` (vanilla-ts, npm, Tauri 2) in scratchpad, copied into repo,
  then customized: product name LoqWat, bundle ID, minimal status UI (`app_info` command),
  strict CSP, bundle targets app+dmg, removed opener plugin and template assets.
- Added whisper-rs 0.16 (`metal` on macOS). First debug build OK; Metal confirmed on M1 Pro.
- First release build **failed** at link: `___isPlatformVersionAtLeast` undefined (ggml-metal
  `@available` checks + deployment target 13.0). Fix 1 (`rustc-link-lib=static=clang_rt.osx`)
  only partly resolved it (archive ordering). Fix 2: pass full path to `libclang_rt.osx.a` via
  `rustc-link-arg` → builds.
- Tauri bundled the `whisper-info` diagnostic bin into the .app → moved to `examples/`.
- Release .app built (4.34 MiB), launched, idle resources measured; clippy/tsc clean.
- **Next:** Milestone 1 — download `base.en` / `small.en` (pinned revision + SHA verification),
  file-transcription benchmark (cold vs warm, memory), then microphone capture via `cpal`.
