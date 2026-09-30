// IPC 이름과 알림 출처 분류를 검증한다. 직렬화 형식은 Rust 픽스처 테스트가 담당한다.
import { describe, expect, it } from "vitest";
import { Commands, Events } from "../ipc";
import { notificationType } from "../types";

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
    expect(Commands.getAppSettings).toBe("get_app_settings");
    expect(Commands.setAppSettings).toBe("set_app_settings");
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
    // 세션 로그(docs/session-log-design.md §6)
    expect(Commands.listSessionLogs).toBe("list_session_logs");
    expect(Commands.openSessionLog).toBe("open_session_log");
    expect(Commands.generateStudyMaterial).toBe("generate_study_material");

    expect(Events.sessionState).toBe("session-state");
    expect(Events.notificationNew).toBe("notification-new");
    expect(Events.notificationCleared).toBe("notification-cleared");
    // 알림과 분리된 사용량 채널(억제된 Stop에서도 온다).
    expect(Events.turnUsage).toBe("turn-usage");

    // 마스코트 창(이슈 #72) — Rust `emit_to`/커맨드 이름과 짝이 맞아야 한다.
    expect(Commands.setMascotVisible).toBe("set_mascot_visible");
    expect(Commands.mascotActivate).toBe("mascot_activate");
    expect(Events.mascotState).toBe("mascot-state");
    expect(Events.mascotReady).toBe("mascot-ready");
    expect(Events.mascotOpenTerminal).toBe("mascot-open-terminal");
  });

  it("has no duplicate values across Commands and Events combined", () => {
    const allValues = [...Object.values(Commands), ...Object.values(Events)];
    const unique = new Set(allValues);
    expect(unique.size).toBe(allValues.length);
  });
});
