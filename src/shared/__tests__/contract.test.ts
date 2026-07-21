// src/shared/__tests__/contract.test.ts
//
// Roundtrip snapshot tests for the shared TS contract:
// - fixed JSON strings (as they would arrive from the Rust backend) must be
//   assignable to the TS types without casts/`any`.
// - `notificationType()` derivation matches the source->type mapping.
// - `Commands`/`Events` name constants have no duplicate string values.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { Commands, Events } from "../ipc";
import type {
  ActivityEvent,
  AdoptedSessionInfo,
  AgentProfile,
  CreateSessionResult,
  GeneratedSpriteImage,
  GetAppSettingsResult,
  NotificationEvent,
  OutputChunk,
  PersistedState,
  SessionEventRecord,
  SessionExitInfo,
  SessionState,
  SessionStateEvent,
  SessionStatus,
  UsageSnapshot,
  UsageWindow,
} from "../types";
import { notificationType } from "../types";

// R-9: 케이스들 중 일부는 이제 src/shared/contract-fixtures/*.json으로 옮겨졌다
// (Rust 통합 테스트 src-tauri/tests/contract_fixtures.rs와 공유하는 왕복
// 검증 픽스처). 그 JSON을 여기서도 읽어 쓴다 — 손으로 다시 쓴 문자열을
// 남겨두면 인라인+픽스처+타입 3중 유지가 되어 드리프트 위험이 생긴다.
// 이 파일에 남은 인라인 JSON은 픽스처가 없는(고유한) 케이스뿐이다.
const FIXTURES_DIR = resolve(__dirname, "../contract-fixtures");
function loadFixtureRaw(name: string): string {
  return readFileSync(resolve(FIXTURES_DIR, name), "utf8");
}

describe("SessionState / SessionStatus", () => {
  it("accepts exactly the four backend states", () => {
    const states: SessionState[] = ["starting", "running", "exited", "disposed"];
    expect(states).toHaveLength(4);
  });

  it("SessionStatus additionally allows 'idle'", () => {
    const idle: SessionStatus = "idle";
    const running: SessionStatus = "running";
    expect(idle).toBe("idle");
    expect(running).toBe("running");
  });
});

describe("roundtrip: fixed JSON assignable to TS types", () => {
  it("SessionStateEvent (no exit)", () => {
    const json = '{"sessionId":"s1","agentId":"a1","state":"running","at":1720000000000}';
    const parsed: SessionStateEvent = JSON.parse(json);
    expect(parsed.state).toBe("running");
    expect(parsed.exit).toBeUndefined();
  });

  it("SessionStateEvent (with exit)", () => {
    const parsed: SessionStateEvent = JSON.parse(loadFixtureRaw("session-state-event.exit.json"));
    const exit: SessionExitInfo | undefined = parsed.exit;
    expect(exit?.exitCode).toBe(1);
    expect(exit?.signal).toBeUndefined();
    expect(exit?.intentional).toBe(false);
  });

  it("NotificationEvent", () => {
    const parsed: NotificationEvent = JSON.parse(loadFixtureRaw("notification-event.json"));
    expect(parsed.source).toBe("hook");
    expect(notificationType(parsed.source)).toBe("question");
  });

  it("OutputChunk", () => {
    const parsed: OutputChunk = JSON.parse(loadFixtureRaw("output-chunk.json"));
    expect(parsed.agentId).toBe("a1");
    expect(parsed.seq).toBe(42);
    expect(parsed.bytes).toBe(5);
  });

  it("CreateSessionResult", () => {
    const parsed: CreateSessionResult = JSON.parse(loadFixtureRaw("create-session-result.json"));
    expect(parsed.state).toBe("starting");
  });

  it("AgentProfile / PersistedState without cwd/startupCommand (backward compat with files saved before those fields existed)", () => {
    const parsed: PersistedState = JSON.parse(loadFixtureRaw("persisted-state.minimal.json"));
    const profile: AgentProfile = parsed.agents[0];
    expect(parsed.version).toBe(1);
    expect(profile.deskIndex).toBe(0);
    expect(profile.cwd).toBeUndefined();
    expect(profile.startupCommand).toBeUndefined();
  });

  it("AgentProfile / PersistedState with cwd and startupCommand", () => {
    const parsed: PersistedState = JSON.parse(loadFixtureRaw("persisted-state.full.json"));
    const profile: AgentProfile = parsed.agents[0];
    expect(profile.cwd).toBe("/tmp/proj");
    expect(profile.startupCommand).toBe("source ./init.sh");
  });

  it("ActivityEvent without text (tool / legacy)", () => {
    const json = '{"agentId":"a1","sessionId":"s1","kind":"tool","at":1720000000004}';
    const parsed: ActivityEvent = JSON.parse(json);
    expect(parsed.kind).toBe("tool");
    expect(parsed.text).toBeUndefined();
  });

  it("ActivityEvent with prompt text", () => {
    const parsed: ActivityEvent = JSON.parse(loadFixtureRaw("activity-event.prompt.json"));
    expect(parsed.text).toBe("버그 고쳐줘");
  });

  it("observer events keep their provider-neutral public shapes", () => {
    const activity: ActivityEvent = {
      agentId: "a1",
      sessionId: "s1",
      kind: "prompt",
      at: 1,
      text: "marker",
    };
    const notification: NotificationEvent = {
      id: "n1",
      sessionId: "s1",
      agentId: "a1",
      source: "hook",
      message: "확인이 필요합니다",
      dedupKey: "k1",
      at: 2,
    };

    expect("provider" in activity).toBe(false);
    expect("provider" in notification).toBe(false);
  });

  it("AdoptedSessionInfo", () => {
    const parsed: AdoptedSessionInfo = JSON.parse(loadFixtureRaw("adopted-session-info.json"));
    expect(parsed.agentId).toBe("a1");
    expect(parsed.rows).toBe(24);
    expect(parsed.cols).toBe(80);
  });

  it("SessionEventRecord (session_started, 옵션 필드 있음)", () => {
    // 수집 측이 실제로 쓰는 형태: envelope + 세션 시작 스냅샷 필드들.
    const parsed: SessionEventRecord = JSON.parse(loadFixtureRaw("session-event-record.started.json"));
    expect(parsed.kind).toBe("session_started");
    expect(parsed.agentName).toBe("Ada");
    expect(parsed.agentRole).toBe("backend");
    expect(parsed.cwd).toBe("/tmp/proj");
    expect(parsed.shell).toBe("/bin/zsh");
    // session_state 전용 필드는 이 종류엔 없다(skip_serializing_if).
    expect(parsed.state).toBeUndefined();
  });

  it("SessionEventRecord (tool, 옵션 필드 없음 = envelope만)", () => {
    const parsed: SessionEventRecord = JSON.parse(loadFixtureRaw("session-event-record.tool.json"));
    expect(parsed.kind).toBe("tool");
    expect(parsed.seq).toBe(42);
    expect(parsed.agentName).toBeUndefined();
    expect(parsed.agentRole).toBeUndefined();
    expect(parsed.cwd).toBeUndefined();
    expect(parsed.shell).toBeUndefined();
    expect(parsed.state).toBeUndefined();
  });

  it("SessionEventRecord (session_state, state 필드 있음)", () => {
    const json =
      '{"schemaVersion":1,"runId":"run-1","seq":7,"at":1783728200000,' +
      '"agentId":"a1","sessionId":"s1","kind":"session_state","state":"exited"}';
    const parsed: SessionEventRecord = JSON.parse(json);
    const state: SessionState | undefined = parsed.state;
    expect(state).toBe("exited");
  });

  it("UsageSnapshot (both providers, limits[] + fallback shapes)", () => {
    // Rust load_usage_snapshot이 실제로 내보내는 형태: 정규화된 원시 스냅샷.
    // resetsAtMs/label/windowMinutes/planLabel/isActive는 T | null(optional 아님).
    const parsed: UsageSnapshot = JSON.parse(loadFixtureRaw("usage-snapshot.json"));
    expect(parsed.claude?.provider).toBe("claude");
    expect(parsed.claude?.windows).toHaveLength(2);
    const w0: UsageWindow = parsed.claude!.windows[0];
    expect(w0.kind).toBe("session");
    expect(w0.label).toBeNull();
    expect(w0.resetsAtMs).toBe(1784281800243);
    expect(w0.windowMinutes).toBeNull();
    expect(w0.isActive).toBe(true);
    // weekly_model이 false로 와도(=is_active는 유효성이 아니라 "지금 구속
    // 중인 윈도" 표시일 뿐) 걸러지지 않고 그대로 남아 있어야 한다.
    expect(parsed.claude?.windows[1].label).toBe("Fable");
    expect(parsed.claude?.windows[1].isActive).toBe(false);
    expect(parsed.codex?.windows[0].windowMinutes).toBe(10080);
    expect(parsed.codex?.windows[0].isActive).toBeNull();
    expect(parsed.codex?.planLabel).toBe("prolite");
  });

  it("UsageSnapshot (a failed source is null, not omitted)", () => {
    // 백엔드가 실패한 provider를 null로 직렬화한다(커맨드 자체는 성공).
    const json = '{"claude":null,"codex":null}';
    const parsed: UsageSnapshot = JSON.parse(json);
    expect(parsed.claude).toBeNull();
    expect(parsed.codex).toBeNull();
  });

  it("GeneratedSpriteImage", () => {
    const json = '{"pngBase64":"AAAA","costUsd":0.02}';
    const parsed: GeneratedSpriteImage = JSON.parse(json);
    expect(parsed.pngBase64).toBe("AAAA");
    expect(parsed.costUsd).toBe(0.02);
    // cost_usd는 skip_serializing_if — 없는 형태도 유효해야 한다.
    const noCost: GeneratedSpriteImage = JSON.parse('{"pngBase64":"BBBB"}');
    expect(noCost.costUsd).toBeUndefined();
  });
});

describe("notificationType derivation", () => {
  it("hook -> question, stop -> done, bell -> info", () => {
    expect(notificationType("hook")).toBe("question");
    expect(notificationType("stop")).toBe("done");
    expect(notificationType("bell")).toBe("info");
  });
});

describe("Commands / Events name constants", () => {
  it("match the exact snake_case/kebab-case wire strings the Rust backend emits", () => {
    expect(Commands.createSession).toBe("create_session");
    expect(Commands.disposeSession).toBe("dispose_session");
    expect(Commands.writeInput).toBe("write_input");
    expect(Commands.resize).toBe("resize_session");
    expect(Commands.clearNotifications).toBe("clear_notifications");
    expect(Commands.listNotifications).toBe("list_notifications");
    expect(Commands.loadState).toBe("load_state");
    expect(Commands.saveState).toBe("save_state");
    expect(Commands.setBadgeCount).toBe("set_badge_count");
    expect(Commands.subscribeOutput).toBe("subscribe_output");
    expect(Commands.unsubscribeOutput).toBe("unsubscribe_output");
    expect(Commands.summarizeText).toBe("summarize_text");
    expect(Commands.handoffSupported).toBe("handoff_supported");
    expect(Commands.handoffSessions).toBe("handoff_sessions");
    expect(Commands.adoptDetachedSessions).toBe("adopt_detached_sessions");
    expect(Commands.sessionBrokerMode).toBe("session_broker_mode");
    expect(Commands.uploadSessionSnapshots).toBe("upload_session_snapshots");
    expect(Commands.loadSessionEvents).toBe("load_session_events");
    expect(Commands.loadUsageSnapshot).toBe("load_usage_snapshot");

    expect(Events.sessionState).toBe("session-state");
    expect(Events.notificationNew).toBe("notification-new");
    expect(Events.notificationCleared).toBe("notification-cleared");
  });

  it("has no duplicate values across Commands and Events combined", () => {
    const allValues = [...Object.values(Commands), ...Object.values(Events)];
    const unique = new Set(allValues);
    expect(unique.size).toBe(allValues.length);
  });
});

describe("AppSettings (opt-in 설정 계약)", () => {
  it("Rust GetAppSettingsResult JSON이 TS 타입에 그대로 할당된다", () => {
    const parsed: GetAppSettingsResult = JSON.parse(loadFixtureRaw("get-app-settings-result.json"));
    expect(parsed.firstRun).toBe(true);
    expect(parsed.settings).toEqual({
      version: 1,
      summarizerEnabled: false,
      summaryProvider: "claude",
      diaryEnabled: false,
      observerEnabled: false,
      soundEnabled: true,
      soundVolume: 0.5,
      externalTerminal: "terminal",
      externalEditor: "system",
      attentionHoldMs: 5000,
      gitStatusEnabled: true,
      fileIndexBackend: "walker",
      cliEnabled: false,
      keepAwakeEnabled: false,
    });
  });

  it("커맨드 이름 상수가 등록되어 있다", () => {
    expect(Commands.getAppSettings).toBe("get_app_settings");
    expect(Commands.setAppSettings).toBe("set_app_settings");
  });
});
