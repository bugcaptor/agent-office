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

import { t } from "@renderer/i18n";
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
import {
  humanTurns,
  humanWorkedMs,
  monthKeyOf,
  monthRange,
  pickWinner,
  shiftMonth,
  type SelectionResult,
} from "./selection";
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

/**
 * 소감 생성 실패 사유(코드) → `journal` 카탈로그 키. 생성기가 내려주는 것은
 * 코드이므로 문구는 표시 시점에 만든다(diaryStore의 NOTICE_KEYS와 같은 관례).
 */
const SPEECH_ERROR_KEYS: Record<SpeechFailReason, string> = {
  "no-winner": "awards.speechError.noWinner",
  "profile-missing": "awards.speechError.profileMissing",
  disabled: "awards.speechError.disabled",
  "cli-missing": "awards.speechError.cliMissing",
  timeout: "awards.speechError.timeout",
  failed: "awards.speechError.failed",
};

/** 소감 생성 실패 사유 → 사용자 안내 문구(호출 시점에 번역). */
function speechNoticeFor(reason: SpeechFailReason): string {
  return t(`journal:${SPEECH_ERROR_KEYS[reason]}`);
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
        // 순위를 매긴 값과 같은 사람 몫을 스냅샷한다 — 같은 화면의 순위표
        // 1행과 숫자가 어긋나면 사용자가 납득할 수 없다(규칙 v2).
        workedMs: humanWorkedMs(top),
        turns: humanTurns(top),
        toolEvents: top.toolEvents,
        activeDays: top.humanActiveDays,
        tokensIn: top.tokensIn,
        tokensOut: top.tokensOut,
        costUsd: top.costUsd,
      },
    };
  }
  const leaderboard: AwardStanding[] = pick.leaderboard.map((s) => ({
    agentId: s.agentId,
    name: s.name,
    workedMs: humanWorkedMs(s),
    turns: humanTurns(s),
    activeDays: s.humanActiveDays,
    // 봇 몫이 0이면 키 자체를 넣지 않는다(v1 레코드와 같은 모양).
    ...(s.botWorkedMs > 0 ? { botWorkedMs: s.botWorkedMs } : {}),
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
  // `profiles`는 집계의 이름·색·삭제 판정에만 쓰인다 — 선정은 집계 결과(사람
  // 몫)만 본다(규칙 v2에서 프로필 기반 봇 제외를 없앴다).
  return pickWinner(data.summary);
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
      console.warn("awards: failed to load award records", err);
      set({ loaded: true, error: t("journal:awards.error.loadFailed") });
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
      console.warn("awards: backfill finalization failed", err);
      set({ error: t("journal:awards.error.finalizeFailed") });
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
      console.warn("awards: failed to compute the provisional leader", err);
      set({ error: t("journal:awards.error.provisionalFailed") });
      return null;
    }
  },

  generateSpeechFor: async (month) => {
    if (get().generating[month]) return; // 인플라이트 가드(더블클릭 등)
    const record = get().awards.find((a) => a.month === month);
    if (!record) {
      set({ error: t("journal:awards.error.noRecord") });
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
      console.warn("awards: failed to save the acceptance speech", err);
      set({ error: t("journal:awards.error.saveSpeechFailed") });
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
