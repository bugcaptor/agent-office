// src/shared/__tests__/contract.test.ts
//
// Roundtrip snapshot tests for the shared TS contract:
// - fixed JSON strings (as they would arrive from the Rust backend) must be
//   assignable to the TS types without casts/`any`.
// - `notificationType()` derivation matches the source->type mapping.
// - `Commands`/`Events` name constants have no duplicate string values.

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
    const json =
      '{"sessionId":"s1","agentId":"a1","state":"exited",' +
      '"exit":{"sessionId":"s1","exitCode":1,"intentional":false},"at":1720000000001}';
    const parsed: SessionStateEvent = JSON.parse(json);
    const exit: SessionExitInfo | undefined = parsed.exit;
    expect(exit?.exitCode).toBe(1);
    expect(exit?.signal).toBeUndefined();
    expect(exit?.intentional).toBe(false);
  });

  it("NotificationEvent", () => {
    const json =
      '{"id":"n1","sessionId":"s1","agentId":"a1","source":"hook",' +
      '"message":"needs input","dedupKey":"hook:s1","at":1720000000002}';
    const parsed: NotificationEvent = JSON.parse(json);
    expect(parsed.source).toBe("hook");
    expect(notificationType(parsed.source)).toBe("question");
  });

  it("OutputChunk", () => {
    const json = '{"sessionId":"s1","agentId":"a1","data":"hello","frames":3,"seq":42}';
    const parsed: OutputChunk = JSON.parse(json);
    expect(parsed.agentId).toBe("a1");
    expect(parsed.seq).toBe(42);
  });

  it("CreateSessionResult", () => {
    const json = '{"sessionId":"s1","state":"starting"}';
    const parsed: CreateSessionResult = JSON.parse(json);
    expect(parsed.state).toBe("starting");
  });

  it("AgentProfile / PersistedState", () => {
    const json =
      '{"agents":[{"id":"p1","name":"Ada","role":"backend","note":"",' +
      '"seed":"abc123","createdAt":1720000000003,"deskIndex":0}],"version":1}';
    const parsed: PersistedState = JSON.parse(json);
    const profile: AgentProfile = parsed.agents[0];
    expect(parsed.version).toBe(1);
    expect(profile.deskIndex).toBe(0);
  });

  it("AgentProfile / PersistedState without cwd (backward compat with files saved before the cwd field existed)", () => {
    const json =
      '{"agents":[{"id":"p1","name":"Ada","role":"backend","note":"",' +
      '"seed":"abc123","createdAt":1720000000003,"deskIndex":0}],"version":1}';
    const parsed: PersistedState = JSON.parse(json);
    const profile: AgentProfile = parsed.agents[0];
    expect(profile.cwd).toBeUndefined();
  });

  it("AgentProfile / PersistedState with cwd", () => {
    const json =
      '{"agents":[{"id":"p1","name":"Ada","role":"backend","note":"",' +
      '"seed":"abc123","createdAt":1720000000003,"deskIndex":0,"cwd":"/tmp/proj"}],"version":1}';
    const parsed: PersistedState = JSON.parse(json);
    const profile: AgentProfile = parsed.agents[0];
    expect(profile.cwd).toBe("/tmp/proj");
  });

  it("AgentProfile / PersistedState with startupCommand", () => {
    const json =
      '{"agents":[{"id":"p1","name":"Ada","role":"backend","note":"",' +
      '"seed":"abc123","createdAt":1720000000003,"deskIndex":0,' +
      '"startupCommand":"source ./init.sh"}],"version":1}';
    const parsed: PersistedState = JSON.parse(json);
    const profile: AgentProfile = parsed.agents[0];
    expect(profile.startupCommand).toBe("source ./init.sh");
  });

  it("AgentProfile without startupCommand (backward compat)", () => {
    const json =
      '{"agents":[{"id":"p1","name":"Ada","role":"backend","note":"",' +
      '"seed":"abc123","createdAt":1720000000003,"deskIndex":0}],"version":1}';
    const parsed: PersistedState = JSON.parse(json);
    const profile: AgentProfile = parsed.agents[0];
    expect(profile.startupCommand).toBeUndefined();
  });

  it("ActivityEvent without text (tool / legacy)", () => {
    const json = '{"agentId":"a1","sessionId":"s1","kind":"tool","at":1720000000004}';
    const parsed: ActivityEvent = JSON.parse(json);
    expect(parsed.kind).toBe("tool");
    expect(parsed.text).toBeUndefined();
  });

  it("ActivityEvent with prompt text", () => {
    const json =
      '{"agentId":"a1","sessionId":"s1","kind":"prompt","at":1720000000005,"text":"버그 고쳐줘"}';
    const parsed: ActivityEvent = JSON.parse(json);
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
    const json = '{"agentId":"a1","sessionId":"s1","rows":24,"cols":80}';
    const parsed: AdoptedSessionInfo = JSON.parse(json);
    expect(parsed.agentId).toBe("a1");
    expect(parsed.rows).toBe(24);
    expect(parsed.cols).toBe(80);
  });

  it("SessionEventRecord (session_started, 옵션 필드 있음)", () => {
    // 수집 측이 실제로 쓰는 형태: envelope + 세션 시작 스냅샷 필드들.
    const json =
      '{"schemaVersion":1,"runId":"run-1","seq":1,"at":1783728000000,' +
      '"agentId":"a1","sessionId":"s1","kind":"session_started",' +
      '"agentName":"Ada","agentRole":"backend","cwd":"/tmp/proj","shell":"/bin/zsh"}';
    const parsed: SessionEventRecord = JSON.parse(json);
    expect(parsed.kind).toBe("session_started");
    expect(parsed.agentName).toBe("Ada");
    expect(parsed.agentRole).toBe("backend");
    expect(parsed.cwd).toBe("/tmp/proj");
    expect(parsed.shell).toBe("/bin/zsh");
    // session_state 전용 필드는 이 종류엔 없다(skip_serializing_if).
    expect(parsed.state).toBeUndefined();
  });

  it("SessionEventRecord (tool, 옵션 필드 없음 = envelope만)", () => {
    const json =
      '{"schemaVersion":1,"runId":"run-1","seq":42,"at":1783728100000,' +
      '"agentId":"a1","sessionId":"s1","kind":"tool"}';
    const parsed: SessionEventRecord = JSON.parse(json);
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
    const json =
      '{"claude":{"provider":"claude","fetchedAtMs":1784281391475,"planLabel":"max_20x",' +
      '"windows":[' +
      '{"kind":"session","label":null,"usedPercent":61,"resetsAtMs":1784281800243,"windowMinutes":null,"isActive":true},' +
      '{"kind":"weekly_model","label":"Fable","usedPercent":24,"resetsAtMs":1784606400000,"windowMinutes":null,"isActive":false}' +
      ']},' +
      '"codex":{"provider":"codex","fetchedAtMs":1784287217595,"planLabel":"prolite",' +
      '"windows":[{"kind":"weekly","label":null,"usedPercent":11,"resetsAtMs":1784786662000,"windowMinutes":10080,"isActive":null}]}}';
    const parsed: UsageSnapshot = JSON.parse(json);
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
    const json =
      '{"settings":{"version":1,"summarizerEnabled":false,"summaryProvider":"claude","observerEnabled":false,"soundEnabled":true,"soundVolume":0.5,"externalTerminal":"terminal"},"firstRun":true}';
    const parsed: GetAppSettingsResult = JSON.parse(json);
    expect(parsed.firstRun).toBe(true);
    expect(parsed.settings).toEqual({
      version: 1,
      summarizerEnabled: false,
      summaryProvider: "claude",
      observerEnabled: false,
      soundEnabled: true,
      soundVolume: 0.5,
      externalTerminal: "terminal",
    });
  });

  it("커맨드 이름 상수가 등록되어 있다", () => {
    expect(Commands.getAppSettings).toBe("get_app_settings");
    expect(Commands.setAppSettings).toBe("set_app_settings");
  });
});
