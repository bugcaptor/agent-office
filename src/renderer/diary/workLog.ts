// src/renderer/diary/workLog.ts
//
// 캐릭터 일기(#56)의 원천 데이터: 세션이 실제로 무슨 일을 했는지 누적하는
// per-agent 작업 로그 버퍼. 영속 세션 이벤트 로그(session-events)는 구조 메타만
// 남기고 프롬프트·내레이션 본문이 없고, 본문 텍스트는 store의 taskLabels에
// 턴마다 리셋되는 휘발성으로만 존재한다. 그래서 요약기와 같은 store 구독 방식으로
// taskLabels 변화를 감시해 {프롬프트·목표·도구요약·내레이션, 시각}을 append한다.
//
// 버퍼는 비영속(런타임 전용) — 앱을 껐다 켜면 미생성 세션의 로그는 유실된다
// (일기 트리거 설계로 완화). 상한을 둬 무한 성장하지 않되, 축출은 **세션 인지형**
// 이다(#75): 새 세션 활동이 아직 일기화 안 된 옛 세션 항목을 밀어내 유실시키지
// 않도록, 세션당 항목 상한 + 세션 개수 상한으로 이원화한다. 일기를 생성하고 나면
// generator가 clear로 소진한다.
import { useAppStore } from "../store/appStore";
import type { AgentTaskLabel } from "../store/types";
import type { WorkLogItem, WorkLogKind } from "@shared/types";

// 정본 타입은 @shared/types(백엔드 스냅샷과 공유). 기존 임포터 호환을 위해 재export.
export type { WorkLogItem, WorkLogKind } from "@shared/types";

/**
 * 한 세션이 보관하는 최근 작업 로그 항목 상한(넘으면 그 세션의 오래된 것부터 버림).
 * 예전 `MAX_ITEMS_PER_AGENT`(에이전트 전체 60)를 세션 단위로 옮긴 값 — 단일 세션
 * 동작은 동일하되, 여러 세션이 한 에이전트를 공유해도 서로 밀어내지 않게 한다(#75).
 */
export const MAX_ITEMS_PER_SESSION = 60;
/**
 * 한 에이전트가 보관하는 서로 다른 세션 개수 상한(넘으면 가장 오래된 세션을 통째로
 * 버림). 재시작을 아주 많이 반복해도 미기록 세션이 무한 누적하지 않게 하는 메모리
 * 방어선. 최악 메모리 = MAX_SESSIONS_PER_AGENT × MAX_ITEMS_PER_SESSION 항목(#75).
 */
export const MAX_SESSIONS_PER_AGENT = 40;
/** 한 항목 본문의 문자 상한(과도한 도구/내레이션 텍스트 방지). */
export const ITEM_MAX_CHARS = 400;

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
 *
 * 변이(append/clear/seed)마다 `onChange(agentId)`를 발화한다 — 영속화기
 * (`workLogPersister`)가 이 훅으로 dirty를 감지해 디바운스 저장한다. 훅은
 * 선택적(테스트/영속화 미설치 시 no-op)이라 순수 자료구조 성질을 유지한다.
 */
export class WorkLog {
  private byAgent = new Map<string, WorkLogItem[]>();
  /** 변이 알림 훅(영속화기가 설치). 없으면 no-op. */
  private onChange?: (agentId: string) => void;

  /** 변이 알림 훅을 설치한다(영속화기 1개 전제). 해제는 undefined로 재설정. */
  setOnChange(cb: ((agentId: string) => void) | undefined): void {
    this.onChange = cb;
  }

  /**
   * 세션 인지형 축출(#75). 시간순(오래→최신) 리스트를 제자리에서 절단한다:
   *  (1) 세션당 항목 상한 — 각 세션이 MAX_ITEMS_PER_SESSION를 넘으면 그 세션의
   *      가장 오래된 항목부터 버린다(최신 우선 보존). 다른 세션엔 손대지 않는다.
   *  (2) 세션 개수 상한 — 서로 다른 세션이 MAX_SESSIONS_PER_AGENT를 넘으면 가장
   *      오래된 세션(등장순 앞)부터 통째로 버린다. 여기서만 "세션 유실"이 일어나되,
   *      새 세션 활동이 아닌 세션 수 누적이 원인일 때뿐이다.
   * 리스트는 항목 수가 크지 않아(수천 이하) 매 변이 O(n) 전량 스캔으로 충분하다.
   */
  private static bound(list: WorkLogItem[]): void {
    // (1) 세션당 항목 상한 — 뒤(최신)에서부터 세며 상한 초과 과거 항목을 제거.
    const perSession = new Map<string, number>();
    for (let i = list.length - 1; i >= 0; i--) {
      const sid = list[i].sessionId;
      const n = (perSession.get(sid) ?? 0) + 1;
      perSession.set(sid, n);
      if (n > MAX_ITEMS_PER_SESSION) list.splice(i, 1);
    }
    // (2) 세션 개수 상한 — 등장순으로 세션을 세어 오래된 세션을 통째로 드롭.
    if (perSession.size > MAX_SESSIONS_PER_AGENT) {
      const order: string[] = [];
      const seen = new Set<string>();
      for (const it of list) {
        if (!seen.has(it.sessionId)) {
          seen.add(it.sessionId);
          order.push(it.sessionId);
        }
      }
      const drop = new Set(order.slice(0, order.length - MAX_SESSIONS_PER_AGENT));
      if (drop.size > 0) {
        for (let i = list.length - 1; i >= 0; i--) {
          if (drop.has(list[i].sessionId)) list.splice(i, 1);
        }
      }
    }
  }

  /** 한 항목을 append하고 세션 인지 상한(#75)을 적용한다. */
  append(agentId: string, item: WorkLogItem): void {
    const list = this.byAgent.get(agentId) ?? [];
    list.push(item);
    WorkLog.bound(list);
    this.byAgent.set(agentId, list);
    this.onChange?.(agentId);
  }

  /**
   * 디스크 복원분을 버퍼 **앞에** 채운다(부팅 복원). 복원 항목은 항상 과거
   * 세션이므로 prepend가 시간순을 유지한다 — recorder 설치와의 async 순서
   * 경합(이미 새 항목이 들어온 경우)에도 안전하다. 상한을 넘으면 오래된 것부터
   * 버린다. 복원은 디스크→메모리 반영이므로 onChange를 발화하지 않는다(재저장 불필요).
   */
  seed(agentId: string, items: WorkLogItem[]): void {
    if (items.length === 0) return;
    const list = this.byAgent.get(agentId) ?? [];
    const merged = [...items, ...list];
    WorkLog.bound(merged);
    this.byAgent.set(agentId, merged);
  }

  /** 한 캐릭터의 로그 항목(선택적으로 sessionId 필터). 없으면 빈 배열. */
  items(agentId: string, sessionId?: string): WorkLogItem[] {
    const list = this.byAgent.get(agentId) ?? [];
    return sessionId === undefined ? [...list] : list.filter((i) => i.sessionId === sessionId);
  }

  /** 이 캐릭터의 서로 다른 sessionId 목록(중복 제거, 등장순). */
  sessions(agentId: string): string[] {
    const list = this.byAgent.get(agentId);
    if (!list) return [];
    const seen = new Set<string>();
    const out: string[] = [];
    for (const i of list) {
      if (!seen.has(i.sessionId)) {
        seen.add(i.sessionId);
        out.push(i.sessionId);
      }
    }
    return out;
  }

  /** 일기 생성 후 소진. sessionId를 주면 그 세션 항목만 지운다. */
  clear(agentId: string, sessionId?: string): void {
    if (sessionId === undefined) {
      if (!this.byAgent.has(agentId)) return;
      this.byAgent.delete(agentId);
      this.onChange?.(agentId);
      return;
    }
    const list = this.byAgent.get(agentId);
    if (!list) return;
    const kept = list.filter((i) => i.sessionId !== sessionId);
    if (kept.length === list.length) return; // 변화 없음 — 발화 생략.
    if (kept.length) this.byAgent.set(agentId, kept);
    else this.byAgent.delete(agentId);
    this.onChange?.(agentId);
  }
}

/** 앱 전역 작업 로그 버퍼(단일 인스턴스). generator·recorder가 공유한다. */
export const workLog = new WorkLog();

/**
 * 일기 입력 조립 시 목표 예산(문자). 백엔드 캡(2,000자, `cap_text`)보다 낮게 잡아
 * 대부분 케이스에서 백엔드 절단을 타지 않게 한다.
 */
export const FORMAT_BUDGET_CHARS = 1_900;

/**
 * 로그 항목들을 일기 생성 입력용 텍스트로 조립한다. 시간순, 종류별 접두어를 붙인다.
 *
 * 예산(FORMAT_BUDGET_CHARS) 이내면 전부 그대로 잇는다. 초과하면 **우선순위 기반
 * 축소**(#66): (1) `prompt`(+목표)는 일기의 뼈대이므로 전량 보존, (2) 남은 예산에
 * `tool`/`narration`을 **최신 우선**으로 채우되 출력은 시간순 유지, (3) 탈락한
 * 구간은 `- (중략: N개 항목)` 한 줄로 표시. 예전에는 백엔드가 앞 2,000자만 남기는
 * 꼬리 절단이라 긴 세션의 최신 작업이 통째로 유실됐다.
 */
export function formatWorkLog(items: WorkLogItem[]): string {
  const label: Record<WorkLogKind, string> = {
    prompt: "지시",
    tool: "도구",
    narration: "진행",
  };
  const render = (i: WorkLogItem): string => {
    const head = `- [${label[i.kind]}] ${i.text}`;
    return i.kind === "prompt" && i.goal ? `${head} (목표: ${i.goal})` : head;
  };

  const lines = items.map(render);
  const totalCost = lines.reduce((n, l) => n + l.length + 1, 0); // +1 = 개행
  if (totalCost <= FORMAT_BUDGET_CHARS) return lines.join("\n");

  // 예산 초과 — 우선순위 선별.
  const kept = new Set<number>();
  let used = 0;
  // 1) prompt는 예산과 무관하게 전량 확보(지시·목표는 반드시 남긴다).
  items.forEach((it, idx) => {
    if (it.kind === "prompt") {
      kept.add(idx);
      used += lines[idx].length + 1;
    }
  });
  // 2) 남은 예산을 tool/narration으로 최신(뒤)→과거 순 채운다. 안 맞는 항목은
  //    건너뛰고 더 과거의(작을 수 있는) 항목을 계속 시도 — 정보량 최대화.
  for (let idx = items.length - 1; idx >= 0; idx--) {
    if (kept.has(idx)) continue;
    const cost = lines[idx].length + 1;
    if (used + cost > FORMAT_BUDGET_CHARS) continue;
    kept.add(idx);
    used += cost;
  }

  // 시간순 출력 + 탈락 구간을 중략 한 줄로 접기.
  const out: string[] = [];
  let dropRun = 0;
  items.forEach((_, idx) => {
    if (kept.has(idx)) {
      if (dropRun > 0) {
        out.push(`- (중략: ${dropRun}개 항목)`);
        dropRun = 0;
      }
      out.push(lines[idx]);
    } else {
      dropRun += 1;
    }
  });
  if (dropRun > 0) out.push(`- (중략: ${dropRun}개 항목)`);
  return out.join("\n");
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
