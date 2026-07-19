// src/renderer/diary/workLog.ts
//
// 캐릭터 일기(#56)의 원천 데이터: 세션이 실제로 무슨 일을 했는지 누적하는
// per-agent 작업 로그 버퍼. 영속 세션 이벤트 로그(session-events)는 구조 메타만
// 남기고 프롬프트·내레이션 본문이 없고, 본문 텍스트는 store의 taskLabels에
// 턴마다 리셋되는 휘발성으로만 존재한다. 그래서 요약기와 같은 store 구독 방식으로
// taskLabels 변화를 감시해 {프롬프트·목표·도구요약·내레이션, 시각}을 append한다.
//
// 버퍼는 비영속(런타임 전용) — 앱을 껐다 켜면 미생성 세션의 로그는 유실된다
// (일기 트리거 설계로 완화). agentId별 상한(항목 수)을 둬 무한 성장하지 않는다.
// 일기를 생성하고 나면 generator가 clearWorkLog로 소진한다.
import { useAppStore } from "../store/appStore";
import type { AgentTaskLabel } from "../store/types";

/** agentId당 보관하는 최근 작업 로그 항목 상한(오래된 것부터 버림). */
export const MAX_ITEMS_PER_AGENT = 60;
/** 한 항목 본문의 문자 상한(과도한 도구/내레이션 텍스트 방지). */
export const ITEM_MAX_CHARS = 400;

/** 작업 로그 한 항목의 종류. */
export type WorkLogKind = "prompt" | "tool" | "narration";

/** 작업 로그 한 항목. taskLabels의 휘발성 소스에서 캡처한 한 조각. */
export interface WorkLogItem {
  /** 캡처 시각(epoch ms). */
  at: number;
  /** 이 항목이 속한 세션. 세션 재시작 경계 추적용. */
  sessionId: string;
  kind: WorkLogKind;
  /** 항목 본문(프롬프트 원문·도구 요약·내레이션 꼬리). */
  text: string;
  /** prompt 항목일 때, 그 시점 LLM 목표(goal). 일기 서사에 방향을 준다. */
  goal?: string;
}

/** 텍스트 정제: 공백 정규화 + 상한 절단. 비면 undefined. */
function clean(raw: string | undefined): string | undefined {
  if (!raw) return undefined;
  const s = raw.replace(/\s+/g, " ").trim();
  if (!s) return undefined;
  const chars = Array.from(s);
  return chars.length > ITEM_MAX_CHARS ? chars.slice(0, ITEM_MAX_CHARS).join("") : s;
}

/**
 * per-agent 작업 로그 버퍼(순수 자료구조). 구독과 분리해 테스트가 쉽다.
 */
export class WorkLog {
  private byAgent = new Map<string, WorkLogItem[]>();

  /** 한 항목을 append하고 상한을 넘으면 오래된 것부터 버린다. */
  append(agentId: string, item: WorkLogItem): void {
    const list = this.byAgent.get(agentId) ?? [];
    list.push(item);
    if (list.length > MAX_ITEMS_PER_AGENT) {
      list.splice(0, list.length - MAX_ITEMS_PER_AGENT);
    }
    this.byAgent.set(agentId, list);
  }

  /** 한 캐릭터의 로그 항목(선택적으로 sessionId 필터). 없으면 빈 배열. */
  items(agentId: string, sessionId?: string): WorkLogItem[] {
    const list = this.byAgent.get(agentId) ?? [];
    return sessionId === undefined ? [...list] : list.filter((i) => i.sessionId === sessionId);
  }

  /** 일기 생성 후 소진. sessionId를 주면 그 세션 항목만 지운다. */
  clear(agentId: string, sessionId?: string): void {
    if (sessionId === undefined) {
      this.byAgent.delete(agentId);
      return;
    }
    const list = this.byAgent.get(agentId);
    if (!list) return;
    const kept = list.filter((i) => i.sessionId !== sessionId);
    if (kept.length) this.byAgent.set(agentId, kept);
    else this.byAgent.delete(agentId);
  }
}

/** 앱 전역 작업 로그 버퍼(단일 인스턴스). generator·recorder가 공유한다. */
export const workLog = new WorkLog();

/**
 * 로그 항목들을 일기 생성 입력용 텍스트로 조립한다. 시간순, 종류별 접두어를
 * 붙인다. 요약기 백엔드가 다시 2,000자로 캡하므로 여기선 사람이 읽는 형태만 만든다.
 */
export function formatWorkLog(items: WorkLogItem[]): string {
  const label: Record<WorkLogKind, string> = {
    prompt: "지시",
    tool: "도구",
    narration: "진행",
  };
  return items
    .map((i) => {
      const head = `- [${label[i.kind]}] ${i.text}`;
      return i.kind === "prompt" && i.goal ? `${head} (목표: ${i.goal})` : head;
    })
    .join("\n");
}

export interface WorkLogRecorderDeps {
  now?: () => number;
  /** 주입용 버퍼(테스트). 기본은 전역 workLog. */
  target?: WorkLog;
}

/**
 * store의 taskLabels를 구독해 새 프롬프트·도구요약·내레이션이 나타날 때마다
 * 작업 로그에 append한다. 앱 부트에서 1회 호출(bootstrap.ts). 해제 함수를 돌려준다.
 *
 * 멱등성: 라벨별로 마지막으로 기록한 프롬프트 시각·도구/내레이션 텍스트를 기억해
 * 값이 실제로 바뀌었을 때만 append한다(구독은 taskLabels가 조금만 바뀌어도 발화).
 */
export function installWorkLogRecorder(deps: WorkLogRecorderDeps = {}): () => void {
  const now = deps.now ?? Date.now;
  const target = deps.target ?? workLog;

  interface Seen {
    sessionId: string;
    promptAt?: number;
    tool?: string;
    narration?: string;
  }
  const seen = new Map<string, Seen>();

  function observe(labels: Record<string, AgentTaskLabel>): void {
    for (const [agentId, l] of Object.entries(labels)) {
      const stored = seen.get(agentId);
      // 세션이 바뀌면 이 agent의 이전 관측 상태를 리셋(새 세션은 처음부터 기록) —
      // latestPromptAt가 세션 간 겹쳐도 새 세션 첫 프롬프트를 놓치지 않게 baseline을 비운다.
      const sessionChanged = stored?.sessionId !== l.sessionId;
      const prev = sessionChanged ? undefined : stored;
      const next: Seen = { sessionId: l.sessionId, promptAt: prev?.promptAt };

      const promptText = clean(l.latestPromptText);
      if (promptText && l.latestPromptAt !== undefined && l.latestPromptAt !== prev?.promptAt) {
        target.append(agentId, {
          at: now(),
          sessionId: l.sessionId,
          kind: "prompt",
          text: promptText,
          goal: l.goal ?? l.goalFallback,
        });
        next.promptAt = l.latestPromptAt;
      }

      const tool = clean(l.latestToolText);
      if (tool && tool !== prev?.tool) {
        target.append(agentId, { at: now(), sessionId: l.sessionId, kind: "tool", text: tool });
      }
      next.tool = tool;

      const narration = clean(l.latestAssistantText);
      if (narration && narration !== prev?.narration) {
        target.append(agentId, {
          at: now(),
          sessionId: l.sessionId,
          kind: "narration",
          text: narration,
        });
      }
      next.narration = narration;

      seen.set(agentId, next);
    }
  }

  const off = useAppStore.subscribe((s) => s.taskLabels, observe);
  observe(useAppStore.getState().taskLabels);
  return off;
}
