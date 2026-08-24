// src/renderer/awards/awardsStore.ts
//
// "이 달의 우수사원" 전용 zustand 스토어. diary/diaryStore.ts와 같은 관례로
// appStore에서 분리했다 — 시상은 오피스 씬·세션과 무관한 독립 서브시스템이다.
// 비동기 오케스트레이션(로드/확정/소감)은 스토어 액션이 직접 tauriApi와 순수
// 로직을 호출하고, 테스트는 목으로 검증한다.
//
// 핵심 불변식: **완료된 달의 시상은 write-once**다. 한 번 확정한 달은 다시
// 계산하지 않는다(이벤트 보존 기간이 지나면 같은 수치가 안 나오고, 지난 수상이
// 흔들리면 기록의 의미가 없다). 그래서 후보가 한 명도 없던 달도 `winner: null`로
// 확정해 "계산했고 수상자가 없었다"를 남긴다.
import { create } from "zustand";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import {
  aggregate,
  localDayCalendar,
  type DayCalendar,
} from "../analytics/aggregate";
import {
  AWARD_RULES_VERSION,
  type AgentProfile,
  type AwardRecord,
  type AwardSpeech,
  type AwardStanding,
  type AwardWinner,
  type AwardsFile,
  type SessionEventRecord,
} from "@shared/types";
import { monthKeyOf, monthRange, pickWinner, shiftMonth, type SelectionResult } from "./selection";
import { generateSpeech, type SpeechFailReason } from "./speechGenerator";

/**
 * 시상 IPC 계약. **아직 tauriApi에 이 네 함수가 없다**(배선은 다른 작업 몫).
 * 계약을 여기 한 군데 좁게 적어 두고 런타임 객체에 그대로 위임한다 —
 * tauriApi에 함수가 추가되면 이 인터페이스와 `awardsApi` 캐스트를 지우고
 * `tauriApi.` 직접 호출로 바꾸면 된다.
 */
interface AwardsApi {
  loadAwards(): Promise<AwardsFile>;
  finalizeAward(record: AwardRecord, portraitAgentId?: string): Promise<AwardsFile>;
  appendAwardSpeech(month: string, speech: AwardSpeech): Promise<AwardsFile>;
  loadAwardPortrait(month: string): Promise<string | null>;
}

const awardsApi = tauriApi as unknown as AwardsApi;

/**
 * 경계를 걸친 턴을 온전히 복원하려고 표시 창보다 앞선 이벤트까지 함께 읽는다
 * (AnalyticsDialog와 같은 값·같은 이유).
 */
const LOOKBACK_MS = 24 * 60 * 60 * 1000;

/** 소급 확정 상한. 처음 켠 사용자에게 몇 년치를 한꺼번에 계산시키지 않는다. */
export const BACKFILL_MONTHS = 12;

/** 진행 중인 달의 잠정 결과(저장하지 않는다). */
export interface ProvisionalAward extends SelectionResult {
  month: string;
  /** 잠정 결과를 계산한 시각(epoch ms) — "N시 기준" 표시용. */
  at: number;
}

/** 소감 생성 실패 사유 → 사용자 안내 문구. */
function speechNoticeFor(reason: SpeechFailReason): string {
  switch (reason) {
    case "no-winner":
      return "그 달은 수상자가 없어 소감을 쓸 수 없습니다.";
    case "profile-missing":
      return "수상자 캐릭터가 남아 있지 않아 소감을 쓸 수 없습니다.";
    case "disabled":
      return "설정에서 요약기를 먼저 켜 주세요.";
    case "cli-missing":
      return "선택한 CLI를 찾지 못해 소감을 쓰지 못했습니다.";
    case "timeout":
      return "생성이 오래 걸려 실패했습니다. 잠시 후 다시 시도하세요.";
    case "failed":
      return "소감 생성에 실패했습니다. 잠시 후 다시 시도하세요.";
  }
}

/** 현재 프로필 맵. appStore를 액션 안에서 늦게 읽어 모듈 로드 순서에 얽히지 않는다. */
function currentProfiles(): Record<string, AgentProfile> {
  return useAppStore.getState().agents;
}

interface AwardsState {
  /** month 오름차순. 백엔드가 돌려준 파일을 그대로 담는다. */
  awards: AwardRecord[];
  loaded: boolean;
  /** 소급 확정이 도는 중. */
  finalizing: boolean;
  /** 소감 생성 인플라이트(month → true). 같은 달 중복 호출을 막는다. */
  generating: Record<string, boolean>;
  /** 마지막 실패 안내. 성공하면 지운다. */
  error?: string;

  /** 시상 파일을 읽는다. */
  load(): Promise<void>;
  /**
   * 완료된 달 중 레코드가 없는 달을 오래된 것부터 소급 확정한다(최대 12개월).
   * 이미 레코드가 있는 달은 건드리지 않고, 진행 중인 이번 달은 확정하지 않는다.
   */
  ensureFinalized(now?: number): Promise<void>;
  /** 진행 중인 이번 달의 잠정 1위를 라이브로 계산한다(저장하지 않음). */
  provisionalWinner(now?: number): Promise<ProvisionalAward | null>;
  /** 그 달의 수상 소감을 생성해 append한다. */
  generateSpeechFor(month: string): Promise<void>;
  /** 그 캐릭터의 통산 수상 횟수(ProfileDialog 뱃지용 셀렉터). */
  awardCountFor(agentId: string): number;
  /** 그 달의 확정 레코드(없으면 undefined). */
  recordFor(month: string): AwardRecord | undefined;
}

/** 한 달치 집계 → 확정 레코드. 표시에 필요한 값을 전부 스냅샷한다. */
function buildRecord(
  month: string,
  pick: SelectionResult,
  profiles: Record<string, AgentProfile>,
  decidedAt: number,
): AwardRecord {
  const top = pick.winner;
  let winner: AwardWinner | null = null;
  if (top) {
    const profile = profiles[top.agentId];
    winner = {
      agentId: top.agentId,
      // 이름은 집계가 이미 프로필/스냅샷 폴백을 거친 값이다.
      name: top.name,
      role: profile?.role ?? "",
      ...(profile?.archetype ? { archetype: profile.archetype } : {}),
      // 초상이 있으면 백엔드가 확정 시점 스냅샷을 떠 둔다.
      hasPortrait: profile?.portraitUpdatedAt !== undefined,
      stats: {
        workedMs: top.workedMs,
        turns: top.turns,
        toolEvents: top.toolEvents,
        activeDays: top.activeDays,
        tokensIn: top.tokensIn,
        tokensOut: top.tokensOut,
        costUsd: top.costUsd,
      },
    };
  }
  const leaderboard: AwardStanding[] = pick.leaderboard.map((s) => ({
    agentId: s.agentId,
    name: s.name,
    workedMs: s.workedMs,
    turns: s.turns,
    activeDays: s.activeDays,
  }));
  return {
    month,
    decidedAt,
    rulesVersion: AWARD_RULES_VERSION,
    winner,
    leaderboard,
    speeches: [],
  };
}

/** 한 달 창을 집계해 선정 결과를 낸다. 이벤트 조회부터 여기서 한다. */
async function selectForRange(
  fromAt: number,
  toAt: number,
  profiles: Record<string, AgentProfile>,
  cal: DayCalendar,
): Promise<SelectionResult> {
  const events: SessionEventRecord[] = await tauriApi.loadSessionEvents(fromAt - LOOKBACK_MS, toAt);
  const data = aggregate(events, profiles, cal, { fromAt, toAt });
  return pickWinner(data.summary, profiles);
}

export const useAwardsStore = create<AwardsState>((set, get) => ({
  awards: [],
  loaded: false,
  finalizing: false,
  generating: {},
  error: undefined,

  load: async () => {
    try {
      const file = await awardsApi.loadAwards();
      set({ awards: file.awards, loaded: true, error: undefined });
    } catch (err) {
      console.warn("awards: 시상 기록 로드 실패", err);
      set({ loaded: true, error: "시상 기록을 불러오지 못했습니다." });
    }
  },

  ensureFinalized: async (now = Date.now()) => {
    if (get().finalizing) return;
    if (!get().loaded) await get().load();

    const cal = localDayCalendar;
    const current = monthKeyOf(now, cal);
    const have = new Set(get().awards.map((a) => a.month));
    // 완료된 달만, 오래된 것부터. 이번 달은 아직 안 끝났으니 제외한다.
    const missing: string[] = [];
    for (let back = BACKFILL_MONTHS; back >= 1; back--) {
      const month = shiftMonth(current, -back);
      if (!have.has(month)) missing.push(month);
    }
    if (missing.length === 0) return;

    set({ finalizing: true });
    try {
      const profiles = currentProfiles();
      for (const month of missing) {
        const { fromAt, toAt } = monthRange(month, cal);
        const pick = await selectForRange(fromAt, toAt, profiles, cal);
        const record = buildRecord(month, pick, profiles, now);
        // 이벤트가 없던 달도 winner:null로 확정한다 — 다음 실행에서 같은 달을
        // 또 계산하지 않게 하는 것이 목적이다.
        const file = await awardsApi.finalizeAward(record, pick.winner?.agentId);
        set({ awards: file.awards });
      }
      set({ error: undefined });
    } catch (err) {
      console.warn("awards: 소급 확정 실패", err);
      set({ error: "지난달 시상을 확정하지 못했습니다." });
    } finally {
      set({ finalizing: false });
    }
  },

  provisionalWinner: async (now = Date.now()) => {
    const cal = localDayCalendar;
    const month = monthKeyOf(now, cal);
    const range = monthRange(month, cal);
    try {
      // 진행 중인 달이라 상한은 "지금"이다(미래 구간까지 창을 열 이유가 없다).
      const pick = await selectForRange(
        range.fromAt,
        Math.min(range.toAt, now),
        currentProfiles(),
        cal,
      );
      return { month, at: now, ...pick };
    } catch (err) {
      console.warn("awards: 잠정 1위 계산 실패", err);
      set({ error: "이번 달 현황을 계산하지 못했습니다." });
      return null;
    }
  },

  generateSpeechFor: async (month) => {
    if (get().generating[month]) return; // 인플라이트 가드(더블클릭 등)
    const record = get().awards.find((a) => a.month === month);
    if (!record) {
      set({ error: "그 달의 시상 기록이 없습니다." });
      return;
    }
    const agentId = record.winner?.agentId;
    const profile = agentId ? currentProfiles()[agentId] : undefined;
    // 같은 캐릭터가 이 달 **이전에** 받은 횟수 — 이번 수상은 세지 않는다.
    const prior = agentId
      ? get().awards.filter((a) => a.winner?.agentId === agentId && a.month < month).length
      : 0;

    set((s) => ({ generating: { ...s.generating, [month]: true }, error: undefined }));
    try {
      const result = await generateSpeech(record, profile, prior);
      if (!result.ok) {
        set({ error: speechNoticeFor(result.reason) });
        return;
      }
      const file = await awardsApi.appendAwardSpeech(month, result.speech);
      set({ awards: file.awards, error: undefined });
    } catch (err) {
      console.warn("awards: 수상 소감 저장 실패", err);
      set({ error: "수상 소감을 저장하지 못했습니다." });
    } finally {
      set((s) => {
        const next = { ...s.generating };
        delete next[month];
        return { generating: next };
      });
    }
  },

  awardCountFor: (agentId) => get().awards.filter((a) => a.winner?.agentId === agentId).length,

  recordFor: (month) => get().awards.find((a) => a.month === month),
}));
