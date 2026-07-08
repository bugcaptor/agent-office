// src/renderer/labels/summarizer.ts
//
// 머리 위 라벨용 LLM 요약기. 로컬 `claude` CLI 헤드리스 호출로
// 요약한다(haiku, --max-turns 1 — src-tauri/src/claude_cli.rs). store의
// taskLabels를 구독해 goal(세션 첫 프롬프트, 1회)/currentSummary(프롬프트
// 마다)를 채운다.
//
// 실패 정책: `claude` CLI 미설치("claude-not-found") → 앱 실행 동안 영구
// 비활성(원문 폴백은 UI 몫). 그 외 호출 실패 → agent별 30초 쿨다운 후 다음
// store 변화 때 재시도. 원문이 그 사이 바뀌면(stale) 결과를 폐기한다 —
// store의 현재 값과 요청 당시 원문을 비교.
//
// opt-in 게이트: appStore.appSettings.claudeCliEnabled=false면 요청 자체를
// 보내지 않는다(원문 폴백). ON 상태에서 레이스로 "claude-cli-disabled"
// 에러를 받으면(설정이 막 꺼진 경합) 쿨다운/영구비활성 없이 그냥 무시한다 —
// 스토어 설정이 단일 진실원이므로 다음 요청은 게이트가 이미 최신값으로 막거나 통과시킨다.
import { useAppStore } from "../store/appStore";
import type { AgentTaskLabel } from "../store/types";
import { tauriApi } from "../ipc/tauriApi";

export const GOAL_SYSTEM_PROMPT =
  "다음은 코딩 에이전트 세션의 첫 사용자 지시다. 이 세션의 목표를 한국어 12자 이내의 명사구로만 답하라.";
export const CURRENT_SYSTEM_PROMPT =
  "다음 사용자 지시를 한국어 18자 이내 한 줄로 요약하라. 요약문만 답하라.";
const FAILURE_COOLDOWN_MS = 30_000;

export interface SummarizerDeps {
  summarizeFn?: (instruction: string, text: string) => Promise<string>;
  now?: () => number;
}

type SummaryKind = "goal" | "current";

/**
 * store 구독을 설치하고 해제 함수를 돌려준다. 앱 부트에서 1회 호출
 * (bootstrap.ts). deps는 테스트 주입용 — 실제 앱은 인자 없이 부른다.
 */
export function installTaskLabelSummarizer(deps: SummarizerDeps = {}): () => void {
  const summarizeFn = deps.summarizeFn ?? ((instruction, text) => tauriApi.summarizeText(instruction, text));
  const now = deps.now ?? Date.now;

  const cache = new Map<string, string>(); // `${kind}|${원문}` -> 요약
  const inflight = new Set<string>(); // cache와 같은 키
  const cooldownUntil = new Map<string, number>(); // agentId -> epoch ms
  let disabled = false; // claude CLI 미설치 확인 시 true — 앱 실행 동안 영구

  /** stale 가드를 통과할 때만 store에 반영한다. */
  function apply(agentId: string, kind: SummaryKind, sourceText: string, summary: string): void {
    const label = useAppStore.getState().taskLabels[agentId];
    if (!label) return;
    if (kind === "goal" && label.firstPromptText === sourceText) {
      useAppStore.getState().setTaskLabelSummary(agentId, { goal: summary });
    } else if (kind === "current" && label.latestPromptText === sourceText) {
      useAppStore.getState().setTaskLabelSummary(agentId, { currentSummary: summary });
    }
  }

  function request(agentId: string, kind: SummaryKind, text: string): void {
    if (!useAppStore.getState().appSettings.claudeCliEnabled) return; // opt-in OFF — 원문 폴백
    if (disabled) return;
    const key = `${kind}|${text}`;
    const cached = cache.get(key);
    if (cached !== undefined) {
      apply(agentId, kind, text, cached);
      return;
    }
    if (inflight.has(key)) return;
    if (now() < (cooldownUntil.get(agentId) ?? 0)) return;
    inflight.add(key);
    void (async () => {
      try {
        const sys = kind === "goal" ? GOAL_SYSTEM_PROMPT : CURRENT_SYSTEM_PROMPT;
        // 호출 1회당 사용자의 Claude 구독/크레딧을 소모한다(haiku, --max-turns 1).
        const summary = await summarizeFn(sys, text);
        cache.set(key, summary);
        apply(agentId, kind, text, summary);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        if (message.includes("claude-not-found")) {
          disabled = true;
          console.warn("taskLabels: claude CLI 미설치 — 요약 비활성(원문 폴백 표시)");
        } else if (message.includes("claude-cli-disabled")) {
          // 설정 OFF 경합 — 스토어 게이트가 다음 요청을 막는다. 쿨다운 불필요.
        } else {
          console.warn(`taskLabels: 요약 실패(kind=${kind}, agent=${agentId})`, err);
          cooldownUntil.set(agentId, now() + FAILURE_COOLDOWN_MS);
        }
      } finally {
        inflight.delete(key);
      }
    })();
  }

  /** 요약이 비어 있는 라벨을 훑어 필요한 요청을 낸다(멱등). */
  function sweep(labels: Record<string, AgentTaskLabel>): void {
    for (const [agentId, l] of Object.entries(labels)) {
      if (l.firstPromptText && l.goal === undefined) request(agentId, "goal", l.firstPromptText);
      if (l.latestPromptText && l.currentSummary === undefined)
        request(agentId, "current", l.latestPromptText);
    }
  }

  const off = useAppStore.subscribe((s) => s.taskLabels, sweep);
  sweep(useAppStore.getState().taskLabels);
  return off;
}
