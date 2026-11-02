import { invoke } from "@tauri-apps/api/core";

interface AppInfo {
  version: string;
  whisper: string;
  arch: string;
  os: string;
}

window.addEventListener("DOMContentLoaded", async () => {
  const el = document.querySelector<HTMLElement>("#status");
  if (!el) return;
  try {
    const info = await invoke<AppInfo>("app_info");
    el.textContent = `v${info.version} · whisper.cpp ${info.whisper} · ${info.os}/${info.arch}`;
  } catch (e) {
    el.textContent = `Backend unavailable: ${String(e)}`;
  }
});
