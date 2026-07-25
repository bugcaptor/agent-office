// 세션 로그 보기(docs/session-log-design.md §7)의 표시 포맷과 스토어 동작.
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { formatBytes, formatDuration, formatWhen, shortenPath } from "../format";
import { useSessionLogStore, PAGE_SIZE } from "../sessionLogStore";
import { tauriApi } from "../../ipc/tauriApi";
import { useMarkdownStore } from "../../markdown/markdownStore";
import type { SessionLogItem } from "@shared/types";

function item(overrides: Partial<SessionLogItem> = {}): SessionLogItem {
  return {
    path: "/root/term-1/20260725-140311-abcd1234.log",
    sessionId: "abcd1234",
    startedAt: 1_780_000_000_000,
    modifiedAt: 1_780_000_600_000,
    bytes: 2048,
    cwd: "/Users/me/dev/foo",
    ...overrides,
  };
}

describe("format", () => {
  it("시각을 분 단위까지 보여준다", () => {
    const at = new Date(2026, 6, 25, 14, 3).getTime();
    expect(formatWhen(at)).toBe("2026-07-25 14:03");
  });

  it("시각이 없으면 미상으로 표시한다", () => {
    expect(formatWhen(0)).toBe("시각 미상");
  });

  it("지속 시간을 사람 단위로 접는다", () => {
    expect(formatDuration(0, 30_000)).toBe("1분 미만");
    expect(formatDuration(0, 12 * 60_000)).toBe("12분");
    expect(formatDuration(0, 60 * 60_000)).toBe("1시간");
    expect(formatDuration(0, 95 * 60_000)).toBe("1시간 35분");
  });

  it("파일 크기를 단위로 접는다", () => {
    expect(formatBytes(512)).toBe("512B");
    expect(formatBytes(2048)).toBe("2KB");
    expect(formatBytes(3 * 1024 * 1024)).toBe("3.0MB");
  });

  it("홈 경로를 ~로 줄이고 긴 경로는 앞을 자른다", () => {
    expect(shortenPath("/Users/me/dev/foo")).toBe("~/dev/foo");
    const long = shortenPath("/Users/me/dev/very/deep/nested/project/inner", 20);
    expect(long.startsWith("…")).toBe(true);
    expect(long.length).toBe(20);
    // 뒤쪽(실제 프로젝트 이름)은 살아남아야 한다.
    expect(long.endsWith("inner")).toBe(true);
  });
});

describe("sessionLogStore", () => {
  beforeEach(() => {
    useSessionLogStore.setState({
      overlay: null,
      items: [],
      total: 0,
      page: 0,
      selected: null,
      loading: false,
      generating: false,
      notice: null,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("열면 첫 페이지를 10개 단위로 요청한다", async () => {
    const list = vi
      .spyOn(tauriApi, "listSessionLogs")
      .mockResolvedValue({ total: 23, items: [item()] });

    useSessionLogStore.getState().open("term-1", "루나");
    await vi.waitFor(() => expect(useSessionLogStore.getState().loading).toBe(false));

    expect(list).toHaveBeenCalledWith("term-1", 0, PAGE_SIZE);
    expect(useSessionLogStore.getState().total).toBe(23);
    expect(useSessionLogStore.getState().items).toHaveLength(1);
  });

  it("페이지를 넘기면 offset이 따라가고 선택은 풀린다", async () => {
    const list = vi
      .spyOn(tauriApi, "listSessionLogs")
      .mockResolvedValue({ total: 23, items: [item()] });
    useSessionLogStore.setState({ overlay: { agentId: "term-1", agentName: "루나" }, selected: "x" });

    await useSessionLogStore.getState().setPage(2);

    expect(list).toHaveBeenCalledWith("term-1", 2 * PAGE_SIZE, PAGE_SIZE);
    expect(useSessionLogStore.getState().selected).toBeNull();
  });

  it("다른 캐릭터로 바뀐 뒤 도착한 응답은 반영하지 않는다", async () => {
    let resolve: ((v: { total: number; items: SessionLogItem[] }) => void) | undefined;
    vi.spyOn(tauriApi, "listSessionLogs").mockReturnValue(
      new Promise((r) => {
        resolve = r;
      }) as ReturnType<typeof tauriApi.listSessionLogs>
    );

    useSessionLogStore.getState().open("term-1", "루나");
    // 응답이 오기 전에 다른 캐릭터로 전환.
    useSessionLogStore.setState({ overlay: { agentId: "term-2", agentName: "노아" } });
    resolve?.({ total: 5, items: [item()] });
    await Promise.resolve();

    expect(useSessionLogStore.getState().items).toHaveLength(0);
  });

  it("학습자료를 만들면 인앱 마크다운 뷰어로 넘긴다", async () => {
    vi.spyOn(tauriApi, "listSessionLogs").mockResolvedValue({ total: 1, items: [item()] });
    vi.spyOn(tauriApi, "generateStudyMaterial").mockResolvedValue({
      path: "/root/study/term-1-20260725-151200.md",
      dir: "/root/study",
      fileName: "term-1-20260725-151200.md",
    });
    const openFile = vi.spyOn(useMarkdownStore.getState(), "openFile").mockResolvedValue();

    useSessionLogStore.setState({
      overlay: { agentId: "term-1", agentName: "루나" },
      selected: item().path,
    });
    await useSessionLogStore.getState().makeStudyMaterial();

    expect(tauriApi.generateStudyMaterial).toHaveBeenCalledWith("term-1", item().path);
    expect(openFile).toHaveBeenCalledWith(
      "/root/study",
      "term-1-20260725-151200.md",
      "term-1",
      expect.any(Function)
    );
    // 뷰어로 넘어갔으므로 오버레이는 닫힌다.
    expect(useSessionLogStore.getState().overlay).toBeNull();
  });

  it("요약기가 꺼져 있으면 안내로 바꿔 보여준다", async () => {
    vi.spyOn(tauriApi, "generateStudyMaterial").mockRejectedValue(
      new Error("summarizer-disabled")
    );
    useSessionLogStore.setState({
      overlay: { agentId: "term-1", agentName: "루나" },
      selected: item().path,
    });

    await useSessionLogStore.getState().makeStudyMaterial();

    expect(useSessionLogStore.getState().generating).toBe(false);
    expect(useSessionLogStore.getState().notice).toContain("요약기");
    // 실패했으면 오버레이는 그대로 열려 있어야 한다.
    expect(useSessionLogStore.getState().overlay).not.toBeNull();
  });

  it("선택이 없으면 아무 동작도 하지 않는다", async () => {
    const gen = vi.spyOn(tauriApi, "generateStudyMaterial");
    const open = vi.spyOn(tauriApi, "openSessionLog");
    useSessionLogStore.setState({
      overlay: { agentId: "term-1", agentName: "루나" },
      selected: null,
    });

    await useSessionLogStore.getState().makeStudyMaterial();
    await useSessionLogStore.getState().openInEditor();

    expect(gen).not.toHaveBeenCalled();
    expect(open).not.toHaveBeenCalled();
  });
});
