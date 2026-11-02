import { listen } from "@tauri-apps/api/event";

const label = document.getElementById("label")!;
const level = document.getElementById("level")!;
const dot = document.getElementById("dot")!;

listen<string>("indicator", (e) => {
  label.textContent = e.payload;
  dot.dataset.busy = String(e.payload !== "Listening…");
  if (e.payload !== "Listening…") level.style.width = "0%";
});

listen<number>("level", (e) => {
  // Peak level is roughly 0..0.3 for normal speech; scale for visibility.
  const pct = Math.min(100, Math.sqrt(e.payload) * 180);
  level.style.width = `${pct}%`;
});
