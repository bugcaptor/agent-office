// src/shared/__tests__/contract-fixtures.test.ts
//
// R-9 옵션 A: Rust<->TS 타입 계약 테스트, TS 쪽 절반.
//
// `src/shared/contract-fixtures/*.json`은 Rust 통합 테스트
// (src-tauri/tests/contract_fixtures.rs)와 공유하는 픽스처다. 그쪽은 serde
// 왕복으로 "Rust가 실제로 이 모양을 내보내고 받는다"를 검증하고, 여기서는
// "그 모양이 TS 타입에 그대로 할당된다"를 검증한다.
//
// `import x from "*.json"`은 쓰지 않는다 -- TS의 JSON 모듈 리터럴 추론이
// string 프로퍼티를 넓혀버려("hook" -> string) 유니온 타입 대입이 컴파일
// 에러가 난다. 대신 `fs.readFileSync` + `JSON.parse`로 읽는다(반환 타입은
// `any`이므로 명시적 타입 어노테이션을 붙여 대입 가능성만 확인한다).
//
// `any`에서 타입 어노테이션 대입은 초과/누락 프로퍼티를 정적으로 잡지
// 못한다(엄격한 객체 리터럴 체크는 리터럴에만 적용된다) -- 그래서 casing
// 드리프트 탐지는 `Object.keys()` 집합 런타임 단언에 의존한다.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

import type {
  ActivityEvent,
  AdoptedSessionInfo,
  AgentProfile,
  AppSettings,
  BotAgentStatus,
  BotStatus,
  CreateSessionRequest,
  CreateSessionResult,
  GetAppSettingsResult,
  GitCommitEntry,
  GitFileHistoryResult,
  GitFileStatus,
  GitStatusResult,
  NotificationEvent,
  OutputChunk,
  PersistedState,
  ProviderUsage,
  SessionEventRecord,
  SessionExitInfo,
  SessionStateEvent,
  UsageSnapshot,
  UsageWindow,
} from "../types";

const FIXTURES_DIR = resolve(__dirname, "../contract-fixtures");

function loadFixture(name: string): unknown {
  const raw = readFileSync(resolve(FIXTURES_DIR, name), "utf8");
  return JSON.parse(raw);
}

function loadFixtureRaw(name: string): string {
  return readFileSync(resolve(FIXTURES_DIR, name), "utf8");
}

/** 최상위 키 집합이 정확히 일치하는지 -- casing 드리프트("resetsAtMs" vs
 * "resets_at_ms" 등)는 타입 대입만으로는 안 잡히므로 런타임에서 잡는다. */
function expectKeys(obj: object, expected: string[]): void {
  expect(Object.keys(obj).sort()).toEqual([...expected].sort());
}

describe("contract fixtures: Rust serde output assignable to TS types", () => {
  it("SessionStateEvent (exit)", () => {
    const parsed: SessionStateEvent = loadFixture("session-state-event.exit.json") as SessionStateEvent;
    expectKeys(parsed, ["sessionId", "agentId", "state", "exit", "at"]);
    expect(parsed.state).toBe("exited");
    const exit: SessionExitInfo | undefined = parsed.exit;
    expectKeys(exit!, ["sessionId", "exitCode", "intentional"]);
    expect(exit?.exitCode).toBe(1);
    expect(exit?.signal).toBeUndefined();
    expect(exit?.intentional).toBe(false);
  });

  it("NotificationEvent", () => {
    const parsed: NotificationEvent = loadFixture("notification-event.json") as NotificationEvent;
    expectKeys(parsed, ["id", "sessionId", "agentId", "source", "message", "dedupKey", "at"]);
    expect(parsed.source).toBe("hook");
  });

  it("OutputChunk", () => {
    const parsed: OutputChunk = loadFixture("output-chunk.json") as OutputChunk;
    expectKeys(parsed, ["sessionId", "agentId", "data", "frames", "seq", "bytes"]);
    expect(parsed.seq).toBe(42);
    expect(parsed.bytes).toBe(5);
  });

  it("CreateSessionRequest", () => {
    const parsed: CreateSessionRequest = loadFixture("create-session-request.json") as CreateSessionRequest;
    expectKeys(parsed, [
      "agentId",
      "cols",
      "rows",
      "cwd",
      "shell",
      "startupCommand",
      "personalityPrompt",
      "autostartClaude",
    ]);
    expect(parsed.agentId).toBe("a1");
    expect(parsed.shell).toBe("pwsh");
    expect(parsed.personalityPrompt).toBe("친절하게 대답해");
  });

  it("CreateSessionResult", () => {
    const parsed: CreateSessionResult = loadFixture("create-session-result.json") as CreateSessionResult;
    expectKeys(parsed, ["sessionId", "state"]);
    expect(parsed.state).toBe("starting");
  });

  it("AdoptedSessionInfo", () => {
    const parsed: AdoptedSessionInfo = loadFixture("adopted-session-info.json") as AdoptedSessionInfo;
    expectKeys(parsed, ["agentId", "sessionId", "rows", "cols"]);
    expect(parsed.rows).toBe(24);
    expect(parsed.cols).toBe(80);
  });

  it("PersistedState / AgentProfile (full: cwd + startupCommand 있음)", () => {
    const parsed: PersistedState = loadFixture("persisted-state.full.json") as PersistedState;
    expectKeys(parsed, ["agents", "version", "vacationMode"]);
    expect(parsed.vacationMode).toBe(true);
    const profile: AgentProfile = parsed.agents[0];
    expectKeys(profile, [
      "id",
      "name",
      "role",
      "note",
      "seed",
      "createdAt",
      "deskIndex",
      "cwd",
      "startupCommand",
    ]);
    expect(profile.cwd).toBe("/tmp/proj");
    expect(profile.startupCommand).toBe("source ./init.sh");
  });

  it("PersistedState / AgentProfile (minimal: cwd + startupCommand 없음, backward compat)", () => {
    const parsed: PersistedState = loadFixture("persisted-state.minimal.json") as PersistedState;
    expectKeys(parsed, ["agents", "version"]);
    expect(parsed.vacationMode).toBeUndefined();
    const profile: AgentProfile = parsed.agents[0];
    expectKeys(profile, ["id", "name", "role", "note", "seed", "createdAt", "deskIndex"]);
    expect(profile.cwd).toBeUndefined();
    expect(profile.startupCommand).toBeUndefined();
  });

  it("ActivityEvent (prompt, 한글 text)", () => {
    const parsed: ActivityEvent = loadFixture("activity-event.prompt.json") as ActivityEvent;
    expectKeys(parsed, ["agentId", "sessionId", "kind", "at", "text"]);
    expect(parsed.kind).toBe("prompt");
    expect(parsed.text).toBe("버그 고쳐줘");
    expect(parsed.assistantText).toBeUndefined();
    expect(parsed.cwd).toBeUndefined();
    expect(parsed.count).toBeUndefined();
  });

  it("SessionEventRecord (session_started, 옵션 필드 있음)", () => {
    const parsed: SessionEventRecord = loadFixture("session-event-record.started.json") as SessionEventRecord;
    expectKeys(parsed, [
      "schemaVersion",
      "runId",
      "seq",
      "at",
      "agentId",
      "sessionId",
      "kind",
      "agentName",
      "agentRole",
      "cwd",
      "shell",
    ]);
    expect(parsed.kind).toBe("session_started");
    expect(parsed.agentName).toBe("Ada");
    expect(parsed.state).toBeUndefined();
  });

  it("SessionEventRecord (tool, envelope만)", () => {
    const parsed: SessionEventRecord = loadFixture("session-event-record.tool.json") as SessionEventRecord;
    expectKeys(parsed, ["schemaVersion", "runId", "seq", "at", "agentId", "sessionId", "kind"]);
    expect(parsed.kind).toBe("tool");
    expect(parsed.seq).toBe(42);
  });

  it("UsageSnapshot (both providers, limits[] + null 폴백)", () => {
    const parsed: UsageSnapshot = loadFixture("usage-snapshot.json") as UsageSnapshot;
    expectKeys(parsed, ["claude", "codex"]);
    const claude: ProviderUsage = parsed.claude!;
    expectKeys(claude, ["provider", "fetchedAtMs", "planLabel", "windows"]);
    expect(claude.windows).toHaveLength(2);
    const w0: UsageWindow = claude.windows[0];
    expectKeys(w0, ["kind", "label", "usedPercent", "resetsAtMs", "windowMinutes", "isActive"]);
    expect(w0.kind).toBe("session");
    expect(w0.label).toBeNull();
    expect(w0.isActive).toBe(true);
    expect(claude.windows[1].label).toBe("Fable");
    expect(claude.windows[1].isActive).toBe(false);
    const codex: ProviderUsage = parsed.codex!;
    expect(codex.windows[0].windowMinutes).toBe(10080);
    expect(codex.windows[0].isActive).toBeNull();
    expect(codex.planLabel).toBe("prolite");
  });

  it("GetAppSettingsResult / AppSettings", () => {
    const parsed: GetAppSettingsResult = loadFixture("get-app-settings-result.json") as GetAppSettingsResult;
    expectKeys(parsed, ["settings", "firstRun"]);
    expect(parsed.firstRun).toBe(true);
    const settings: AppSettings = parsed.settings;
    expectKeys(settings, [
      "version",
      "summarizerEnabled",
      "summaryProvider",
      "diaryEnabled",
      "observerEnabled",
      "soundEnabled",
      "soundVolume",
      "externalTerminal",
      "externalEditor",
      "attentionHoldMs",
      "gitStatusEnabled",
      "fileIndexBackend",
      "cliEnabled",
      "keepAwakeEnabled",
      "mascotEnabled",
    ]);
    expect(settings.gitStatusEnabled).toBe(true);
    expect(settings.fileIndexBackend).toBe("walker");
    expect(settings.cliEnabled).toBe(false);
    expect(settings.keepAwakeEnabled).toBe(false);
    expect(settings.mascotEnabled).toBe(false);
  });

  it("GitStatusResult / GitFileStatus", () => {
    const parsed: GitStatusResult = loadFixture("git-status-result.json") as GitStatusResult;
    expectKeys(parsed, ["isRepo", "branch", "ahead", "behind", "entries", "timedOut", "truncated"]);
    expect(parsed.branch).toBe("main");
    const entry: GitFileStatus = parsed.entries[0];
    expectKeys(entry, ["path", "status", "xy"]);
    expect(entry.status).toBe("M");
  });

  it("GitFileHistoryResult / GitCommitEntry", () => {
    const parsed: GitFileHistoryResult = loadFixture("git-file-history-result.json") as GitFileHistoryResult;
    expectKeys(parsed, ["commits", "hasMore", "timedOut"]);
    const commit: GitCommitEntry = parsed.commits[0];
    expectKeys(commit, ["hash", "shortHash", "author", "date", "subject"]);
    expect(commit.shortHash).toBe("abcdef0");
  });

  it("BotStatus / BotAgentStatus", () => {
    const parsed: BotStatus = loadFixture("bot-status.json") as BotStatus;
    expectKeys(parsed, ["agents"]);
    const a1: BotAgentStatus = parsed.agents["a1"];
    expectKeys(a1, ["running", "phase", "issue", "slug", "pollIntervalSec", "lastPollAtMs"]);
    expect(a1.phase).toBe("working");
    expect(a1.issue).toBe(42);
    expect(a1.error).toBeUndefined();
  });

  it("fixtures parse as valid JSON (sanity: raw text is well-formed)", () => {
    for (const name of [
      "session-state-event.exit.json",
      "notification-event.json",
      "output-chunk.json",
      "create-session-request.json",
      "create-session-result.json",
      "adopted-session-info.json",
      "persisted-state.full.json",
      "persisted-state.minimal.json",
      "activity-event.prompt.json",
      "session-event-record.started.json",
      "session-event-record.tool.json",
      "usage-snapshot.json",
      "get-app-settings-result.json",
      "git-status-result.json",
      "git-file-history-result.json",
      "bot-status.json",
    ]) {
      expect(() => JSON.parse(loadFixtureRaw(name))).not.toThrow();
    }
  });
});
