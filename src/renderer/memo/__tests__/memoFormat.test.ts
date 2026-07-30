// src/renderer/memo/__tests__/memoFormat.test.ts
//
// 메모 헤더 시각(RFC3339 문자열) 표시 포매터 + 열림 상태 localStorage 영속.
// 둘 다 순수 모듈이므로 node 환경(terminalViewMode.test.ts와 같은 관례).
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { formatMemoWhen } from "../memoFormat";
import {
  MEMO_VISIBLE_STORAGE_KEY,
  loadStoredMemoVisible,
  persistMemoVisible,
} from "../memoVisibility";

describe("formatMemoWhen", () => {
  it("RFC3339을 `YYYY-MM-DD HH:mm`으로 줄인다", () => {
    // 오프셋이 붙은 값이므로 로컬 타임존과 무관하게 같은 순간을 가리킨다 —
    // 표시 결과는 실행 환경의 타임존을 따르므로 Date로 기대값을 만든다.
    const iso = "2026-07-30T12:34:56+09:00";
    const d = new Date(Date.parse(iso));
    const pad = (n: number) => String(n).padStart(2, "0");
    expect(formatMemoWhen(iso)).toBe(
      `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(
        d.getHours()
      )}:${pad(d.getMinutes())}`
    );
  });

  it("빈 문자열은 빈 문자열", () => {
    expect(formatMemoWhen("")).toBe("");
  });

  it("파싱 불가한 값(손으로 고친 헤더)은 원문 그대로 보여준다", () => {
    expect(formatMemoWhen("어제쯤")).toBe("어제쯤");
  });
});

describe("열림 상태 영속", () => {
  const backing = new Map<string, string>();
  const original = (globalThis as { localStorage?: unknown }).localStorage;

  beforeEach(() => {
    backing.clear();
    (globalThis as { localStorage?: unknown }).localStorage = {
      getItem: (k: string) => (backing.has(k) ? backing.get(k)! : null),
      setItem: (k: string, v: string) => void backing.set(k, v),
      removeItem: (k: string) => void backing.delete(k),
      clear: () => backing.clear(),
      key: () => null,
      length: 0,
    };
  });

  afterEach(() => {
    (globalThis as { localStorage?: unknown }).localStorage = original;
  });

  it("persist 후 load가 같은 값을 돌려준다", () => {
    persistMemoVisible(true);
    expect(backing.get(MEMO_VISIBLE_STORAGE_KEY)).toBe("true");
    expect(loadStoredMemoVisible()).toBe(true);

    persistMemoVisible(false);
    expect(loadStoredMemoVisible()).toBe(false);
  });

  it("저장된 값이 없으면 닫힘(false)이 기본", () => {
    expect(loadStoredMemoVisible()).toBe(false);
  });

  it("알 수 없는 값은 닫힘으로 본다", () => {
    backing.set(MEMO_VISIBLE_STORAGE_KEY, "yes");
    expect(loadStoredMemoVisible()).toBe(false);
  });

  it("localStorage가 없는 환경에서도 던지지 않는다", () => {
    (globalThis as { localStorage?: unknown }).localStorage = undefined;
    expect(() => persistMemoVisible(true)).not.toThrow();
    expect(loadStoredMemoVisible()).toBe(false);
  });
});
