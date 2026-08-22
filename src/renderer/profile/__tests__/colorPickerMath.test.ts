// src/renderer/profile/__tests__/colorPickerMath.test.ts
//
// 컬러 피커 순수 계산부(kbm #2fj). DOM 없이 좌표 ↔ 색 변환만 검증한다.
import { describe, expect, it } from "vitest";
import {
  PRESET_HUE_STEPS,
  PRESET_SWATCHES,
  hexToHsv,
  hsvToHex,
  normalizeHex,
  ratioAt,
  sameHex,
} from "../colorPickerMath";

describe("normalizeHex", () => {
  it("# 생략·3자리 축약·대문자를 모두 '#rrggbb' 소문자로 정규화한다", () => {
    expect(normalizeHex("#AABBCC")).toBe("#aabbcc");
    expect(normalizeHex("aabbcc")).toBe("#aabbcc");
    expect(normalizeHex("#abc")).toBe("#aabbcc");
    expect(normalizeHex("  #AbC  ")).toBe("#aabbcc");
  });

  it("형식이 아니면 null(예외 아님) — 입력 중인 값과 확정값을 구분한다", () => {
    for (const bad of ["", "#", "#ab", "#abcd", "#abcdeg", "rgb(1,2,3)", undefined]) {
      expect(normalizeHex(bad)).toBeNull();
    }
  });
});

describe("hsvToHex / hexToHsv", () => {
  it("대표색이 알려진 값으로 나온다", () => {
    expect(hsvToHex({ h: 0, s: 1, v: 1 })).toBe("#ff0000");
    expect(hsvToHex({ h: 120, s: 1, v: 1 })).toBe("#00ff00");
    expect(hsvToHex({ h: 240, s: 1, v: 1 })).toBe("#0000ff");
    expect(hsvToHex({ h: 0, s: 0, v: 1 })).toBe("#ffffff");
    expect(hsvToHex({ h: 0, s: 0, v: 0 })).toBe("#000000");
  });

  it("색상은 360으로 감기고 채도/명도는 0..1로 클램프된다", () => {
    expect(hsvToHex({ h: 360, s: 1, v: 1 })).toBe("#ff0000");
    expect(hsvToHex({ h: -120, s: 1, v: 1 })).toBe("#0000ff");
    expect(hsvToHex({ h: 0, s: 5, v: 5 })).toBe("#ff0000");
    expect(hsvToHex({ h: 0, s: -1, v: -1 })).toBe("#000000");
  });

  it("hex → HSV → hex 왕복이 값을 보존한다", () => {
    for (const hex of ["#ff0000", "#123456", "#abcdef", "#7f7f7f", "#000000", "#ffffff"]) {
      expect(hsvToHex(hexToHsv(hex))).toBe(hex);
    }
  });

  it("무채색은 keepHue로 준 색상을 유지한다 — 슬라이더가 빨강으로 튀지 않게", () => {
    expect(hexToHsv("#808080", 200).h).toBe(200);
    expect(hexToHsv("#000000", 200).h).toBe(200);
    // 형식이 아닌 입력도 같은 규칙(검정 + keepHue).
    expect(hexToHsv("nope", 200)).toEqual({ h: 200, s: 0, v: 0 });
  });
});

describe("sameHex", () => {
  it("표기가 달라도 같은 색이면 참, 형식이 아니면 거짓", () => {
    expect(sameHex("#ABC", "#aabbcc")).toBe(true);
    expect(sameHex("aabbcc", "#AABBCC")).toBe(true);
    expect(sameHex("#aabbcc", "#aabbcd")).toBe(false);
    expect(sameHex("zzz", "zzz")).toBe(false);
  });
});

describe("ratioAt", () => {
  it("트랙 안에서는 0..1 비율, 밖으로 나가면 끝에서 멈춘다", () => {
    expect(ratioAt(50, 0, 100)).toBeCloseTo(0.5);
    expect(ratioAt(-10, 0, 100)).toBe(0);
    expect(ratioAt(200, 0, 100)).toBe(1);
    // 크기 0(측정 전 레이아웃)에서도 NaN을 내지 않는다.
    expect(ratioAt(5, 0, 0)).toBe(0);
  });
});

describe("PRESET_SWATCHES", () => {
  it("모든 칸이 유효한 hex이고 줄마다 색상 수가 같다", () => {
    expect(PRESET_SWATCHES.length).toBeGreaterThan(1);
    for (const row of PRESET_SWATCHES) {
      expect(row).toHaveLength(PRESET_HUE_STEPS);
      for (const c of row) expect(c).toMatch(/^#[0-9a-f]{6}$/);
    }
  });

  it("마지막 줄은 검정→흰색 무채색 계단이다", () => {
    const grays = PRESET_SWATCHES[PRESET_SWATCHES.length - 1];
    expect(grays[0]).toBe("#000000");
    expect(grays[grays.length - 1]).toBe("#ffffff");
    for (const c of grays) expect(c.slice(1, 3)).toBe(c.slice(5, 7));
  });
});
