// src/renderer/guiHarness/scenarios.ts
//
// 하네스 시나리오 — 부팅 시 가짜 백엔드가 돌려줄 초기 데이터. 응답 모양은
// Rust serde 출력과 대조되는 `@shared/contract-fixtures`를 바탕으로 만든다
// (손으로 쓴 모양이 백엔드와 어긋나 화면이 거짓으로 깨지거나 멀쩡해 보이지 않게).
// URL `?scenario=<id>&lang=<code>`로 고른다.
import { IS_WINDOWS } from "../shared/platform";
import type {
  AdoptedSessionInfo,
  AgentProfile,
  AwardsFile,
  Discussion,
  DiscussionsFile,
  GetAppSettingsResult,
  NotificationEvent,
  PersistedState,
  UsageSnapshot,
  WorkItem,
} from "@shared/types";
import settingsFixture from "@shared/contract-fixtures/get-app-settings-result.json";
import awardsFixture from "@shared/contract-fixtures/awards-file.json";
import discussionsFixture from "@shared/contract-fixtures/discussions-file.json";
import usageFixture from "@shared/contract-fixtures/usage-snapshot.json";

export interface Scenario {
  state: PersistedState;
  settings: GetAppSettingsResult;
  awards: AwardsFile;
  usage: UsageSnapshot;
  /** sessiond에서 되찾은 것처럼 부팅 때 Running으로 시드할 세션. */
  adopted: AdoptedSessionInfo[];
  /** 세션 구독이 붙으면 흘려보낼 터미널 출력(agentId → ANSI 텍스트). */
  output: Record<string, string>;
  /** 부팅 직후 도착시킬 알림. */
  notifications: NotificationEvent[];
  /** 업무 패널의 저장된 업무 목록. */
  workItems: WorkItem[];
  /** 저장된 팀 토의. */
  discussions: Discussion[];
}

const harnessCwd = IS_WINDOWS ? "C:/tmp/gui-harness" : "/tmp/gui-harness";

const profile = (id: string, name: string, seed: string, deskIndex: number): AgentProfile => ({
  id,
  name,
  role: "engineer",
  seed,
  createdAt: 1720000000000 + deskIndex,
  deskIndex,
  cwd: harnessCwd,
});

const AGENTS = [
  profile("p1", "Ada", "abc123", 0),
  profile("p2", "Bo", "zz9", 1),
  profile("p3", "Cy", "q7", 2),
];

function settings(firstRun: boolean, lang: string): GetAppSettingsResult {
  const base = settingsFixture as GetAppSettingsResult;
  return { firstRun, settings: { ...base.settings, language: lang } };
}

function base(lang: string): Scenario {
  return {
    state: { version: 1, agents: AGENTS, vacationMode: false },
    settings: settings(false, lang),
    awards: awardsFixture as AwardsFile,
    usage: usageFixture as UsageSnapshot,
    adopted: [],
    output: {},
    notifications: [],
    workItems: [],
    discussions: [],
  };
}

/** Claude 5시간 창을 96%로, 리셋을 95분 뒤로(가짜 백엔드가 시각을 지금으로 옮긴 뒤 기준). 사용량 대기 장면용. */
function nearLimitClaude(usage: UsageSnapshot): UsageSnapshot {
  const lastAttempt = usage.claudeLive.lastAttemptMs ?? 0;
  const windows = (usage.claude?.windows ?? []).map((w) => (w.kind === "session" ? { ...w, usedPercent: 96, resetsAtMs: lastAttempt + 60_000 + 95 * 60_000 } : w));
  return { ...usage, claude: usage.claude && { ...usage.claude, windows } };
}

const work = (id: string, title: string, status: WorkItem["status"], assigneeAgentId: string | undefined, order: number, extra: Partial<WorkItem> = {}): WorkItem => ({
  id, title, body: `${title} body`, status, assigneeAgentId, order, sessionIds: [], createdAt: 1720000000000 + order, updatedAt: 1720000000000 + order, ...extra,
});

const SCENARIOS: Record<string, (lang: string) => Scenario> = {
  /** 업무 패널을 켜고 캐릭터마다 다른 프로젝트 폴더와 업무를 둔 사무실. */
  workpanels: (lang) => {
    const b = base(lang);
    const dev = IS_WINDOWS ? "C:/Users/me/dev" : "/Users/me/dev";
    const cwds = [`${dev}/agent-office`, `${dev}/kbm-server`, undefined];
    return {
      ...b,
      state: { ...b.state, agents: AGENTS.map((a, i) => ({ ...a, cwd: cwds[i] })) },
      settings: { ...b.settings, settings: { ...b.settings.settings, workPanelsEnabled: true } },
      usage: nearLimitClaude(b.usage),
      // 끝난 토의(결정 메모 있음)와 준비 상태 토의. 참가자 p1~p3은 AGENTS와 같다.
      discussions: (discussionsFixture as DiscussionsFile).discussions,
      workItems: [
        work("w1", "Show project in work panel", "in_progress", "p1", 0),
        work("w2", "Tidy session log filters", "todo", "p1", 1),
        work("w3", "Add GUI harness scenario", "review", "p1", 2),
        work("w4", "Rebuild search index", "todo", "p2", 3, { dependsOn: ["w5"] }),
        work("w5", "DB migration", "todo", "p2", 4),
        work("w6", "Old chore", "done", "p2", 5),
        work("w7", "", "todo", undefined, 6, { body: "Unassigned note: things to sort out next sprint" }),
        // 자동 진행 루트: w4 ← w5를 Bo가 진행하다 판단자가 멈춘 상태.
        work("w8", "Ship search", "todo", "p2", 7, { dependsOn: ["w4"], body: "Keep the old index until the new one is verified.", autoRun: {
          state: "paused", judgeSessionId: "00000000-0000-4000-8000-000000000000", judgeStarted: true, currentId: "w5", followUps: 1,
          pauseReason: "judge-stop", pauseDetail: "The migration drops a column that production still reads. Confirm before continuing.", costUsd: 0.042, startedAt: 1720000000000,
          log: [
            { at: 1720000000000, kind: "start" },
            { at: 1720000060000, kind: "send", taskId: "w5" },
            { at: 1720000900000, kind: "followup", taskId: "w5", text: "Run the migration against a copy of the schema and report the result." },
            { at: 1720001500000, kind: "pause", text: "judge-stop" },
          ],
        } }),
        // 플랜 루트(메모 전용): w2를 마치면 끝나고 루트 자체는 실행하지 않는다.
        work("w9", "Clean up logs", "todo", "p1", 8, { dependsOn: ["w2"], plan: true, body: "Goal: session logs stay readable. Keep the old filter names working." }),
        // 사용량 대기: Cy의 플랜이 Claude 5시간 창이 모자랄 것 같아 리셋을 기다린다.
        work("w10", "Write release notes", "done", "p3", 9),
        work("w11", "Publish changelog page", "todo", "p3", 10, { dependsOn: ["w10"] }),
        work("w12", "Release 2.0", "todo", "p3", 11, { dependsOn: ["w11"], plan: true, body: "Ship docs together with the release." , autoRun: {
          state: "paused", judgeSessionId: "00000000-0000-4000-8000-000000000001", judgeStarted: true, followUps: 0, pauseReason: "usage-wait", costUsd: 0.018,
          startedAt: Date.now() - 2 * 3600_000, usageGuard: true,
          usageWait: { provider: "claude", kind: "session", label: null, nowPercent: 78.4, addPercent: 21.3, limit: 95, until: Date.now() + 95 * 60_000 + 120_000, resetsAtMs: Date.now() + 95 * 60_000, stage: "task", worker: "claude" },
          log: [
            { at: Date.now() - 2 * 3600_000, kind: "start" },
            { at: Date.now() - 2 * 3600_000 + 60_000, kind: "send", taskId: "w10" },
            { at: Date.now() - 20 * 60_000, kind: "next", taskId: "w10", text: "Notes live in docs/release-2.0.md." },
            { at: Date.now() - 20 * 60_000, kind: "pause", text: "usage-wait" },
          ],
        } }),
        // 예약: 매일 반복 템플릿(켜짐, 지난 회차 하나), CLI가 만든 꺼진 예약, 한 번 예약.
        work("w13", "Morning dependency audit", "todo", "p1", 12, { body: "Run npm audit and cargo audit; leave a task for anything new.", schedule: {
          rule: { kind: "weekly", days: [1, 2, 3, 4, 5], time: "09:00" }, enabled: true, nextAt: Date.now() + 14 * 3600_000, lastRunAt: Date.now() - 10 * 3600_000,
          lastSkip: { at: Date.now() - 34 * 3600_000, reason: "missed" },
        } }),
        work("w14", "Morning dependency audit", "review", "p1", 13, { scheduledFrom: "w13", body: "Run npm audit and cargo audit; leave a task for anything new." }),
        work("w15", "Clean build caches", "todo", "p2", 14, { schedule: { rule: { kind: "interval", minutes: 360 }, enabled: false } }),
        work("w16", "Tag release candidate", "todo", "p3", 15, { schedule: { rule: { kind: "once", at: Date.now() + 26 * 3600_000 }, enabled: true, nextAt: Date.now() + 26 * 3600_000 } }),
      ],
    };
  },
  /** 업무 패널 + 팀원 다섯 명. 토의 열기 대화상자를 사회자 1 + 토론자 4로 꽉 채워 보는 장면. */
  discussions: (lang) => {
    const w = SCENARIOS.workpanels(lang);
    return { ...w, state: { ...w.state, agents: [...w.state.agents, profile("p4", "Dee", "m4", 3), profile("p5", "Eun", "k5", 4)] } };
  },
  /**
   * 회의실 연출: 팀원 다섯 명, Bo(p2)만 본업 세션이 돈다. Ada가 사회자이고 Bo·Cy·Dee가 토론자인 준비 상태 토의
   * 하나 — 스크립트가 관찰을 켜고 시작하면 쉬는 Ada·Cy·Dee는 회의실로, 일하는 Bo는 자리에서 말풍선만 띄운다.
   */
  meeting: (lang) => {
    const b = base(lang);
    const now = Date.now();
    const meeting: Discussion = {
      id: "disc-meeting", topic: "Where should the release notes live?", context: "", cwd: harnessCwd,
      moderatorAgentId: "p1",
      participants: [
        { agentId: "p1", agent: { cli: "claude" } },
        { agentId: "p2", agent: { cli: "claude" } },
        { agentId: "p3", agent: { cli: "codex" } },
        { agentId: "p4", agent: { cli: "claude" } },
      ],
      limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: false },
      state: "ready", round: 0, transcript: [], costUsd: 0, createdAt: now, updatedAt: now,
    };
    return {
      ...b,
      state: { ...b.state, agents: [...AGENTS, profile("p4", "Dee", "m4", 3), profile("p5", "Eun", "k5", 4)] },
      settings: { ...b.settings, settings: { ...b.settings.settings, observerEnabled: true } },
      adopted: [{ agentId: "p2", sessionId: "s2", cols: 100, rows: 30 }],
      output: { p2: "\x1b[1;32m$\x1b[0m cargo build\r\n   Compiling agent-office v0.1.0\r\n" },
      discussions: [meeting],
    };
  },
  /** 첫 실행: 캐릭터 없음, 온보딩 다이얼로그. */
  fresh: (lang) => ({ ...base(lang), state: { version: 1, agents: [] }, settings: settings(true, lang) }),
  /** 온보딩을 마친 사무실, 세션 없음. */
  office: base,
  /** 세션 둘이 돌고 하나는 입력을 기다리는 상태. */
  working: (lang) => ({
    ...base(lang),
    adopted: [
      { agentId: "p1", sessionId: "s1", cols: 100, rows: 30 },
      { agentId: "p2", sessionId: "s2", cols: 100, rows: 30 },
    ],
    output: {
      p1: "\x1b[1;32m$\x1b[0m npm test\r\n\r\n \x1b[32m✓\x1b[0m src/a.test.ts (12)\r\n \x1b[32m✓\x1b[0m src/b.test.ts (4)\r\n\r\n Tests  16 passed\r\n\x1b[1;32m$\x1b[0m ",
      p2: "\x1b[1;36m?\x1b[0m Apply this change? (y/n) ",
    },
    notifications: [
      {
        id: "n1",
        sessionId: "s2",
        agentId: "p2",
        source: "hook",
        message: "needs input",
        dedupKey: "hook:s2",
        at: Date.now() - 60_000,
      },
    ],
  }),
};

export const SCENARIO_IDS = Object.keys(SCENARIOS);

export function loadScenario(id: string, lang: string): Scenario {
  const make = SCENARIOS[id];
  if (!make) throw new Error(`gui-harness: unknown scenario "${id}" (${SCENARIO_IDS.join(", ")})`);
  return make(lang);
}
