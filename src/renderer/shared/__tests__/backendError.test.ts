// src/renderer/shared/__tests__/backendError.test.ts
//
// 백엔드 에러 코드 → 사용자 문구 매핑의 명세. 검증할 것이 셋이다:
//
//  1. `"{code}: {detail}"` 파싱 — 상세에 콜론이 있어도 코드만 떼어낸다.
//  2. **기술적 상세는 번역하지 않고 그대로 붙는다** — 사용자가 개발자에게
//     전달할 원문이다.
//  3. **언어를 바꾸면 문구가 따라온다** — `t()`를 모듈 최상위에서 부르면
//     이 부분이 조용히 깨진다(그래서 `backendErrorText`가 호출 시점에 부른다).
//
// 정본(ko)과 en 양쪽을 다 본다. 테스트 UI 언어는 test-setup이 ko로 못박으므로
// en 블록은 스스로 언어를 바꾸고 afterAll에서 되돌린다(파일 간 누수 방지).
import { afterAll, describe, expect, it } from "vitest";

import { initI18nForTest, t } from "@renderer/i18n";
import {
  BACKEND_ERROR_KEY,
  backendErrorText,
  parseBackendError,
  type BackendErrorCode,
} from "../backendError";

afterAll(async () => {
  await initI18nForTest(); // 정본(ko)으로 복구
});

describe("parseBackendError", () => {
  it("코드만 있는 에러는 detail이 빈 문자열이다", () => {
    expect(parseBackendError("empty-log")).toEqual({ code: "empty-log", detail: "" });
    expect(parseBackendError("missing_elevenlabs_key")).toEqual({
      code: "missing_elevenlabs_key",
      detail: "",
    });
  });

  it("첫 콜론만 가른다 — 상세 안의 콜론은 상세에 남는다", () => {
    expect(parseBackendError("read-failed: C:\\a\\b.md (오류: 2)")).toEqual({
      code: "read-failed",
      detail: "C:\\a\\b.md (오류: 2)",
    });
  });

  it("Error 인스턴스와 그 밖의 값도 같은 규칙으로 다룬다", () => {
    expect(parseBackendError(new Error("write-failed: a.md (x)")).code).toBe("write-failed");
    expect(parseBackendError(undefined).code).toBe("undefined");
  });
});

describe("backendErrorText — 코드가 문구가 된다", () => {
  it("매핑에 있는 코드는 카탈로그 문구(ko)", () => {
    expect(backendErrorText("missing_elevenlabs_key")).toBe(t("common:errors.ttsKeyMissing"));
    expect(backendErrorText("web-remote-down")).toBe(t("common:errors.webRemoteDown"));
  });

  it("상세는 번역하지 않고 괄호로 붙는다", () => {
    const text = backendErrorText("write-failed: notes/a.md (Permission denied)");
    expect(text).toBe(`${t("common:errors.writeFailed")} (notes/a.md (Permission denied))`);
    // 원문 경로·OS 메시지가 그대로 살아 있어야 개발자에게 전달할 수 있다.
    expect(text).toContain("notes/a.md");
    expect(text).toContain("Permission denied");
  });

  it("모르는 코드는 원문을 그대로 보여준다(상류 오류는 종류가 열려 있다)", () => {
    expect(backendErrorText("codex exited 7: boom")).toBe("codex exited 7: boom");
    expect(backendErrorText("openrouter-key-missing")).toBe("openrouter-key-missing");
  });

  it("overrides가 공통 매핑을 이긴다", () => {
    const over = { timeout: "activity:sessionLog.errTimeout" };
    expect(backendErrorText("timeout", over)).toBe(t("activity:sessionLog.errTimeout"));
    expect(backendErrorText("timeout")).toBe(t("common:errors.timeout"));
  });

  // 매핑된 키가 실제로 카탈로그에 있어야 한다 — 오타가 나면 i18next가 키
  // 문자열을 그대로 돌려주므로 "코드 === 문구"로 걸린다.
  it("매핑된 키가 전부 카탈로그에 실재한다", () => {
    const missing = (Object.entries(BACKEND_ERROR_KEY) as [BackendErrorCode, string][])
      .filter(([, key]) => t(key) === key)
      .map(([code]) => code);
    expect(missing).toEqual([]);
  });
});

describe("backendErrorText — 언어를 따라간다", () => {
  it("en으로 바꾸면 영어 문구가 나온다", async () => {
    const ko = backendErrorText("tailscale-cli-not-found");
    await initI18nForTest("en");
    const en = backendErrorText("tailscale-cli-not-found");
    expect(en).toBe("Could not find the Tailscale CLI.");
    expect(en).not.toBe(ko);
    // 상세는 언어와 무관하게 원문 그대로.
    expect(backendErrorText("read-failed: a.md (os error 2)")).toBe(
      "Could not read the file. (a.md (os error 2))"
    );
  });

  it("en에서도 모르는 코드는 원문이다", async () => {
    await initI18nForTest("en");
    expect(backendErrorText("gemini-not-found")).toBe("gemini-not-found");
  });
});
