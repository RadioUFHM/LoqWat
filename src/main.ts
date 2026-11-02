import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Phase = "idle" | "recording" | "transcribing" | "inserting" | "recovery" | "error";
type Engine =
  | { state: "not_loaded" | "loading" | "ready" }
  | { state: "unavailable"; detail: string };

interface UiState {
  phase: Phase;
  session: number;
  mode: "toggle" | "hold";
  paused: boolean;
  engine: Engine;
  message: string | null;
  limit_reached: boolean;
  recovery_text: string | null;
  shortcut: string;
  shortcut_error: string | null;
  model: string | null;
  version: string;
  accessibility: boolean;
  has_last_text: boolean;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

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
    case "unavailable": return `Unavailable — ${e.detail}`;
  }
}

function render(s: UiState) {
  const phase = $("phase");
  phase.textContent = s.paused ? "Paused" : PHASE_LABEL[s.phase];
  phase.dataset.phase = s.paused ? "paused" : s.phase;

  $("shortcut").textContent = s.shortcut.replace("Alt", "⌥ Option").replace("+", " + ");
  $("model").textContent = s.model ?? "none installed";
  $("engine").textContent = engineLabel(s.engine);
  const se = $("shortcut-error");
  se.hidden = !s.shortcut_error;
  se.textContent = s.shortcut_error ?? "";

  for (const r of document.querySelectorAll<HTMLInputElement>('input[name="mode"]')) {
    r.checked = r.value === s.mode;
  }
  $<HTMLInputElement>("paused").checked = s.paused;

  const toggle = $<HTMLButtonElement>("toggle");
  toggle.textContent = s.phase === "recording" ? "Stop Dictation" : "Start Dictation";
  toggle.disabled = s.paused || s.phase === "transcribing" || s.phase === "inserting";

  $("ax-card").hidden = s.accessibility;

  const card = $("result-card");
  const recovering = s.phase === "recovery";
  card.hidden = !(recovering || s.phase === "error" || !!s.message);
  $("result-title").textContent =
    s.phase === "error" ? "Error" : recovering ? "Text not inserted" : "Status";
  const msg = [s.message, s.limit_reached ? "Recording reached the 5-minute limit; captured audio was processed." : null]
    .filter(Boolean)
    .join(" ");
  $("message").textContent = msg;
  const result = $<HTMLTextAreaElement>("result");
  result.hidden = !s.recovery_text;
  result.value = s.recovery_text ?? "";
  $("recovery-actions").hidden = !(recovering || s.phase === "error");
  $("copy").hidden = !recovering;
  $("insert-elsewhere").hidden = !recovering;
  $("clip-note").hidden = !recovering;
  $("dismiss").textContent = recovering ? "Discard" : "OK";
  if (!recovering) $("countdown").hidden = true;

  $("version").textContent = `LoqWat v${s.version}`;
}

window.addEventListener("DOMContentLoaded", async () => {
  $("toggle").addEventListener("click", () => invoke("toggle_dictation"));
  $("dismiss").addEventListener("click", () => invoke("dismiss"));
  $("copy").addEventListener("click", () => invoke("copy_recovery"));
  $("insert-elsewhere").addEventListener("click", () => {
    $("countdown").hidden = false;
    invoke("insert_recovery");
  });
  $("grant-ax").addEventListener("click", async () => {
    await invoke("grant_accessibility");
  });
  // Permission may be granted while the window is open; re-check on focus.
  window.addEventListener("focus", async () => render(await invoke<UiState>("get_state")));
  $<HTMLInputElement>("paused").addEventListener("change", (e) =>
    invoke("set_paused", { paused: (e.target as HTMLInputElement).checked }),
  );
  for (const r of document.querySelectorAll<HTMLInputElement>('input[name="mode"]')) {
    r.addEventListener("change", () => invoke("set_mode", { mode: r.value }));
  }
  await listen<UiState>("state", (e) => render(e.payload));
  render(await invoke<UiState>("get_state"));
});
