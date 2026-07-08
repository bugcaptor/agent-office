// src/renderer/ui/__tests__/contextMenu.theme.test.ts
//
// 회귀 테스트: 컨텍스트 메뉴(우클릭 → "프로필 편집" 등)가 테마를 따라가는지.
// 과거 contextMenu.css가 미드나이트 색(#1b1b24 등)을 하드코딩해 밝음(daylight)
// 테마에서도 메뉴가 어둡게 렌더되는 버그가 있었다. tokens.css의 계약(색은
// var(--...) 토큰만, 실값은 themes.ts가 단일 원천)을 파일 수준에서 검증한다.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const css = readFileSync(resolve(__dirname, "../contextMenu.css"), "utf8");

describe("contextMenu.css 테마 적합성", () => {
  it("헥스 색 리터럴을 하드코딩하지 않는다(테마 토큰만 사용)", () => {
    expect(css).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
  });

  it("메뉴 배경/텍스트/호버가 테마 토큰을 사용한다", () => {
    expect(css).toMatch(/background:\s*var\(--bg-panel\)/);
    expect(css).toMatch(/color:\s*var\(--text\)/);
    expect(css).toMatch(/background:\s*var\(--bg-panel-hi\)/);
  });
});
