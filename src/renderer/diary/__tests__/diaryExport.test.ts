// src/renderer/diary/__tests__/diaryExport.test.ts
//
// 일기 내보내기(#65) 순수 변환 계층: Markdown 문서 구조(작성순·제목·날짜·본문
// 원문 보존), JSON 번들 판별자/스키마, 파일명 안전화·타임스탬프.
import { describe, expect, it } from "vitest";
import type { DiaryEntry } from "@shared/types";
import {
  diaryFileName,
  formatDiaryJson,
  formatDiaryMarkdown,
  formatWhen,
  sanitizeFileBase,
} from "../diaryExport";

/** 로컬 타임존과 무관하게 고정 시각을 얻는다(로컬 표기를 검증하므로 로컬 생성). */
function at(y: number, mo: number, d: number, h: number, mi: number): number {
  return new Date(y, mo - 1, d, h, mi, 0, 0).getTime();
}

function entry(when: number, body: string): DiaryEntry {
  return { at: when, sessionId: "s1", body };
}

describe("formatWhen", () => {
  it("로컬 날짜·시각을 0채움으로 표기한다", () => {
    expect(formatWhen(at(2026, 7, 5, 9, 3))).toBe("2026-07-05 09:03");
  });
});

describe("sanitizeFileBase", () => {
  it("영숫자·한글·-·_는 유지하고 나머지는 대시로 바꾼다", () => {
    expect(sanitizeFileBase("Ada_Backend-1")).toBe("Ada_Backend-1");
    expect(sanitizeFileBase("백엔드담당")).toBe("백엔드담당");
    expect(sanitizeFileBase("a b/c:d")).toBe("a-b-c-d");
    expect(sanitizeFileBase("../etc")).toBe("---etc");
  });

  it("빈 입력은 agent로 폴백하고 40자로 자른다", () => {
    expect(sanitizeFileBase("")).toBe("agent");
    expect(sanitizeFileBase("가".repeat(100))).toBe("가".repeat(40));
  });
});

describe("diaryFileName", () => {
  it("<이름>-일기-<YYYYMMDD-HHmm>.md 형태다", () => {
    expect(diaryFileName("컴파일러", at(2026, 7, 25, 14, 30))).toBe("컴파일러-일기-20260725-1430.md");
  });

  it("이름의 경로 구분자가 파일명으로 새지 않는다", () => {
    expect(diaryFileName("a/b", at(2026, 1, 2, 3, 4))).toBe("a-b-일기-20260102-0304.md");
  });
});

describe("formatDiaryMarkdown", () => {
  it("제목·편수 뒤에 작성순(오래된 → 최신)으로 날짜·본문을 쓴다", () => {
    const md = formatDiaryMarkdown("컴파일러", [
      entry(at(2026, 7, 20, 14, 30), "첫 일기"),
      entry(at(2026, 7, 21, 9, 0), "둘째 일기"),
    ]);
    expect(md).toBe(
      [
        "# 컴파일러의 일기",
        "",
        "총 2편",
        "",
        "## 2026-07-20 14:30",
        "",
        "첫 일기",
        "",
        "## 2026-07-21 09:00",
        "",
        "둘째 일기",
        "",
      ].join("\n"),
    );
    // 순서 확인: 오래된 것이 먼저 나온다.
    expect(md.indexOf("첫 일기")).toBeLessThan(md.indexOf("둘째 일기"));
  });

  it("본문의 Markdown 특수문자를 이스케이프하지 않고 원문 보존한다", () => {
    const body = "# 헤딩처럼 보이는 줄\n\n```rs\nfn main() {}\n```";
    const md = formatDiaryMarkdown("루비", [entry(at(2026, 7, 25, 0, 0), body)]);
    expect(md).toContain(body);
  });

  it("빈 목록도 헤더만 있는 문서로 만든다(던지지 않음)", () => {
    expect(formatDiaryMarkdown("루비", [])).toBe("# 루비의 일기\n\n총 0편\n");
  });
});

describe("formatDiaryJson", () => {
  it("판별자·스키마 버전·이름·시각·원본 엔트리를 담는다", () => {
    const entries = [entry(at(2026, 7, 20, 14, 30), "본문")];
    const text = formatDiaryJson("컴파일러", entries, 1_700_000_000_000);
    const parsed = JSON.parse(text);
    expect(parsed).toEqual({
      kind: "agent-office.diary",
      schemaVersion: 1,
      agentName: "컴파일러",
      exportedAt: 1_700_000_000_000,
      entries,
    });
    expect(text.endsWith("\n")).toBe(true);
  });
});
