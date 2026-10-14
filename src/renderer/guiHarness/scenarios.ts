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
  };
}

const SCENARIOS: Record<string, (lang: string) => Scenario> = {
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
