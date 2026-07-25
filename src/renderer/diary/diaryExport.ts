// src/renderer/diary/diaryExport.ts
//
// 캐릭터 일기 내보내기(#65)의 순수 변환 계층. 스토어가 들고 있는 `entries`를
// 사람이 읽는 Markdown과 자기완결형 JSON 번들 두 형태의 문자열로 만들고,
// 저장 다이얼로그의 초기 파일명을 짓는다. 파일 I/O(저장 다이얼로그·쓰기)는
// 백엔드 `export_diary_file`이 하고, 여기서는 값 변환만 담당한다
// (characterIo.ts와 같은 관례 — vitest로 직접 검증).
//
// Markdown은 본문을 이스케이프하지 않는다: 일기 본문에 코드블록·`#` 등이 있어도
// 원문 그대로 보존하는 편이 "사람이 읽는 문서"라는 목적에 맞다.

import {
  DIARY_BUNDLE_KIND,
  DIARY_BUNDLE_SCHEMA_VERSION,
  type DiaryBundle,
  type DiaryEntry,
} from "@shared/types";

const pad = (n: number) => String(n).padStart(2, "0");

/** epoch ms → 사람이 읽는 로컬 날짜·시각(`2026-07-25 14:30`). 일기 목록과
 *  내보낸 Markdown이 같은 표기를 쓰도록 여기 한 곳에 둔다. */
export function formatWhen(at: number): string {
  const d = new Date(at);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** 파일명 안전화 — Rust `shell_export::sanitize_agent_name` 미러(영숫자·한글·
 *  `-`·`_`만 남기고 나머지는 `-`, 40자 절단, 빈 결과는 `agent`). 저장 다이얼로그의
 *  초기 파일명이므로 경로 구분자·콜론이 새어 들어가지 않게 한다. */
export function sanitizeFileBase(name: string): string {
  const out = [...name]
    .slice(0, 40)
    .map((ch) => (/[\p{L}\p{N}_-]/u.test(ch) ? ch : "-"))
    .join("");
  return out || "agent";
}

/** 저장 다이얼로그 초기 파일명: `<캐릭터명>-일기-<YYYYMMDD-HHmm>.md`.
 *  확장자는 기본 필터(Markdown)에 맞추고, 사용자가 `.json`으로 바꾸면 백엔드가
 *  JSON 본문을 쓴다. */
export function diaryFileName(agentName: string, at: number): string {
  const d = new Date(at);
  const stamp = `${d.getFullYear()}${pad(d.getMonth() + 1)}${pad(d.getDate())}-${pad(d.getHours())}${pad(d.getMinutes())}`;
  return `${sanitizeFileBase(agentName)}-일기-${stamp}.md`;
}

/**
 * 일기 전체를 사람이 읽는 Markdown 문서로. 작성순(오래된 → 최신)으로 쓴다 —
 * 화면은 최신 먼저지만, 문서로 읽을 땐 시간순이 자연스럽다.
 * `entries`가 비어도 헤더만 있는 문서를 돌려준다(호출부가 빈 목록을 막지만
 * 여기서 던지지는 않는다).
 */
export function formatDiaryMarkdown(agentName: string, entries: DiaryEntry[]): string {
  const lines: string[] = [`# ${agentName}의 일기`, "", `총 ${entries.length}편`, ""];
  for (const e of entries) {
    lines.push(`## ${formatWhen(e.at)}`, "", e.body.trimEnd(), "");
  }
  return lines.join("\n");
}

/** 일기 전체를 자기완결형 JSON 번들로(원본 레코드 보존). 들여쓰기 2칸 —
 *  사람이 열어 볼 수도 있는 파일이다. */
export function formatDiaryJson(
  agentName: string,
  entries: DiaryEntry[],
  exportedAt: number,
): string {
  const bundle: DiaryBundle = {
    kind: DIARY_BUNDLE_KIND,
    schemaVersion: DIARY_BUNDLE_SCHEMA_VERSION,
    agentName,
    exportedAt,
    entries,
  };
  return `${JSON.stringify(bundle, null, 2)}\n`;
}
