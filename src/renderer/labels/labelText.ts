// src/renderer/labels/labelText.ts
//
// 머리 위 라벨의 파생 텍스트 순수 헬퍼. store에 저장하지 않고
// 표시 시점에 파생한다.

/** cwd의 basename. `/`와 `\` 둘 다 구분자로 취급, 트레일링 구분자 무시. */
export function projectNameFromCwd(cwd: string | undefined): string | undefined {
  if (!cwd) return undefined;
  const parts = cwd.split(/[/\\]+/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : undefined;
}

/** 원문 폴백 표시: 첫 비공백 줄을 max자(chars)로 절단, 넘치면 "…" 부착. */
export function firstLine(text: string | undefined, max: number): string | undefined {
  if (!text) return undefined;
  const line = text
    .split("\n")
    .map((l) => l.trim())
    .find((l) => l.length > 0);
  if (!line) return undefined;
  const chars = Array.from(line);
  return chars.length <= max ? line : chars.slice(0, max).join("") + "…";
}
