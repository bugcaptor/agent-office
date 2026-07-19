// src/renderer/diary/diaryGenerator.ts
//
// 캐릭터 일기(#56) 생성기. 요약기 파이프라인을 그대로 재사용한다: 렌더러가
// 설정의 provider(요약기와 공유, summaryProvider)를 골라
// tauriApi.summarizeText(provider, DIARY_SYSTEM_PROMPT, userText)로 위임한다.
// "일기"는 다른 시스템 프롬프트 + 성격 프롬프트 + 누적 작업 로그를 넣은 같은 호출이다.
//
// opt-in 게이트: appSettings.diaryEnabled=false면 CLI를 호출하지 않는다
// (요약기와 동일 정책). provider CLI 미설치("${provider}-not-found")·실패·
// 타임아웃은 조용히 폴백(일기 미생성) — 나머지 기능은 정상 동작.
//
// 트리거: (1) 세션 종료 시 자동(diaryAutoWriter, #60) — diaryEnabled면 기본 동작.
// (2) 탭 컨텍스트 메뉴의 "일기 쓰기"(수동). 자동 경로는 종료된 세션을 sessionId로
// 명시해 그 세션 로그만 담고, 수동 경로는 인자 없이 현재 세션을 유추한다. 성공 시
// append_diary_entry로 영속화하고 그 세션의 작업 로그를 소진(clear)해 다음 일기가
// 새 작업만 담게 한다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { DiaryEntry, SummaryProvider } from "@shared/types";
import { formatWorkLog, workLog, type WorkLog } from "./workLog";

export const DIARY_SYSTEM_PROMPT =
  "너는 한 캐릭터의 일기 작성기다. 아래 [성격]을 문체로 삼아, [작업 로그]를 1인칭 한국어 일기 한 편으로 써라. 성격에 따라 초등학생 일기처럼 쓰기도 하고 차가운 작업 일지처럼 쓰기도 한다 — [성격]의 말투·태도를 문체에 그대로 반영하라. [성격]이 비어 있으면 담백한 중립 문체로 써라. 반드시 실제로 한 일(수정한 파일·실행한 명령·목표)이 드러나야 한다(작업 로그를 겸한다). 분량은 3~8문장. 규칙: 한국어만, 사과·메타발언·머리말·따옴표·마크다운 금지, 일기 본문만 출력.";

/** 일기 본문 최소 길이(공백 제외). 이보다 짧으면 생성 실패로 본다. */
const BODY_MIN_CHARS = 4;

/** 생성 결과 사유 — 호출부(UI)가 사용자 피드백에 쓴다. */
export type DiaryResult =
  | { ok: true; entry: DiaryEntry }
  | { ok: false; reason: "disabled" | "no-work" | "in-flight" | "cli-missing" | "failed" };

export interface DiaryGeneratorDeps {
  summarizeFn?: (
    provider: SummaryProvider,
    instruction: string,
    text: string,
  ) => Promise<string>;
  appendFn?: (agentId: string, entry: DiaryEntry) => Promise<void>;
  now?: () => number;
  /** 주입용 작업 로그 버퍼(테스트). 기본은 전역 workLog. */
  log?: WorkLog;
}

/** 응답 정제: 코드펜스·머리말·따옴표 제거, 공백 정규화. 비면 null. */
export function sanitizeDiaryBody(raw: string): string | null {
  const s = raw
    .replace(/```[a-zA-Z]*\n?/g, "")
    .replace(/```/g, "")
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
    .join("\n")
    .trim();
  if (Array.from(s.replace(/\s/g, "")).length < BODY_MIN_CHARS) return null;
  return s;
}

/** 동시 중복 생성 방지(더블클릭 등) — agentId별 인플라이트 표시. */
const inflight = new Set<string>();

/**
 * 한 캐릭터의 누적 작업 로그로 일기 한 편을 생성·영속화한다. opt-in OFF·작업
 * 로그 없음·CLI 미설치·실패는 조용한 사유 반환(throw 안 함). 성공 시 그 세션의
 * 작업 로그를 소진한다.
 *
 * targetSessionId를 주면(자동 경로) 그 세션의 로그만 담고 그 세션으로 기록한다.
 * 생략하면(수동 경로) 현재 라벨의 세션을 유추한다(하위호환).
 */
export async function generateDiary(
  agentId: string,
  deps: DiaryGeneratorDeps = {},
  targetSessionId?: string,
): Promise<DiaryResult> {
  const summarizeFn =
    deps.summarizeFn ??
    ((provider, instruction, text) => tauriApi.summarizeText(provider, instruction, text));
  const appendFn = deps.appendFn ?? ((id, entry) => tauriApi.appendDiaryEntry(id, entry));
  const now = deps.now ?? Date.now;
  const log = deps.log ?? workLog;

  const state = useAppStore.getState();
  if (!state.appSettings.diaryEnabled) return { ok: false, reason: "disabled" };

  const items = targetSessionId === undefined ? log.items(agentId) : log.items(agentId, targetSessionId);
  if (items.length === 0) return { ok: false, reason: "no-work" };

  if (inflight.has(agentId)) return { ok: false, reason: "in-flight" };

  const provider = state.appSettings.summaryProvider;
  const personality = state.agents[agentId]?.personalityPrompt?.trim() ?? "";
  // 일기가 다룰 세션 — 자동 경로는 지정 세션, 수동 경로는 현재 라벨의 세션
  // (없으면 최신 로그 항목의 세션).
  const sessionId =
    targetSessionId ?? state.taskLabels[agentId]?.sessionId ?? items[items.length - 1].sessionId;

  const userText = `[성격]\n${personality || "(없음)"}\n\n[작업 로그]\n${formatWorkLog(items)}`;

  inflight.add(agentId);
  try {
    const raw = await summarizeFn(provider, DIARY_SYSTEM_PROMPT, userText);
    const body = sanitizeDiaryBody(raw);
    if (body === null) return { ok: false, reason: "failed" };
    const entry: DiaryEntry = { at: now(), sessionId, body };
    await appendFn(agentId, entry);
    // 이 세션의 로그를 소진 — 다음 일기는 새 작업만 담는다.
    log.clear(agentId, sessionId);
    return { ok: true, entry };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    if (message.includes(`${provider}-not-found`)) {
      console.warn(`diary: ${provider} CLI 미설치 — 일기 생성 건너뜀`);
      return { ok: false, reason: "cli-missing" };
    }
    if (message.includes("summarizer-disabled")) {
      // 설정 OFF 경합 — 스토어 게이트가 다음 요청을 막는다.
      return { ok: false, reason: "disabled" };
    }
    console.warn(`diary: 일기 생성 실패(agent=${agentId})`, err);
    return { ok: false, reason: "failed" };
  } finally {
    inflight.delete(agentId);
  }
}
