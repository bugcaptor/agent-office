// src/renderer/guiHarness/scenarios.ts
//
// 하네스 시나리오 — 부팅 시 가짜 백엔드가 돌려줄 초기 데이터. 응답 모양은
// Rust serde 출력과 대조되는 `@shared/contract-fixtures`를 바탕으로 만든다
// (손으로 쓴 모양이 백엔드와 어긋나 화면이 거짓으로 깨지거나 멀쩡해 보이지 않게).
// URL `?scenario=<id>&lang=<code>`로 고른다.
import type {
  AdoptedSessionInfo,
  AgentProfile,
  AwardsFile,
  GetAppSettingsResult,
  NotificationEvent,
  PersistedState,
  UsageSnapshot,
  WorkItem,
} from "@shared/types";
import settingsFixture from "@shared/contract-fixtures/get-app-settings-result.json";
import awardsFixture from "@shared/contract-fixtures/awards-file.json";
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
}

const profile = (id: string, name: string, seed: string, deskIndex: number): AgentProfile => ({
  id,
  name,
  role: "engineer",
  seed,
  createdAt: 1720000000000 + deskIndex,
  deskIndex,
  cwd: "/tmp/gui-harness",
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
  };
}

const work = (id: string, title: string, status: WorkItem["status"], assigneeAgentId: string | undefined, order: number, extra: Partial<WorkItem> = {}): WorkItem => ({
  id, title, body: `${title} body`, status, assigneeAgentId, order, sessionIds: [], createdAt: 1720000000000 + order, updatedAt: 1720000000000 + order, ...extra,
});

const SCENARIOS: Record<string, (lang: string) => Scenario> = {
  /** 업무 패널을 켜고 캐릭터마다 다른 프로젝트 폴더와 업무를 둔 사무실. */
  workpanels: (lang) => {
    const b = base(lang);
    const cwds = ["/Users/me/dev/agent-office", "/Users/me/dev/kbm-server", undefined];
    return {
      ...b,
      state: { ...b.state, agents: AGENTS.map((a, i) => ({ ...a, cwd: cwds[i] })) },
      settings: { ...b.settings, settings: { ...b.settings.settings, workPanelsEnabled: true } },
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
      ],
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
