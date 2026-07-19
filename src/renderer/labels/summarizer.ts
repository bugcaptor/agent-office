// src/renderer/labels/summarizer.ts
//
// 머리 위 라벨용 LLM 요약기. 설정에서 캡처한 로컬 CLI provider로 요약한다.
// store의 taskLabels를 구독해 프롬프트마다 한 번의 통합 호출로 goal(세션
// 목표)과 currentSummary(현재 명령 요약)를 함께 채운다.
//
// 통합 호출 계약: latestPromptText가 있고 currentSummary가 비어 있는 라벨마다
// 이전 goal을 컨텍스트로 넘겨 목표+현재를 한 번에 받는다. 새 지시가 이전
// 작업의 후속·보완이거나 애매하면 이전 목표를 유지하는 바이어스를 둔다
// (goal이 첫 프롬프트에 동결되지 않고 세션이 작업을 갈아타면 따라온다).
// 호출 횟수는 기존 current 요약과 같으므로 순감소다.
//
// 실패 정책: 선택된 CLI 미설치("${provider}-not-found") → 그 provider만 앱
// 실행 동안 비활성(원문 폴백은 UI 몫). 그 외 호출 실패 → agent별 30초
// 쿨다운 후 다음 store 변화 때 재시도한다. 응답이 두 줄이 아니거나 한 줄이라도
// 메타 발언·깨짐·과길이라 sanitizeLabelPair가 거부하면(null) 동일하게 30초
// 쿨다운 후 재시도한다.
//
// opt-in 게이트: appStore.appSettings.summarizerEnabled=false면 요청 자체를
// 보내지 않는다(원문 폴백). ON 상태에서 레이스로 "summarizer-disabled"
// 에러를 받으면(설정이 막 꺼진 경합) 쿨다운/영구비활성 없이 그냥 무시한다 —
// 스토어 설정이 단일 진실원이므로 다음 요청은 게이트가 이미 최신값으로 막거나 통과시킨다.
import { useAppStore } from "../store/appStore";
import type { AgentTaskLabel } from "../store/types";
import { tauriApi } from "../ipc/tauriApi";
import type { SummaryProvider } from "@shared/types";

export const LABEL_SYSTEM_PROMPT =
  "너는 코딩 세션 라벨 생성기다. [이전 목표]와 [새 지시]를 보고 정확히 두 줄을 출력하라. 1줄: 세션 목표 — 새 지시가 새로운 작업이면 한국어 명사구 12자 이내로 새로 뽑고, 이전 작업의 후속·보완 지시이거나 판단이 애매하면 이전 목표를 그대로 출력하라. 이전 목표가 (없음)이면 새로 뽑아라. 2줄: 새 지시 요약 — 한국어 18자 이내 한 줄. 규칙: 정확히 두 줄, 한국어만, 사과·설명·따옴표·번호·머리말 금지. 판단 불가면 1줄은 이전 목표(없으면 '작업 중'), 2줄은 '작업 중'. 예) 이전 목표: 로그인 버그 수정 / 새 지시: 테스트도 고쳐줘 → 1줄 '로그인 버그 수정', 2줄 '테스트 수정'";
const FAILURE_COOLDOWN_MS = 30_000;
const SUMMARY_MAX_CHARS = 40;
const META_MARKERS = ["인코딩", "죄송", "할 수 없"];

/** 한 줄을 라벨용으로 정제. 따옴표·머리말 제거, 메타·깨짐·과길이는 null. */
function sanitizeLine(line: string): string | null {
  const s = line
    .trim()
    .replace(/^["'`]+|["'`]+$/g, "")
    .replace(/^(1줄|2줄|요약|목표)\s*[:：]\s*/, "")
    .trim();
  if (!s) return null;
  if (Array.from(s).length > SUMMARY_MAX_CHARS) return null;
  if (s.includes("�") || /\?{2,}/.test(s) || /^[\s?]+$/.test(s)) return null;
  if (META_MARKERS.some((m) => s.includes(m))) return null;
  return s;
}

/** LLM 응답 첫 비공백 줄을 라벨용으로 정제(단일 줄 정제). 거부 시 null. */
export function sanitizeSummary(raw: string): string | null {
  const firstLine = raw.split("\n").map((l) => l.trim()).find((l) => l.length > 0);
  if (!firstLine) return null;
  return sanitizeLine(firstLine);
}

/**
 * 통합 응답을 목표/현재 쌍으로 정제. 비공백 줄 앞 2개를 취하며, 2줄 미만이거나
 * 한 줄이라도 sanitizeLine이 거부하면 전체 null(쿨다운 재시도).
 */
export function sanitizeLabelPair(raw: string): { goal: string; current: string } | null {
  const lines = raw
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l.length > 0)
    .slice(0, 2);
  if (lines.length < 2) return null;
  const goal = sanitizeLine(lines[0]);
  const current = sanitizeLine(lines[1]);
  if (goal === null || current === null) return null;
  return { goal, current };
}

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
  sessionId: string;
  sourceText: string; // latestPromptText
  latestPromptAt?: number;
  prevGoal: string | null; // 통합 호출에 넘기는 이전 목표 컨텍스트
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

  const cache = new Map<string, { goal: string; current: string }>(); // `${provider}|${prevGoal}|${원문}` -> 쌍
  const inflight = new Set<string>(); // cache와 같은 키
  // provider 변경과 무관한 Agent Office identity별(= 어떤 프롬프트를 처리 중인지) 활성 요청 소유권.
  const activeIdentityKeys = new Set<string>();
  const cooldownUntil = new Map<string, number>(); // agentId -> epoch ms
  const disabledProviders = new Set<SummaryProvider>();

  function activeIdentityKey(identity: RequestIdentity): string {
    return JSON.stringify([
      identity.agentId,
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
    return (
      label.latestPromptText === identity.sourceText &&
      label.latestPromptAt === identity.latestPromptAt
    );
  }

  /** 캡처한 Agent Office identity가 그대로일 때만 store에 반영한다. */
  function apply(identity: RequestIdentity, pair: { goal: string; current: string }): void {
    const label = useAppStore.getState().taskLabels[identity.agentId];
    if (!isCurrent(identity, label)) return;
    useAppStore
      .getState()
      .setTaskLabelSummary(identity.agentId, { goal: pair.goal, currentSummary: pair.current });
  }

  function request(agentId: string, text: string): void {
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
      sessionId: label.sessionId,
      sourceText: text,
      latestPromptAt: label.latestPromptAt,
      prevGoal: label.goal ?? null,
    };
    const identityKey = activeIdentityKey(identity);
    if (activeIdentityKeys.has(identityKey)) return;
    const key = `${identity.provider}|${identity.prevGoal ?? ""}|${identity.sourceText}`;
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
        const userText = `[이전 목표]\n${identity.prevGoal ?? "(없음)"}\n[새 지시]\n${identity.sourceText}`;
        // 호출 1회당 선택 provider의 사용자 구독/크레딧을 소모할 수 있다.
        const raw = await summarizeFn(identity.provider, LABEL_SYSTEM_PROMPT, userText);
        const pair = sanitizeLabelPair(raw);
        if (pair === null) {
          // 두 줄 미만·메타·깨짐·과길이 응답 — 실패로 처리(30초 쿨다운, 원문 폴백 표시).
          cooldownUntil.set(identity.agentId, now() + FAILURE_COOLDOWN_MS);
          return;
        }
        cache.set(key, pair);
        apply(identity, pair);
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
            `taskLabels: 요약 실패(agent=${identity.agentId})`,
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

  /** 요약이 비어 있는 라벨을 훑어 필요한 통합 요청을 낸다(멱등). */
  function sweep(labels: Record<string, AgentTaskLabel>): void {
    for (const [agentId, l] of Object.entries(labels)) {
      if (l.latestPromptText && l.currentSummary === undefined)
        request(agentId, l.latestPromptText);
    }
  }

  const off = useAppStore.subscribe((s) => s.taskLabels, sweep);
  sweep(useAppStore.getState().taskLabels);
  return off;
}
