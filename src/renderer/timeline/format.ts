// src/renderer/timeline/format.ts
//
// Duration formatting for the session time panel: under an hour
// shows "Nm SSs" (zero-padded seconds), an hour or more shows "Hh MMm"
// (zero-padded minutes). Pure; used by the panel and unit-tested directly.

const HOUR_MS = 3_600_000;
const MIN_MS = 60_000;

export function formatDuration(ms: number): string {
  // Number.isFinite clamps NaN and ±Infinity too, not just negatives.
  const v = Number.isFinite(ms) && ms > 0 ? Math.floor(ms) : 0;
  if (v < HOUR_MS) {
    const m = Math.floor(v / MIN_MS);
    const s = Math.floor((v % MIN_MS) / 1000);
    return `${m}m ${String(s).padStart(2, "0")}s`;
  }
  const h = Math.floor(v / HOUR_MS);
  const m = Math.floor((v % HOUR_MS) / MIN_MS);
  return `${h}h ${String(m).padStart(2, "0")}m`;
}
