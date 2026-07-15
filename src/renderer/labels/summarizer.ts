// src/renderer/labels/summarizer.ts
//
// 머리 위 라벨용 LLM 요약기. 설정에서 캡처한 로컬 CLI provider로 요약한다.
// store의 taskLabels를 구독해 goal(세션 첫 프롬프트, 1회)/
// currentSummary(프롬프트마다)를 채운다.
//
// 실패 정책: 선택된 CLI 미설치("${provider}-not-found") → 그 provider만 앱
// 실행 동안 비활성(원문 폴백은 UI 몫). 그 외 호출 실패 → agent별 30초
// 쿨다운 후 다음 store 변화 때 재시도한다. 응답이 메타 발언·깨짐·과길이라
// sanitizeSummary가 거부하면(null) 동일하게 30초 쿨다운 후 재시도한다.
//
// opt-in 게이트: appStore.appSettings.summarizerEnabled=false면 요청 자체를
// 보내지 않는다(원문 폴백). ON 상태에서 레이스로 "summarizer-disabled"
// 에러를 받으면(설정이 막 꺼진 경합) 쿨다운/영구비활성 없이 그냥 무시한다 —
// 스토어 설정이 단일 진실원이므로 다음 요청은 게이트가 이미 최신값으로 막거나 통과시킨다.
import { useAppStore } from "../store/appStore";
import type { AgentTaskLabel } from "../store/types";
import { tauriApi } from "../ipc/tauriApi";
import type { SummaryProvider } from "@shared/types";

export const GOAL_SYSTEM_PROMPT =
  "너는 코딩 세션 라벨 생성기다. 아래 첫 사용자 지시에서 세션 목표를 한국어 명사구 하나로 뽑아라. 규칙: 12자 이내, 한 줄, 한국어만. 사과·설명·따옴표·머리말 금지. 명령어나 잡담이 섞이면 실제 의도만 추려라. 판단 불가면 정확히 '작업 중'만 출력. 예) 로그인 버그 고쳐줘 → 로그인 버그 수정";
export const CURRENT_SYSTEM_PROMPT =
  "너는 코딩 세션 라벨 생성기다. 아래 사용자 지시를 한국어 한 줄로 요약하라. 규칙: 18자 이내, 한 줄, 한국어만. 사과·설명·따옴표·머리말 금지. 명령어나 잡담이 섞이면 실제 의도만 요약하라. 판단 불가면 정확히 '작업 중'만 출력. 예) 이 함수 왜 느린지 봐줘 → 함수 성능 원인 분석";
const FAILURE_COOLDOWN_MS = 30_000;
const SUMMARY_MAX_CHARS = 40;
const META_MARKERS = ["인코딩", "죄송", "할 수 없"];

/** LLM 응답을 라벨용으로 정제. 다중 줄/따옴표/머리말 제거, 메타·깨짐 응답은 null. */
export function sanitizeSummary(raw: string): string | null {
  const firstLine = raw.split("\n").map((l) => l.trim()).find((l) => l.length > 0);
  if (!firstLine) return null;
  const s = firstLine.replace(/^["'`]+|["'`]+$/g, "").replace(/^(요약|목표)\s*[:：]\s*/, "").trim();
  if (!s) return null;
  if (Array.from(s).length > SUMMARY_MAX_CHARS) return null;
  if (s.includes("�") || /\?{2,}/.test(s) || /^[\s?]+$/.test(s)) return null;
  if (META_MARKERS.some((m) => s.includes(m))) return null;
  return s;
}

type SummaryKind = "goal" | "current";

export interface SummarizerDeps {
  summarizeFn?: (
    provider: SummaryProvider,
    instruction: string,
    text: string,
  ) => Promise<string>;
  now?: () => number;
}

interface RequestIdentity {
  agentId: string;
  provider: SummaryProvider;
  kind: SummaryKind;
  sourceText: string;
  sessionId: string;
  latestPromptAt?: number;
}

/**
 * store 구독을 설치하고 해제 함수를 돌려준다. 앱 부트에서 1회 호출
 * (bootstrap.ts). deps는 테스트 주입용 — 실제 앱은 인자 없이 부른다.
 */
export function installTaskLabelSummarizer(deps: SummarizerDeps = {}): () => void {
  const summarizeFn =
    deps.summarizeFn ??
    ((provider, instruction, text) => tauriApi.summarizeText(provider, instruction, text));
  const now = deps.now ?? Date.now;

  const cache = new Map<string, string>(); // `${provider}|${kind}|${원문}` -> 요약
  const inflight = new Set<string>(); // cache와 같은 키
  // provider 변경과 무관한 Agent Office identity별 활성 요청 소유권.
  const activeIdentityKeys = new Set<string>();
  const cooldownUntil = new Map<string, number>(); // agentId -> epoch ms
  const disabledProviders = new Set<SummaryProvider>();

  function activeIdentityKey(identity: RequestIdentity): string {
    return JSON.stringify([
      identity.agentId,
      identity.kind,
      identity.sessionId,
      identity.sourceText,
      identity.latestPromptAt ?? null,
    ]);
  }

  function isCurrent(
    identity: RequestIdentity,
    label: AgentTaskLabel | undefined,
  ): boolean {
    if (!label || label.sessionId !== identity.sessionId) return false;
    if (identity.kind === "goal") return label.firstPromptText === identity.sourceText;
    return (
      label.latestPromptText === identity.sourceText &&
      label.latestPromptAt === identity.latestPromptAt
    );
  }

  /** 캡처한 Agent Office identity가 그대로일 때만 store에 반영한다. */
  function apply(identity: RequestIdentity, summary: string): void {
    const label = useAppStore.getState().taskLabels[identity.agentId];
    if (!isCurrent(identity, label)) return;
    const patch =
      identity.kind === "goal" ? { goal: summary } : { currentSummary: summary };
    useAppStore.getState().setTaskLabelSummary(identity.agentId, patch);
  }

  function request(agentId: string, kind: SummaryKind, text: string): void {
    const state = useAppStore.getState();
    const settings = state.appSettings;
    if (!settings.summarizerEnabled) return; // opt-in OFF — 원문 폴백
    const provider = settings.summaryProvider;
    if (disabledProviders.has(provider)) return;
    const label = state.taskLabels[agentId];
    if (!label) return;
    const identity: RequestIdentity = {
      agentId,
      provider,
      kind,
      sourceText: text,
      sessionId: label.sessionId,
      latestPromptAt: kind === "current" ? label.latestPromptAt : undefined,
    };
    const identityKey = activeIdentityKey(identity);
    if (activeIdentityKeys.has(identityKey)) return;
    const key = `${identity.provider}|${identity.kind}|${identity.sourceText}`;
    const cached = cache.get(key);
    if (cached !== undefined) {
      apply(identity, cached);
      return;
    }
    if (inflight.has(key)) return;
    if (now() < (cooldownUntil.get(agentId) ?? 0)) return;
    inflight.add(key);
    activeIdentityKeys.add(identityKey);
    void (async () => {
      try {
        const sys =
          identity.kind === "goal" ? GOAL_SYSTEM_PROMPT : CURRENT_SYSTEM_PROMPT;
        // 호출 1회당 선택 provider의 사용자 구독/크레딧을 소모할 수 있다.
        const raw = await summarizeFn(identity.provider, sys, identity.sourceText);
        const summary = sanitizeSummary(raw);
        if (summary === null) {
          // 메타·깨짐·과길이 응답 — 실패로 처리(30초 쿨다운, 원문 폴백 표시).
          cooldownUntil.set(identity.agentId, now() + FAILURE_COOLDOWN_MS);
          return;
        }
        cache.set(key, summary);
        apply(identity, summary);
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        if (message.includes(`${identity.provider}-not-found`)) {
          disabledProviders.add(identity.provider);
          console.warn(
            `taskLabels: ${identity.provider} CLI 미설치 — 해당 provider 요약 비활성(원문 폴백 표시)`,
          );
        } else if (message.includes("summarizer-disabled")) {
          // 설정 OFF 경합 — 스토어 게이트가 다음 요청을 막는다. 쿨다운 불필요.
        } else {
          console.warn(
            `taskLabels: 요약 실패(kind=${identity.kind}, agent=${identity.agentId})`,
            err,
          );
          cooldownUntil.set(identity.agentId, now() + FAILURE_COOLDOWN_MS);
        }
      } finally {
        activeIdentityKeys.delete(identityKey);
        inflight.delete(key);
        sweep(useAppStore.getState().taskLabels);
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
