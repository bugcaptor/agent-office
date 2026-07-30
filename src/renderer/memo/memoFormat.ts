// src/renderer/memo/memoFormat.ts
//
// 포스트잇 메모(#79) 표시용 순수 포매터. 메모 헤더의 시각은 RFC3339 문자열
// (로컬 오프셋 포함)이므로 diaryExport.formatWhen(epoch ms)을 쓸 수 없다.
// 파싱 실패(손으로 고친 파일 등)는 원문을 그대로 보여준다 — 표시 계층이
// 사용자 파일 때문에 깨지지 않게.

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

/** `2026-07-30T12:34:56+09:00` → `2026-07-30 12:34`. 파싱 불가면 원문 그대로. */
export function formatMemoWhen(iso: string): string {
  if (!iso) return "";
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return iso;
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(
    d.getHours()
  )}:${pad(d.getMinutes())}`;
}
