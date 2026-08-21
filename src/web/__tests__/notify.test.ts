// src/web/__tests__/notify.test.ts
//
// 브라우저 알림의 **게이트 규칙**. API 자체(권한 요청·표시)는 브라우저가 하고
// 여기서 지킬 것은 "언제 띄우지 않는가"다 — 셋 중 하나라도 어긋나면 안 뜬다.

import { describe, expect, it } from "vitest";
import { shouldNotify } from "@web/notify";

const gate = (over: Partial<Parameters<typeof shouldNotify>[0]> = {}) => ({
  enabled: true,
  permission: "granted" as NotificationPermission | null,
  visibility: "hidden",
  ...over,
});

describe("shouldNotify", () => {
  it("토글·권한·가시성이 모두 맞아야 띄운다", () => {
    expect(shouldNotify(gate())).toBe(true);
  });

  it("토글이 꺼져 있으면 권한이 있어도 안 띄운다", () => {
    expect(shouldNotify(gate({ enabled: false }))).toBe(false);
  });

  it("권한이 granted가 아니면 안 띄운다", () => {
    for (const permission of ["default", "denied", null] as const) {
      expect(shouldNotify(gate({ permission }))).toBe(false);
    }
  });

  it("보고 있는 탭에는 시스템 알림을 겹치지 않는다", () => {
    expect(shouldNotify(gate({ visibility: "visible" }))).toBe(false);
    // 브라우저가 주는 나머지 상태는 전부 "안 보고 있다"로 친다.
    expect(shouldNotify(gate({ visibility: "prerender" }))).toBe(true);
  });
});
