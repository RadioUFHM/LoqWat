import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Phase = "idle" | "recording" | "transcribing" | "inserting" | "recovery" | "error";
type Engine =
  | { state: "not_loaded" | "loading" | "ready" }
  | { state: "unavailable"; detail: string };

interface Settings {
  setup_complete: boolean;
  shortcut: string;
  mode: "toggle" | "hold";
  input_device: string | null;
  model_id: string | null;
  vocabulary: string[];
  sound_cues: boolean;
  launch_at_login: boolean;
}

interface UiState {
  phase: Phase;
  session: number;
  mode: "toggle" | "hold";
  paused: boolean;
  engine: Engine;
  message: string | null;
  limit_reached: boolean;
  recovery_text: string | null;
  settings: Settings;
  shortcut_error: string | null;
  model_id: string | null;
  model: string | null;
  vad_installed: boolean;
  version: string;
  accessibility: boolean;
  has_last_text: boolean;
  launch_at_login: boolean;
}

interface ModelRow {
  id: string;
  label: string;
  size: number;
  installed: boolean;
  active: boolean;
  recommended: boolean;
}

interface ModelsView {
  dir: string;
  used_bytes: number;
  models: ModelRow[];
  vad: ModelRow;
  download: { id: string; done: number; total: number } | null;
  source: string;
}

interface DownloadEvent {
  id: string;
  done: number;
  total: number;
  finished: "ok" | "failed" | "cancelled" | null;
  error: string | null;
}

const RECOMMENDED_MODEL = "small.en-q5_1";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const all = <T extends HTMLElement>(sel: string) => Array.from(document.querySelectorAll<T>(sel));

let state: UiState | null = null;
let models: ModelsView | null = null;
let download: { id: string; done: number; total: number } | null = null;
let micTested = false;
let recordingShortcut = false;

const PHASE_LABEL: Record<Phase, string> = {
  idle: "Ready",
  recording: "Listening…",
  transcribing: "Transcribing…",
  inserting: "Inserting…",
  recovery: "Text ready",
  error: "Error",
};

function engineLabel(e: Engine): string {
  switch (e.state) {
    case "ready": return "Ready";
    case "loading": return "Loading model…";
    case "not_loaded": return "Not loaded";
    case "unavailable": return `Unavailable: ${e.detail}`;
  }
}

const mb = (bytes: number) => `${Math.round(bytes / 1_000_000).toLocaleString()} MB`;

/** "Control+Shift+KeyD" → "⌃ ⇧ D" */
export function prettyShortcut(acc: string): string {
  const MOD: Record<string, string> = {
    control: "⌃", ctrl: "⌃", alt: "⌥", option: "⌥", shift: "⇧",
    super: "⌘", cmd: "⌘", command: "⌘", commandorcontrol: "⌘", cmdorctrl: "⌘",
  };
  const KEY: Record<string, string> = {
    Space: "Space", Enter: "Return", Tab: "Tab", Backspace: "Delete",
    ArrowUp: "↑", ArrowDown: "↓", ArrowLeft: "←", ArrowRight: "→",
    Backquote: "`", Minus: "-", Equal: "=", BracketLeft: "[", BracketRight: "]",
    Backslash: "\\", Semicolon: ";", Quote: "'", Comma: ",", Period: ".", Slash: "/",
  };
  return acc
    .split("+")
    .map((p) => {
      const m = MOD[p.toLowerCase()];
      if (m) return m;
      if (/^Key[A-Z]$/.test(p)) return p.slice(3);
      if (/^Digit\d$/.test(p)) return p.slice(5);
      return KEY[p] ?? p;
    })
    .join(" ");
}

/** Build a Tauri accelerator from a keydown, or null for a bare modifier. */
function acceleratorFrom(e: KeyboardEvent): string | null {
  if (["ShiftLeft", "ShiftRight", "AltLeft", "AltRight", "ControlLeft", "ControlRight",
       "MetaLeft", "MetaRight", "CapsLock", "Fn"].includes(e.code)) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Control");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Super");
  parts.push(e.code);
  return parts.join("+");
}

// ------------------------------------------------------------------ tabs

function showTab(name: string) {
  for (const b of all<HTMLButtonElement>("[data-tab]")) {
    b.setAttribute("aria-selected", String(b.dataset.tab === name));
  }
  for (const p of all<HTMLElement>("[data-panel]")) p.hidden = p.dataset.panel !== name;
  if (name === "models" || name === "setup") refreshModels();
  if (name === "prefs" || name === "setup") refreshMics();
}

// ------------------------------------------------------------------ render

function render(s: UiState) {
  const first = state === null;
  state = s;
  const phase = $("phase");
  phase.textContent = s.paused ? "Paused" : PHASE_LABEL[s.phase];
  phase.dataset.phase = s.paused ? "paused" : s.phase;

  if (!recordingShortcut) {
    for (const k of all("kbd.shortcut-kbd")) k.textContent = prettyShortcut(s.settings.shortcut);
  }
  for (const el of all(".model-label")) el.textContent = s.model ?? "none installed";
  for (const el of all(".engine-label")) el.textContent = engineLabel(s.engine);
  for (const el of all(".shortcut-error")) {
    el.hidden = !s.shortcut_error;
    el.textContent = s.shortcut_error ?? "";
  }
  for (const r of all<HTMLInputElement>('input[name="mode"]')) r.checked = r.value === s.mode;
  for (const c of all<HTMLInputElement>("input.paused")) c.checked = s.paused;

  const toggle = $<HTMLButtonElement>("toggle");
  toggle.textContent = s.phase === "recording" ? "Stop Dictation" : "Start Dictation";
  toggle.disabled = s.paused || s.phase === "transcribing" || s.phase === "inserting";

  for (const el of all(".ax-card")) el.hidden = s.accessibility;

  // Result / recovery card
  const card = $("result-card");
  const recovering = s.phase === "recovery";
  card.hidden = !(recovering || s.phase === "error" || !!s.message);
  $("result-title").textContent =
    s.phase === "error" ? "Error" : recovering ? "Text not inserted" : "Status";
  $("message").textContent = [
    s.message,
    s.limit_reached ? "Recording reached the 5-minute limit; captured audio was processed." : null,
  ].filter(Boolean).join(" ");
  const result = $<HTMLTextAreaElement>("result");
  result.hidden = !s.recovery_text;
  result.value = s.recovery_text ?? "";
  $("recovery-actions").hidden = !(recovering || s.phase === "error");
  $("copy").hidden = !recovering;
  $("insert-elsewhere").hidden = !recovering;
  $("clip-note").hidden = !recovering;
  $("dismiss").textContent = recovering ? "Discard" : "OK";
  if (!recovering) $("countdown").hidden = true;
  if (recovering || s.phase === "error") showTab("status");

  // Preferences
  $<HTMLInputElement>("sound-cues").checked = s.settings.sound_cues;
  $<HTMLInputElement>("login").checked = s.launch_at_login;
  const vocab = $<HTMLTextAreaElement>("vocab");
  if (first || document.activeElement !== vocab) vocab.value = s.settings.vocabulary.join("\n");

  renderSetup();
  $("version").textContent = `LoqWat v${s.version}`;
  if (first) showTab(s.settings.setup_complete && s.model_id ? "status" : "setup");
}

function renderSetup() {
  if (!state) return;
  const s = state;
  const mark = (id: string, done: boolean) => {
    const step = $(id);
    step.classList.toggle("done", done);
    step.querySelector(".check")!.textContent = done ? "✓" : "○";
  };
  mark("step-mic", micTested);
  mark("step-ax", s.accessibility);
  all(".ax-status").forEach((el) => (el.textContent = s.accessibility ? "Allowed" : "Not allowed yet"));
  all<HTMLButtonElement>("#step-ax .grant-ax").forEach((b) => (b.hidden = s.accessibility));
  mark("step-model", !!s.model_id);
  mark("step-shortcut", !s.shortcut_error);

  const text = document.querySelector<HTMLElement>(".setup-model-text")!;
  const btn = $<HTMLButtonElement>("setup-download");
  const dlActive = download !== null;
  if (s.model_id) {
    text.textContent = `Using ${s.model}. ${engineLabel(s.engine)}.`;
    btn.hidden = true;
  } else {
    text.textContent =
      "LoqWat needs a speech model (downloaded once, verified, stored on this Mac). Recommended: Small English, quantized.";
    btn.hidden = dlActive;
    btn.disabled = dlActive;
  }
  renderDownload();
}

function renderDownload() {
  const setupMeter = $("setup-dl-meter");
  const progress = document.querySelector<HTMLElement>(".setup-model-progress")!;
  for (const b of all<HTMLButtonElement>(".cancel-download")) b.hidden = !download;
  if (download) {
    const pct = download.total ? Math.floor((download.done / download.total) * 100) : 0;
    setupMeter.hidden = false;
    (setupMeter.firstElementChild as HTMLElement).style.width = `${pct}%`;
    progress.textContent = `Downloading ${download.id}: ${pct}% (${mb(download.done)} of ${mb(download.total)})`;
    const rowBar = document.querySelector<HTMLElement>(`[data-model="${download.id}"] .meter-fill`);
    if (rowBar) rowBar.style.width = `${pct}%`;
    const rowTxt = document.querySelector<HTMLElement>(`[data-model="${download.id}"] .dl-text`);
    if (rowTxt) rowTxt.textContent = `${pct}%`;
  } else {
    setupMeter.hidden = true;
    if (!progress.dataset.sticky) progress.textContent = "";
  }
}

// ------------------------------------------------------------------ models

async function refreshModels() {
  try {
    models = await invoke<ModelsView>("list_models");
  } catch (e) {
    showModelsError(String(e));
    return;
  }
  download = models.download;
  renderModels();
  renderSetup();
}

function showModelsError(msg: string | null) {
  const el = $("models-error");
  el.hidden = !msg;
  el.textContent = msg ?? "";
}

function button(label: string, onClick: () => void, cls = ""): HTMLButtonElement {
  const b = document.createElement("button");
  b.textContent = label;
  if (cls) b.className = cls;
  b.addEventListener("click", onClick);
  return b;
}

function modelRow(m: ModelRow, isVad: boolean): HTMLElement {
  const row = document.createElement("div");
  row.className = "model-row";
  row.dataset.model = m.id;
  const info = document.createElement("div");
  info.className = "model-info";
  const name = document.createElement("div");
  name.textContent = m.label + (m.recommended ? "  · recommended" : "");
  const meta = document.createElement("div");
  meta.className = "hint";
  meta.textContent = `${mb(m.size)} · ${m.installed ? (m.active ? "in use" : "installed") : "not installed"}`;
  info.append(name, meta);
  const actions = document.createElement("div");
  actions.className = "model-actions";
  const busy = download !== null;
  if (download?.id === m.id) {
    const meter = document.createElement("div");
    meter.className = "meter small";
    const fill = document.createElement("div");
    fill.className = "meter-fill";
    meter.append(fill);
    const txt = document.createElement("span");
    txt.className = "hint dl-text";
    actions.append(meter, txt, button("Cancel", () => invoke("cancel_download"), "cancel-download"));
  } else if (!m.installed) {
    const b = button("Download", () => startDownload(m.id));
    b.disabled = busy;
    actions.append(b);
  } else if (!isVad) {
    if (m.active) {
      const badge = document.createElement("span");
      badge.className = "badge";
      badge.textContent = "In use";
      actions.append(badge);
    } else {
      actions.append(
        button("Use", async () => {
          try {
            showModelsError(null);
            await invoke("use_model", { id: m.id });
            await refreshModels();
          } catch (e) { showModelsError(String(e)); }
        }),
        button("Remove", async function (this: void) {
          // Two-step confirm (native confirm() isn't reliable in the webview).
          const b = row.querySelector<HTMLButtonElement>("button.remove")!;
          if (b.dataset.armed !== "1") {
            b.dataset.armed = "1";
            b.textContent = "Remove? Click again";
            setTimeout(() => { b.dataset.armed = ""; b.textContent = "Remove"; }, 4000);
            return;
          }
          try {
            showModelsError(null);
            await invoke("remove_model", { id: m.id });
            await refreshModels();
          } catch (e) { showModelsError(String(e)); }
        }, "remove"),
      );
    }
  } else {
    const badge = document.createElement("span");
    badge.className = "badge";
    badge.textContent = "Installed";
    actions.append(badge);
  }
  row.append(info, actions);
  return row;
}

function renderModels() {
  if (!models) return;
  const list = $("model-list");
  list.replaceChildren();
  const head = document.createElement("strong");
  head.textContent = "Speech models (English)";
  list.append(head);
  for (const m of models.models) list.append(modelRow(m, false));
  const vadHead = document.createElement("strong");
  vadHead.textContent = "Speech detector";
  const vadHint = document.createElement("p");
  vadHint.className = "hint";
  vadHint.textContent = "Stops LoqWat from inventing text when you were silent. Downloaded with your first model.";
  list.append(vadHead, vadHint, modelRow(models.vad, true));
  $("models-usage").textContent = `${mb(models.used_bytes)} used in ${models.dir}`;
  $("models-source").textContent =
    `Downloads come from ${models.source} (a fixed version). Each file's SHA-256 checksum is verified before LoqWat uses it.`;
  renderDownload();
}

async function startDownload(id: string) {
  try {
    showModelsError(null);
    const p = document.querySelector<HTMLElement>(".setup-model-progress")!;
    delete p.dataset.sticky;
    await invoke("download_model", { id });
    download = { id, done: 0, total: models?.models.find((m) => m.id === id)?.size ?? 0 };
    renderModels();
    renderSetup();
  } catch (e) {
    showModelsError(String(e));
  }
}

function onDownloadEvent(ev: DownloadEvent) {
  if (ev.finished) {
    download = null;
    const p = document.querySelector<HTMLElement>(".setup-model-progress")!;
    if (ev.finished === "failed") {
      const msg = `Download failed: ${ev.error ?? "unknown error"}. Nothing was saved; you can try again.`;
      showModelsError(msg);
      p.textContent = msg;
      p.dataset.sticky = "1";
    } else if (ev.finished === "cancelled") {
      p.textContent = "Download cancelled.";
      p.dataset.sticky = "1";
    } else {
      p.textContent = "Downloaded and verified.";
      p.dataset.sticky = "1";
    }
    refreshModels();
    return;
  }
  download = { id: ev.id, done: ev.done, total: ev.total };
  if (!document.querySelector(`[data-model="${ev.id}"] .meter`)) renderModels();
  renderDownload();
}

// ------------------------------------------------------------------ microphone

async function refreshMics() {
  let mics: { name: string; is_default: boolean }[] = [];
  try {
    mics = await invoke("list_microphones");
  } catch (e) {
    all(".mic-result").forEach((el) => (el.textContent = String(e)));
  }
  const chosen = state?.settings.input_device ?? "";
  for (const sel of all<HTMLSelectElement>("select.mic-select")) {
    sel.replaceChildren();
    const def = document.createElement("option");
    const sysDefault = mics.find((m) => m.is_default)?.name;
    def.value = "";
    def.textContent = `System default${sysDefault ? ` (${sysDefault})` : ""}`;
    sel.append(def);
    for (const m of mics) {
      const o = document.createElement("option");
      o.value = m.name;
      o.textContent = m.name;
      sel.append(o);
    }
    if (chosen && !mics.some((m) => m.name === chosen)) {
      const o = document.createElement("option");
      o.value = chosen;
      o.textContent = `${chosen} (not connected)`;
      sel.append(o);
    }
    sel.value = chosen;
  }
}

async function testMic() {
  const buttons = all<HTMLButtonElement>("button.mic-test");
  buttons.forEach((b) => (b.disabled = true));
  all(".mic-result").forEach((el) => (el.textContent = "Listening for 3 seconds. Say something…"));
  try {
    const r = await invoke<{ device: string; peak: number; heard: boolean }>("test_microphone");
    micTested = r.heard;
    const msg = r.heard
      ? `✓ Heard you on ${r.device}. (The test audio was discarded.)`
      : `Didn't hear anything on ${r.device}. Check that it's not muted, pick another microphone, or allow LoqWat under System Settings → Privacy & Security → Microphone.`;
    all(".mic-result").forEach((el) => (el.textContent = msg));
  } catch (e) {
    all(".mic-result").forEach((el) => (el.textContent = String(e)));
  } finally {
    buttons.forEach((b) => (b.disabled = false));
    all<HTMLElement>(".meter-fill.mic").forEach((f) => (f.style.width = "0%"));
    renderSetup();
  }
}

// ------------------------------------------------------------------ shortcut

function beginShortcutCapture() {
  if (recordingShortcut) return;
  recordingShortcut = true;
  invoke("capture_shortcut", { active: true });
  for (const k of all("kbd.shortcut-kbd")) k.textContent = "Press the new shortcut…";
  for (const m of all(".shortcut-msg")) m.textContent = "Press the keys together, e.g. ⌥ Space or ⌃ ⇧ D. Esc cancels.";
  const onKey = async (e: KeyboardEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.code === "Escape" && !e.altKey && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
      finish("Unchanged.");
      return;
    }
    const acc = acceleratorFrom(e);
    if (!acc) return; // wait for a non-modifier key
    try {
      const saved = await invoke<string>("set_shortcut", { shortcut: acc });
      finish(`Saved: ${prettyShortcut(saved)}.`);
    } catch (err) {
      finish(String(err));
    }
  };
  const finish = (msg: string) => {
    window.removeEventListener("keydown", onKey, true);
    recordingShortcut = false;
    if (state) for (const k of all("kbd.shortcut-kbd")) k.textContent = prettyShortcut(state.settings.shortcut);
    for (const m of all(".shortcut-msg")) m.textContent = msg;
    invoke("capture_shortcut", { active: false });
  };
  window.addEventListener("keydown", onKey, true);
}

// ------------------------------------------------------------------ wiring

window.addEventListener("DOMContentLoaded", async () => {
  for (const b of all<HTMLButtonElement>("[data-tab]")) {
    b.addEventListener("click", () => showTab(b.dataset.tab!));
  }

  // Status
  $("toggle").addEventListener("click", () => invoke("toggle_dictation"));
  $("dismiss").addEventListener("click", () => invoke("dismiss"));
  $("copy").addEventListener("click", () => invoke("copy_recovery"));
  $("insert-elsewhere").addEventListener("click", () => {
    $("countdown").hidden = false;
    invoke("insert_recovery");
  });
  for (const b of all(".grant-ax")) b.addEventListener("click", () => invoke("grant_accessibility"));
  for (const c of all<HTMLInputElement>("input.paused")) {
    c.addEventListener("change", () => invoke("set_paused", { paused: c.checked }));
  }
  for (const r of all<HTMLInputElement>('input[name="mode"]')) {
    r.addEventListener("change", () => invoke("set_mode", { mode: r.value }));
  }

  // Microphone
  for (const sel of all<HTMLSelectElement>("select.mic-select")) {
    sel.addEventListener("change", async () => {
      micTested = false;
      await invoke("set_microphone", { name: sel.value || null });
      for (const other of all<HTMLSelectElement>("select.mic-select")) other.value = sel.value;
      all(".mic-result").forEach((el) => (el.textContent = "Microphone changed. Click Test to check it."));
      renderSetup();
    });
  }
  for (const b of all("button.mic-test")) b.addEventListener("click", testMic);
  for (const m of all(".meter-fill")) if (m.closest(".card")?.querySelector(".mic-test")) m.classList.add("mic");

  // Shortcut
  for (const b of all("button.shortcut-change")) b.addEventListener("click", beginShortcutCapture);

  // Setup
  $("setup-download").addEventListener("click", () => startDownload(RECOMMENDED_MODEL));
  $("setup-done").addEventListener("click", async () => {
    await invoke("finish_setup");
    showTab("status");
  });
  $("setup-later").addEventListener("click", () => showTab("status"));
  for (const b of all(".cancel-download")) b.addEventListener("click", () => invoke("cancel_download"));

  // Preferences
  $("vocab-save").addEventListener("click", async () => {
    const terms = $<HTMLTextAreaElement>("vocab").value.split("\n");
    const saved = await invoke<string[]>("set_vocabulary", { terms });
    $<HTMLTextAreaElement>("vocab").value = saved.join("\n");
    $("vocab-msg").textContent = `Saved ${saved.length} term${saved.length === 1 ? "" : "s"}. Used from your next dictation.`;
  });
  $("vocab-reset").addEventListener("click", async () => {
    const saved = await invoke<string[]>("reset_vocabulary");
    $<HTMLTextAreaElement>("vocab").value = saved.join("\n");
    $("vocab-msg").textContent = "Reset to the default list.";
  });
  $<HTMLInputElement>("sound-cues").addEventListener("change", (e) =>
    invoke("set_sound_cues", { enabled: (e.target as HTMLInputElement).checked }),
  );
  $<HTMLInputElement>("login").addEventListener("change", async (e) => {
    const box = e.target as HTMLInputElement;
    const err = $("login-error");
    try {
      box.checked = await invoke<boolean>("set_launch_at_login", { enabled: box.checked });
      err.hidden = true;
    } catch (x) {
      box.checked = !box.checked;
      err.hidden = false;
      err.textContent = String(x);
    }
  });
  $("quit").addEventListener("click", () => invoke("quit"));

  // Models
  $("models-reveal").addEventListener("click", () => invoke("reveal_models_folder").catch((e) => showModelsError(String(e))));
  $("models-import").addEventListener("click", async () => {
    showModelsError(null);
    try {
      const label = await invoke<string | null>("import_model");
      if (label) $("models-usage").textContent = `Imported ${label}.`;
      await refreshModels();
    } catch (e) {
      showModelsError(String(e));
    }
  });

  // Events
  await listen<UiState>("state", (e) => render(e.payload));
  await listen<DownloadEvent>("download", (e) => onDownloadEvent(e.payload));
  await listen<number>("mic-level", (e) => {
    const pct = Math.min(100, Math.round(Math.sqrt(e.payload) * 140));
    all<HTMLElement>(".meter-fill.mic").forEach((f) => (f.style.width = `${pct}%`));
  });
  // Permission may be granted while the window is open; re-check on focus.
  window.addEventListener("focus", async () => {
    render(await invoke<UiState>("get_state"));
    refreshMics();
  });

  render(await invoke<UiState>("get_state"));
  refreshMics();
  refreshModels();
});
