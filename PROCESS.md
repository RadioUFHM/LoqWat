# LoqWat — Development Process Log

Running record of decisions, environment, progress, and issues.
Newest log entries at the bottom of each section. Dates are absolute (YYYY-MM-DD).

---

## ▶ RESUME HERE (bookmarked 2026-11-07)

**Where we are:** Milestones 0–2 ✅🧪. **Milestones 3 and 4** are in daily use, and the user says they're "working well".
The full M3/M4 checklists in §6 have **not** been run, so they are not marked 🧪.
**Milestone 5** 🟨: `format.rs` handles spoken punctuation (including quotes and parentheses) and the user's number
style. It has been measured on the user's own voice (§6, "Real-voice results"): 3/5 exact on punctuation passages, and
the two misses are single misheard words. Default model stays `small.en-q5_1`. The user's verdict: "good so far".

**Before picking up again (user):**
1. Add "lookback" to Preferences → Vocabulary. The saved list predates the new default, so it won't appear automatically.
2. After any rebuild, reset Accessibility: `tccutil reset Accessibility com.fcitradecraft.loqwat`, then grant it again.

**Next, in order (open decisions in bold):**
1. **Personal "heard → write" correction list** in Preferences, e.g. "whole fact" → OFAC. OFAC is still misheard in
   natural speech. Proposed, not yet approved by the user.
2. Rest of Milestone 5: "new line" / "new paragraph", idle model unload, resource bounds.
3. **Stable self-signed signing identity** (pulled forward from M6?), so permissions survive rebuilds. Every rebuild
   currently breaks Accessibility.
4. The user runs the full M3/M4 checklists in §6.

After any change to the formatter or prompt, rescore the voice samples (recordings stay in git-ignored `recordings/voice/`):
`cd src-tauri && cargo run --release --example bench -- --models small.en-q5_1 --runs 1 --vad --format --show-text --prompt "FCI Tradecraft, FinCEN, BSA, AML, KYC, OFAC, lookback." ../recordings/voice/*.wav`

To relaunch: `open src-tauri/target/release/bundle/macos/LoqWat.app` (rebuild: `npx tauri build --bundles app`).
For logs, run the binary directly: `src-tauri/target/release/bundle/macos/LoqWat.app/Contents/MacOS/loqwat`.

**Known open items carried forward:** sleep/wake handling not implemented; placeholder icons; stable code-signing
identity (self-signed cert) so microphone/Accessibility permissions survive rebuilds (Milestone 6). A phone version
was considered and dropped (2026-11-03): the user's Android keyboard dictation already covers it.

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
  to `~/.zshrc` manually: `echo '. "$HOME/.cargo/env"' >> ~/.zshrc`. ✅ Done (verified 2026-11-07: new shell
  finds cargo 1.99.1 and rustc 1.99.1).

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

| 2026-11-02 | Default shortcut **⌥ Option+Space**, configurable later | Easy to press and to hold; registration errors shown in UI |
| 2026-11-02 | Escape registered as a global shortcut **only while recording/transcribing** | Cancels a session without swallowing Escape during normal use |
| 2026-11-02 | `Controller` is a pure state machine (events → actions), with session IDs | Unit-testable; stale or cancelled results can never insert |
| 2026-11-02 | Presses during Transcribing/Inserting are ignored (no queue); Escape during Inserting is ignored | Spec: no overlapping sessions or silent queued inserts; avoid a half-done insert |
| 2026-11-02 | Audio thread owns the cpal stream; engine thread owns the model + VAD; warm-up pass after load | cpal streams aren't `Send` on every platform; keeps inference off the UI thread |
| 2026-11-02 | Menu bar app (`LSUIElement`, Accessory activation policy); indicator is a separate non-focusable, always-on-top window, top-center | Indicator must not activate the app or steal focus |
| 2026-11-02 | Until Milestone 3, results go to the **recovery** path (main window shows the text) rather than claiming insertion | Honest status; reuses the spec's recovery flow |
| 2026-11-02 | Bundle ad-hoc signed as `com.fcitradecraft.loqwat` (`signingIdentity "-"`), hardened runtime, `com.apple.security.device.audio-input` entitlement, `NSMicrophoneUsageDescription` | Without the entitlement the hardened runtime blocks the mic; without the description macOS kills the app |
| 2026-11-02 | Cancel uses our own `extern "C"` abort trampoline instead of whisper-rs `set_abort_callback_safe` | whisper-rs 0.16 bug: the trampoline casts `Box<Box<dyn FnMut>>` to `F`, reads garbage, and aborts encoding (error -6) |
| 2026-11-02 | Insertion: capture app/window/focused element (AX) at start; revalidate with `CFEqual` before paste; refuse on change, secure field, own app, or missing permission → recovery window | Spec §5: never guess the destination |
| 2026-11-02 | Paste: save every pasteboard item/type, write text + `org.nspasteboard.TransientType`, ⌘V via CGEvent after modifiers are released; confirm via `AXNumberOfCharacters` change (≤1.5 s); otherwise "unverified" after a 400 ms settle; restore only if `changeCount` is still ours | Faithful restore without clobbering a user copy; the 400 ms settle is empirical, not a guarantee (documented limit) |
| 2026-11-03 | Settings: versioned JSON at `<app-data>/com.fcitradecraft.loqwat/settings.json`, atomic write; unparseable file moved to `settings.json.bad`, defaults used; unknown fields ignored | Spec §2 SettingsStore; a corrupt file must never block startup |
| 2026-11-03 | Shortcut is configurable; validated (needs ⌥/⌃/⌘ or an F-key; Escape and ⌘V/C/X/Z/Q/W/Tab/Space refused); new one test-registered before saving; old one restored on conflict | Spec §4: configurable shortcuts with clear conflict errors |
| 2026-11-03 | **Pause unregisters the shortcut** (and so does recording a new shortcut in the UI) | Frees the key combination for other apps; fixes the M2 open item |
| 2026-11-03 | Model manager: download (progress, cancel, verified, atomic), remove (not the one in use), switch (engine reload), **import only files that exactly match a catalog entry** (size + SHA-256) | Imports are as trustworthy as downloads; arbitrary files would bypass integrity checks |
| 2026-11-03 | Speech detector (VAD) is downloaded automatically with the first model and can't be removed in the UI | It prevents invented text on silence |
| 2026-11-03 | Runtime config is live: microphone and vocabulary apply at the next recording; a model switch drops the old model before loading the new one | No restart needed; avoids holding two models in memory |
| 2026-11-03 | Launch at login via `tauri-plugin-autostart` 2.7 (LaunchAgent), opt-in, off by default; file dialog via `tauri-plugin-dialog` 2.8 (Rust side only, no JS permissions) | Spec §6; LaunchAgent points at the app's current path, so it must be re-enabled if the app moves |
| 2026-11-03 | Sound cues (off by default): system sounds Tink (start), Pop (stop), Bottle (cancel) | Spec §4 optional cues; the start cue may be faintly captured in the recording |
| 2026-11-03 | Main window hidden at launch, shown only when setup is incomplete or no model is installed | A menu bar app that launches at login shouldn't pop a window every time |
| 2026-11-07 | **Number style** (user rule): one–nine spelled out; 10 and up as digits with thousands commas. Digits kept for money/percent, decimals/times/dates, and after labels (Rule 3, Section 5, page 2, Tier 1, Form…); 1st–9th → first–ninth; no commas in years 1900–2099 or ID-like numbers (Form 8300, case 123456); "3-day" → "three-day" | User's style choice; exceptions chosen by the user 2026-11-07 |
| 2026-11-07 | **Spoken punctuation**: "comma", "colon", "semicolon", "question mark", "exclamation point/mark" always become the mark, replacing any mark whisper added next to the word | User bug: the word *and* the mark both appeared ("Hello, comma, world") |
| 2026-11-07 | "period" → "." **only at a sentence end** (end of dictation, whisper put .?! after it, or a capitalized word follows); otherwise it stays a word | "period" is a common word in AML text (lookback/review period). Limitation: a sentence that really ends in the word needs "period period" |
| 2026-11-07 | Quotes and parentheses: "open/begin/start quote", "close/end quote", "unquote"; "open/left paren(thesis)", "close/right paren(thesis)" ("parent" accepted as a mishearing). A bare "quote" is not a command. Whisper's sentence end after a close goes inside a quote (`done."`) and after a parenthesis (`(draft).`) | User asked how to dictate quotes/parentheses; "quote" alone is common in finance ("get a quote") |
| 2026-11-07 | Voice samples: `examples/record_samples` reads passages from `scripts/voice-prompts.txt` (committed, no personal data), saves WAV + reference to git-ignored `recordings/voice/`; `bench --format` checks each result exactly against `<id>.expected.txt` | Measure accuracy on the user's real voice instead of TTS; WER ignores punctuation, so formatting needs an exact check |
| 2026-11-07 | Whisper prompt = user vocabulary + fixed punctuation hint (not shown in the vocabulary UI); `bench --format` uses the same prompt | Measured on the user's voice: small models stopped dropping spoken commands |
| 2026-11-07 | Keep `small.en-q5_1` as default after real-voice test | Same accuracy as turbo on this voice, 2.5× faster |
| 2026-11-07 | Formatting runs in `format.rs` after `clean_transcript`, on whole tokens only; anything not matching a rule is left as whisper wrote it | Predictable; easy to test |

## 4. Milestones

Status key: ⬜ not started · 🟨 in progress · ✅ done · 🧪 tested (on real OS) · ⛔ blocked

| # | Milestone | Spec § | Status |
|---|---|---|---|
| 0 | Repo + toolchain + Tauri scaffold | 1–2 | ✅ 🧪 release .app built and launched on dev Mac |
| 1 | Prove transcription (file → mic), benchmark models | 3 | ✅ 🧪 file + live mic verified on dev Mac (accuracy pending user confirmation) |
| 2 | Recording lifecycle / state machine / indicator | 4 | ✅ 🧪 user-tested on dev Mac (sleep/wake + sound cues deferred) |
| 3 | Mac text insertion + clipboard + recovery window | 5 | ✅ built, clipboard tests pass · in daily use, "working well" (2026-11-07); full checklist not run |
| 4 | First-run setup, model manager, preferences | 6 | ✅ built, unit tests pass · in daily use, "working well" (2026-11-07); full checklist not run |
| 5 | Formatting, vocabulary hints, privacy/resource bounds | 7–8 | 🟨 spoken punctuation + number style built (unit tests); new line/paragraph, idle unload, bounds to do |
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
- ~~Numbers are written as digits; confirm the user is OK with it.~~ Resolved 2026-11-07: user set a number style
  (see §3), now applied by `format.rs`. Not handled yet: numbers 10+ that whisper spells out as words ("twelve")
  stay as words. Whisper rarely does this.

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

### 2026-11-02 — Milestone 2 (M1 Pro, macOS 27.0.1, release .app launched via `open`)
| Check | Result |
|---|---|
| Unit tests (controller 17 + earlier 12) | ✅ 29 pass |
| Integration test `tests/engine_cancel.rs` (real model, cancel flag unset/set/after) | ✅ pass |
| App launch | ✅ engine ready, load 203–404 ms + warm-up ~230 ms; idle RSS ~443 MB, CPU ~0.1% |
| ⌥Space registration | ✅ no conflict reported |
| Microphone prompt for LoqWat itself | ✅ appeared (UserNotificationCenter) and was granted |
| **Focus:** 26 s recording started from Microsoft Word | ✅ Word stayed frontmost for the whole recording (frontmost-app log, 250 ms polling via `lsappinfo`) |
| First in-app dictations (25.7 s, 12.5 s) | ❌ whisper error -6 → root-caused to whisper-rs abort callback → fixed in `18d896e` |
| **Retest (fixed build, Yeti mic, from TextEdit)** | ✅ 27.9 s → 271 chars in 784 ms inference / 910 ms total; 2–6 s clips 267–316 ms total |
| Focus during retest | ✅ TextEdit stayed frontmost through the 28 s recording + transcription; LoqWat came forward only for the result window (by design until Milestone 3) |
| Esc cancel | ✅ session 5 "capture discarded" |
| Silence | ✅ sessions 3 and 6 → "no speech (VAD)"; nothing invented |
| Key repeat (hold ⌥Space ~3 s, toggle mode) | ✅ one session; no rapid duplicate sessions in the log |
| Hold-to-talk mode | ✅ user-reported working (session 7, 2.7 s) |

### 2026-11-02 — Milestone 3 (automated only)
| Check | Result |
|---|---|
| Pasteboard save/restore on a private named pasteboard: multiple items + types (text, RTF, PNG bytes) | ✅ restored byte-for-byte |
| Empty pasteboard restore | ✅ |
| `changeCount` detects a user copy made after our write (restore skipped) | ✅ |
| Unit tests total / clippy | ✅ 34 pass / clean |
| Release app launch | ✅ engine ready (load 258 ms, warm-up 269 ms) |

**User test checklist (pending):** grant Accessibility; dictate into TextEdit (text appears at caret); copy text first and
check ⌘V afterwards gives the original; switch apps mid-transcription → recovery window; password field → refused;
Copy / Insert into a field… / Discard; Word, Mail, Gmail in Chrome (may report "can't confirm").

### 2026-11-03 — Milestone 4 (automated only)
| Check | Result |
|---|---|
| Unit tests (+ settings 8, shortcut validation 2, model import/remove 2) | ✅ 46 pass |
| `cargo clippy --all-targets`, `tsc --noEmit` | ✅ clean |
| Release `.app` | ✅ 9.0 MiB (was 4.3; added autostart + dialog plugins) |
| Launch with no settings file | ✅ shortcut registered from defaults, engine ready (load 397 ms, warm-up 268 ms) |
| UI rendering | ⚠️ not seen by Claude (no screen-recording permission); needs the user's check |

**User test checklist (pending):** setup window appears on first launch; microphone list + Test (meter moves, "heard you");
pick another mic and dictate; change shortcut (e.g. ⌃⇧D) and use it; try a taken/invalid one → clear error, old one still works;
Pause → ⌥Space free for other apps, Unpause → works again; edit vocabulary + Save; sound cues; launch at login on/off
(check System Settings → General → Login Items); Models: download base.en (progress, cancel, retry), Use it, switch back,
Remove it; import a model file; quit and relaunch → settings kept, no setup window.

### 2026-11-07 — Milestone 5 formatting (automated only)
| Check | Result |
|---|---|
| `format.rs` unit tests (spoken punctuation, "period" rule, capitals, abbreviations, number style, exceptions, years/IDs) | ✅ 10 pass |
| All unit tests / `cargo clippy --all-targets` | ✅ 56 pass / clean |
| Release `.app` rebuilt | ✅ 9.03 MiB |

### 2026-11-07 — Quotes/parentheses + voice-sample kit (automated only)
| Check | Result |
|---|---|
| Quote/parenthesis unit tests | ✅ 2 new (12 formatting tests total) |
| Prompt file parses; each punctuation passage's `expect` matches the formatter on a perfect transcript | ✅ (caught one passage, "draft period I will", where the "I" rule made it ambiguous; reworded) |
| All tests / clippy | ✅ 58 lib + 2 example tests pass / clean |
| Release `.app` rebuilt; `record_samples` + `bench` precompiled | ✅ |

**Voice samples (pending, user):** in a normal Terminal window (not Claude Code's `!`, which has no keyboard input):
`cd LoqWat/src-tauri && cargo run --release --example record_samples`. Seven passages, about 4 minutes. Terminal needs
microphone permission. Then score with
`cargo run --release --example bench -- --models small.en-q5_1 --vad --format --show-text --prompt "FCI Tradecraft, FinCEN, BSA, AML, KYC, OFAC." ../recordings/voice/*.wav`.

### 2026-11-07 — Real-voice results (user's voice, Yeti mic, 7 passages, ~2.5 min)
Exact = formatted output identical to the passage's `expect` line (5 punctuation passages). WER on the 42 s natural read.

| Model | Exact (before fixes) | Exact (after) | Natural WER | Warm, 42 s clip |
|---|---|---|---|---|
| small.en-q5_1 (default) | 1/5 | **3/5** | 4.4% | 0.96 s |
| small.en | 1/5 | — | 4.4% | 1.05 s |
| base.en | 0/5 | — | 32.5% (dropped the last third) | 0.50 s |
| large-v3-turbo-q5_0 | 2/5 | 3/5 | 3.5% | 2.38 s |

- **Every model** heard "and" as "in" (02) and "date" as "data" (05). These are pronunciation or audio, not formatting.
- **Before the fixes**, small models dropped "exclamation point" and "semicolon" entirely and wrote "open, parent".
  Appending the punctuation words to the prompt (`format::PUNCTUATION_HINT`) fixed both. Plain speech (06) gained no
  invented punctuation words.
- whisper segment breaks: "going to... ...retune" and "20... 26" → now rejoined. "1.08 p.m." / "108 p.m." → "1:08 p.m.".
- Comma whisper put before "open paren" → removed (kept before an opening quote).
- "look back" → "lookback" once "lookback" is in the vocabulary (now a default; existing users must add it).
- **OFAC** in natural speech: "The whole fact" (small), "Sofac" (turbo), even with OFAC in the prompt. Not fixed.
- Free talk: the user's own "open quote / close quote / end quote" were applied correctly; name "Rio Miner" correct on small
  (base.en: "Minor").
- Decision: keep **small.en-q5_1**. large-turbo is no more accurate on this voice and is 2.5× slower; base.en is unreliable.

**User test (pending):** dictate "Hello comma how are you question mark"; "The lookback period ends Friday period";
"We filed 3 SARs and 1200 CTRs in 2026"; "Rule 3 applies to 5 percent".
M3/M4: user reports normal use is working well; the full checklists above have not been run.

Frontmost-app logger used for focus checks:
```zsh
prev=""; while true; do cur=$(lsappinfo info -only name "$(lsappinfo front)" | head -1 | sed -E 's/^"([^"]*)".*/\1/');
[ "$cur" != "$prev" ] && echo "$(date +%T) $cur"; prev=$cur; sleep 0.25; done
```

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

#### Milestone 2
- Added `controller.rs` (state machine), `runtime.rs` (audio/engine threads, Hooks trait), rewrote `lib.rs`
  (tray menu, ⌥Space + Escape shortcuts, indicator window, status/recovery main window, commands), UI pages
  `index.html` + `indicator.html`, `Info.plist`, `Entitlements.plist`.
- Fixed the log filter (crate is `loqwat_lib`, not `loqwat`).
- Bundle identifier was linker-generated (`loqwat-ef71…`); switched to ad-hoc signing with the real bundle ID.
  This turned on the hardened runtime, so added the audio-input entitlement.
- First user test: the indicator didn't steal focus ✅; transcription failed with whisper error -6 → fixed
  (whisper-rs abort-callback bug), regression test added.
- **Bookmarked here** before the user restarted. See "RESUME HERE" at the top.
- **Resumed after the Claude Code restart.** User retest passed (see §6) → Milestone 2 ✅. The user noted the text
  doesn't go into TextEdit; that's expected until Milestone 3.

### 2026-11-03
- Built Milestone 3 (see §3 decisions, §6 results); committed `efd50e0`. User asked about a phone version; decided against
  it (Android keyboard dictation already covers it). User chose to start Milestone 4 before testing M3.
#### Milestone 4
- New modules: `settings.rs` (versioned store, vocabulary normalization: trim, dedupe, ≤40 terms, ≤40 chars each, ≤400 chars total),
  `shortcut.rs` (validation). `models.rs`: `import` (catalog match only), `remove`. `runtime.rs`: live config,
  `reload_engine`, model resolution from settings. `platform`: sound cues, reveal folder.
- `lib.rs`: settings-driven startup, shortcut sync (pause releases it), 18 new commands (microphones + test, shortcut capture/set,
  vocabulary, cues, login item, setup, models list/download/cancel/use/remove/import/reveal).
- UI: tabs Status / Setup / Preferences / Models; setup checklist (mic test, Accessibility, model download, shortcut).
- Clearer message when the chosen microphone is disconnected.

### 2026-11-07
- Confirmed Rust PATH fix is in `~/.zshrc` (cargo/rustc 1.99.1 in a new shell); marked done in §2.
- User feedback: M3/M4 working well in daily use (full checklists not run); punctuation words appear along with the mark.
- User set the number style and, when asked, chose the exceptions and the "period" rule (§3).
- Added `format.rs` (spoken punctuation + number style), called from `runtime::run_job` after transcription; 10 unit tests.
- Release `.app` rebuilt; awaiting the user's real-voice check.
- Accessibility stopped working after the rebuild (ad-hoc signature changed); fixed with
  `tccutil reset Accessibility com.fcitradecraft.loqwat` + re-grant. Committed and pushed `8ffe1f3`.
- User: dictation "pretty good"; asked how to get quotation marks and parentheses → added the commands (§3).
- User agreed to record voice samples → added `scripts/voice-prompts.txt`, `examples/record_samples.rs`, `bench --format`.
- User recorded the 7 passages. Scored all 4 models (§6). Added the punctuation hint to the prompt, segment-break rejoin,
  clock-time fix, no comma before "(", "parren", default vocabulary + "lookback". 61 lib + 2 example tests, clippy clean;
  `.app` rebuilt.
