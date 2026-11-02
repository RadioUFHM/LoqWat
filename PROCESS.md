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
| 2026-11-02 | Models from `huggingface.co/ggerganov/whisper.cpp` pinned to revision `5359861c…`; size + SHA-256 checked; download to `<file>.part`, rename only after verification | Spec §6: trusted source, pinned, integrity-checked, atomic |
| 2026-11-02 | Models stored in `~/Library/Application Support/com.fcitradecraft.loqwat/models` (Windows: `%APPDATA%\com.fcitradecraft.loqwat\models`) | Outside source tree and app bundle |
| 2026-11-02 | Audio libs: `cpal` 0.18 (capture), `rubato` 5 FFT resampler (→16 kHz), `hound` (WAV); HTTP: `ureq` 3 (rustls); hashing: `sha2` | Maintained, pure-Rust where possible, cross-platform |
| 2026-11-02 | Engine keeps one `WhisperContext` + one reusable `WhisperState`; greedy decoding default; `no_context`, `suppress_blank`, `suppress_nst`; language fixed to English | Persistent warm engine (spec §2); no carry-over between utterances |
| 2026-11-02 | Clips < 1.1 s padded with trailing silence | whisper.cpp rejects/garbles very short input |
| 2026-11-02 | `clean_transcript` strips `[...]` / `(...)` non-speech annotations (e.g. `[BLANK_AUDIO]`); never edits words | Avoid inserting noise labels; preserve what the user said |
| 2026-11-02 | Mic capture held in memory, capped at 300 s, sets `limit_reached` flag instead of silently truncating | Spec §4/§8 |
| 2026-11-02 | Benchmark audio is synthetic (macOS `say` voices + generated noise), generated by `scripts/make_test_audio.sh` into git-ignored `test-audio/` | Repeatable; real-voice recordings still required to confirm |
| 2026-11-02 | **Provisional default model: `small.en-q5_1`** (190 MB) | Same accuracy as full small.en on all tests, ~385 ms warm for 13 s audio, ~405 MB peak RSS vs 744 MB. Pending real-voice confirmation |
| 2026-11-02 | Vocabulary hints passed as whisper `initial_prompt` | Fixed every FinCEN/OFAC/KYC error in tests across all models |
| 2026-11-02 | **Silero VAD v6.2.0 speech gate** (`ggml-org/whisper-vad` @ `9ffd54a1…`, 885 KB) before transcription; gate only, never trims audio; threshold 0.5, min speech 150 ms | whisper invents "you"/"Thank you." on silence/noise with no-speech prob 0.00, so that signal is useless; VAD rejected both test cases in ~12 ms |

## 4. Milestones

Status key: ⬜ not started · 🟨 in progress · ✅ done · 🧪 tested (on real OS) · ⛔ blocked

| # | Milestone | Spec § | Status |
|---|---|---|---|
| 0 | Repo + toolchain + Tauri scaffold | 1–2 | ✅ 🧪 release .app built and launched on dev Mac |
| 1 | Prove transcription (file → mic), benchmark models | 3 | ✅ 🧪 file + live mic verified on dev Mac (accuracy pending user confirmation) |
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
- ~~Metal shader library compiles at first engine init (~8.9 s).~~ Resolved: macOS caches it; every later
  cold load measured 135–340 ms. The very first launch after install may still be slow — show "loading".
- Real-voice accuracy unverified (all benchmarks so far are TTS). Needs user recordings.
- Small models misheard "Every analyst" as "three analysts" in the 50 s TTS clip (base.en and large-turbo
  got it right). Watch for this in real-voice tests.
- Numbers are written as digits ("14", "1,200", "$4.7 million"). This is model output style, not an error;
  confirm the user is OK with it.

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

### 2026-11-02 — Milestone 1 transcription benchmark (M1 Pro, macOS 27.0.1, Metal, greedy, release build)
Test audio: synthetic TTS (Samantha US, Daniel UK, Moira IE) at 48 kHz mono, plus a 44.1 kHz stereo float copy,
TTS + Gaussian noise, digital silence, noise only. Warm = median of repeated runs with the model already loaded.
Each model ran in a fresh process.

| Model | Disk | Cold load | Peak RSS | Warm 3.5 s | Warm 13 s | Warm 49.5 s | Vocab errors (no hint → with hint) |
|---|---|---|---|---|---|---|---|
| base.en | 148 MB | 135–171 ms | 336 MB | 85 ms | 177 ms | 595 ms | many → 1 ("Kite" for KYC, IE voice) |
| small.en-q5_1 | 190 MB | 157–177 ms | 405 MB | 238 ms | 385 ms | 1,117 ms | several → **0** |
| small.en | 488 MB | 275–281 ms | 744 MB | 222 ms | 393 ms | 1,205 ms | several → **0** |
| large-v3-turbo-q5_0 | 574 MB | 302–339 ms | 773 MB | 1,032 ms | 1,127 ms | 2,486 ms | several → 1 ("Offit", noisy) |

- **Latency target (≤ ~2 s after a 10–20 s utterance, warm): met by every model except large-turbo on long clips.**
- First transcription after load is ~150–450 ms slower than warm (GPU warm-up). The app should run one silent warm-up pass after loading.
- WER on the names clips is ~21% even when every word is right, because the reference spells numbers out
  ("fourteenth", "twelve hundred") and whisper writes digits. Read the transcripts, not just the WER.
- Long 49.5 s clip: 2.0–3.3% WER for all models.
- Ordinary sentence (both voices): 0% WER for all models.
- Stereo 44.1 kHz float input: output identical to 48 kHz mono (downmix + resample OK); conversion ≤ 52 ms for 50 s.
- Silence / noise only: **every model invented text** ("you", "Thank you.") with no-speech prob 0.00.
  Silero VAD gate rejected both (11–13 ms) and passed all speech clips (8–122 ms).
- Noise mixed into speech: little effect (with the hint, small models still perfect apart from "rule" for "rules").

### Microphone capture
- Input devices enumerate correctly (Yeti Stereo [default], Headphones, C922 webcam, MacBook Pro mic).
- First attempt blocked waiting on the macOS microphone permission dialog for the terminal app; user granted access.
- Bug: `mic_test` treated stdin EOF (non-interactive `!` shell) as "Enter" and stopped after 0.1 s. Fixed to stop only on a real line.
- **2026-11-02 live test ✅** (user's real voice, Yeti Stereo Microphone, 2 ch @ 48 kHz, small.en-q5_1 + vocabulary hint + VAD):
  15.1 s captured, peak 0.154 / RMS 0.021; VAD 5 segments, 14.1 s of speech; **transcribed in 363 ms** (warm),
  model load 95 ms, warm-up 215 ms, peak RSS 372 MB. FinCEN, OFAC and SAR came out correctly; numbers as digits.
  The text ended mid-sentence because the 15 s test limit stopped capture (expected). Word-level accuracy is
  pending the user's confirmation.
- Silent 3 s room capture: VAD 0 segments → nothing would be inserted ✅.

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

#### Milestone 1
- Added modules: `audio` (downmix, FFT resample → 16 kHz, WAV read, RMS/peak), `engine` (persistent
  whisper context + state, greedy, prompt, cancel flag, annotation cleanup), `models` (pinned catalog,
  verified atomic download, `.part` cleanup), `capture` (cpal, any sample format, 300 s cap, live level,
  stream errors), `vad` (Silero speech gate), `metrics` (peak RSS).
- Examples: `fetch_model`, `bench` (per-model child process, WER vs reference), `mic_test`.
- 12 unit tests (resampling, downmix, digest/size/truncation checks, `.part` cleanup, annotation stripping) pass.
- Downloaded and verified base.en, small.en, small.en-q5_1, large-v3-turbo-q5_0, silero-v6.2.0.
- Benchmarks run (see §6). Found the silence hallucination and added the VAD gate. The vocabulary hint fixed the domain terms.
- Live mic test blocked on the permission dialog → handed to the user → passed after the stdin-EOF fix (see §6).
- **Next:** Milestone 2: recording lifecycle state machine, global shortcut (tap, then hold), Escape cancel,
  session IDs, non-activating indicator.
