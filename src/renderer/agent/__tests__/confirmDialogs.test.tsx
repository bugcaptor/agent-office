// @vitest-environment jsdom
//
// src/renderer/agent/__tests__/confirmDialogs.test.tsx
//
// 확인(confirm-*) 모달 6종 TDD. 예전엔 파일 6개가 거의 같은 테스트를 복붙하고
// 있었다 — 공통 껍데기(self-gate / 이름 표시 / 확인·취소 / backdrop 클릭 /
// 경고 문단 마크업)는 표 기반으로 한 번에 돌리고, 종류별로 진짜 다른 동작
// (resume의 sessionId, terminate의 탕비실 안내, 전체 퇴근의 인원 수와 액션
// 분기, 봇 모드 시작의 상시 경고)만 따로 검증한다.
//
// 오케스트레이터(deleteAgent/restartAgentSession/...)는 모듈 목으로 대체해
// 다이얼로그의 배선만 본다(실제 PTY/스토어/xterm 정리는 각 오케스트레이터
// 테스트가 담당).
import type { ReactElement } from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile, ModalState } from "../../store/types";

const deleteAgent = vi.fn().mockResolvedValue(undefined);
const restartAgentSession = vi.fn().mockResolvedValue(undefined);
const resumeAgentSession = vi.fn().mockResolvedValue(undefined);
const terminateAgentSession = vi.fn().mockResolvedValue(undefined);
const clockOutAgent = vi.fn().mockResolvedValue(undefined);
const clockOutAll = vi.fn().mockResolvedValue(undefined);
const startBot = vi.fn().mockResolvedValue(undefined);

vi.mock("../deleteAgent", () => ({
  deleteAgent: (...args: unknown[]) => deleteAgent(...args),
}));
vi.mock("../restartAgentSession", () => ({
  restartAgentSession: (...args: unknown[]) => restartAgentSession(...args),
}));
vi.mock("../resumeAgentSession", () => ({
  resumeAgentSession: (...args: unknown[]) => resumeAgentSession(...args),
}));
vi.mock("../terminateSession", () => ({
  terminateAgentSession: (...args: unknown[]) => terminateAgentSession(...args),
}));
vi.mock("../clockOut", () => ({
  clockOutAgent: (...args: unknown[]) => clockOutAgent(...args),
  clockOutAll: (...args: unknown[]) => clockOutAll(...args),
}));

const {
  ConfirmDeleteDialog,
  ConfirmRestartDialog,
  ConfirmResumeDialog,
  ConfirmTerminateDialog,
  ConfirmBotStartDialog,
  ConfirmClockOutDialog,
} = await import("../confirmDialogs");

function mkProfile(id: string, name: string): AgentProfile {
  return {
    id,
    name,
    role: "eng",
    seed: id,
    createdAt: Date.now(),
    deskIndex: 0,
  };
}

const initialState = useAppStore.getState();
const allActions = [
  deleteAgent,
  restartAgentSession,
  resumeAgentSession,
  terminateAgentSession,
  clockOutAgent,
  clockOutAll,
  startBot,
];

beforeEach(() => {
  useAppStore.setState(initialState, true);
  // 봇 시작만 오케스트레이터가 아니라 스토어 액션 — 같은 방식으로 스파이한다.
  useAppStore.setState({ startBot: (agentId: string) => startBot(agentId) });
  allActions.forEach((fn) => fn.mockClear());
});

afterEach(() => cleanup());

interface Spec {
  /** 컴포넌트 이름(테스트 라벨). */
  label: string;
  Comp: () => ReactElement | null;
  /** `confirm-<slug>-dialog` / `-warning` 클래스 어근. */
  slug: string;
  /** 이 다이얼로그를 여는 모달 상태. */
  modal: ModalState;
  /** 이 다이얼로그가 반응하면 안 되는 다른 종류의 모달 상태. */
  otherModal: ModalState;
  confirmLabel: string;
  /** 확인 시 호출돼야 하는 목. */
  action: ReturnType<typeof vi.fn>;
  /** 확인 시 넘어가야 하는 인자. */
  args: unknown[];
  /** 경고 문단 문구. */
  warning: RegExp;
  /** true면 세션이 안 돌아도 경고를 항상 띄운다(전체 퇴근 / 봇 모드 시작). */
  alwaysWarns: boolean;
  /** 대상 캐릭터 이름을 본문에 표시하는가(전체 퇴근은 아님). */
  showsName: boolean;
}

const specs: Spec[] = [
  {
    label: "ConfirmDeleteDialog",
    Comp: ConfirmDeleteDialog,
    slug: "delete",
    modal: { kind: "confirm-delete", agentId: "a1" },
    otherModal: { kind: "confirm-restart", agentId: "a1" },
    confirmLabel: "삭제",
    action: deleteAgent,
    args: ["a1"],
    warning: /실행 중인 세션이 종료됩니다/,
    alwaysWarns: false,
    showsName: true,
  },
  {
    label: "ConfirmRestartDialog",
    Comp: ConfirmRestartDialog,
    slug: "restart",
    modal: { kind: "confirm-restart", agentId: "a1" },
    otherModal: { kind: "confirm-delete", agentId: "a1" },
    confirmLabel: "재시작",
    action: restartAgentSession,
    args: ["a1"],
    warning: /실행 중인 세션이 종료되고 스크롤백이 지워집니다/,
    alwaysWarns: false,
    showsName: true,
  },
  {
    label: "ConfirmResumeDialog",
    Comp: ConfirmResumeDialog,
    slug: "resume",
    modal: { kind: "confirm-resume", agentId: "a1", sessionId: "abc-123" },
    otherModal: { kind: "confirm-restart", agentId: "a1" },
    confirmLabel: "이어하기",
    action: resumeAgentSession,
    args: ["a1", "abc-123"],
    warning: /실행 중인 세션이 종료되고 스크롤백이 지워집니다/,
    alwaysWarns: false,
    showsName: true,
  },
  {
    label: "ConfirmTerminateDialog",
    Comp: ConfirmTerminateDialog,
    slug: "terminate",
    modal: { kind: "confirm-terminate", agentId: "a1" },
    otherModal: { kind: "confirm-restart", agentId: "a1" },
    confirmLabel: "종료",
    action: terminateAgentSession,
    args: ["a1"],
    warning: /실행 중인 세션이 종료됩니다/,
    alwaysWarns: false,
    showsName: true,
  },
  {
    label: "ConfirmBotStartDialog",
    Comp: ConfirmBotStartDialog,
    slug: "bot-start",
    modal: { kind: "confirm-bot-start", agentId: "a1" },
    otherModal: { kind: "confirm-terminate", agentId: "a1" },
    confirmLabel: "그래도 켜기",
    action: startBot,
    args: ["a1"],
    warning: /맨 셸에서 봇을 켜면/,
    alwaysWarns: true,
    showsName: true,
  },
  {
    label: "ConfirmClockOutDialog (개별)",
    Comp: ConfirmClockOutDialog,
    slug: "clock-out",
    modal: { kind: "confirm-clock-out", agentId: "a1" },
    otherModal: { kind: "confirm-delete", agentId: "a1" },
    confirmLabel: "퇴근",
    action: clockOutAgent,
    args: ["a1"],
    warning: /진행 중인 세션이 종료됩니다/,
    alwaysWarns: false,
    showsName: true,
  },
  {
    label: "ConfirmClockOutDialog (전체)",
    Comp: ConfirmClockOutDialog,
    slug: "clock-out",
    modal: { kind: "confirm-clock-out-all" },
    otherModal: { kind: "confirm-delete", agentId: "a1" },
    confirmLabel: "퇴근",
    action: clockOutAll,
    args: [],
    warning: /진행 중인 세션이 모두 종료됩니다/,
    alwaysWarns: true,
    showsName: false,
  },
];

describe.each(specs)("$label", (spec: Spec) => {
  const { Comp } = spec;

  function open(status?: "running" | "exited") {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난")); // addAgent 직후 세션 status: starting
    if (status) s.setSessionState({ agentId: "a1", status });
    s.openModal(spec.modal);
  }

  it("해당 modal kind가 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<Comp />);
    expect(container.firstChild).toBeNull();
  });

  it("다른 종류의 모달이 열려 있어도 렌더하지 않는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal(spec.otherModal);

    const { container } = render(<Comp />);
    expect(container.firstChild).toBeNull();
  });

  it("본문이 확인 대상을 알려준다", () => {
    open();
    render(<Comp />);
    // 개별 대상이 있는 다이얼로그는 이름을, 전체 퇴근은 근무 중 인원 수를 쓴다.
    if (spec.showsName) expect(screen.getByText("코난")).toBeTruthy();
    else expect(screen.getByText(/근무 중인 캐릭터 1명/)).toBeTruthy();
  });

  it("세션이 실행 중이면 경고를 표시한다 (running)", () => {
    open("running");
    render(<Comp />);
    expect(screen.getByText(spec.warning)).toBeTruthy();
  });

  it(
    spec.alwaysWarns
      ? "세션이 종료(exited) 상태여도 경고를 표시한다"
      : "세션이 종료(exited) 상태면 경고를 표시하지 않는다",
    () => {
      open("exited");
      render(<Comp />);
      if (spec.alwaysWarns) expect(screen.getByText(spec.warning)).toBeTruthy();
      else expect(screen.queryByText(spec.warning)).toBeNull();
    }
  );

  it("경고 문단은 종류별 클래스와 --accent-warn 토큰을 쓴다", () => {
    open("running");
    const { container } = render(<Comp />);

    const warn = container.querySelector(`.confirm-${spec.slug}-warning`);
    expect(warn).not.toBeNull();
    // 하드코딩 색(#e0574a)이 아니라 테마 토큰이어야 한다.
    expect((warn as HTMLElement).getAttribute("style")).toContain("var(--accent-warn)");
  });

  it("패널은 종류별 클래스를 단다", () => {
    open();
    const { container } = render(<Comp />);
    expect(container.querySelector(`.pixel-panel.confirm-${spec.slug}-dialog`)).not.toBeNull();
  });

  it("확인 시 액션을 호출하고 모달을 닫는다", () => {
    open();
    render(<Comp />);
    fireEvent.click(screen.getByRole("button", { name: spec.confirmLabel }));

    expect(spec.action).toHaveBeenCalledWith(...spec.args);
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("취소 시 액션을 호출하지 않고 모달만 닫는다", () => {
    open();
    render(<Comp />);
    fireEvent.click(screen.getByRole("button", { name: "취소" }));

    expect(spec.action).not.toHaveBeenCalled();
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("backdrop 왼쪽 클릭으로 닫히고 액션은 실행되지 않는다", () => {
    open();
    const { container } = render(<Comp />);
    const backdrop = container.querySelector(".modal-backdrop")!;

    fireEvent.mouseDown(backdrop, { button: 0 });

    expect(spec.action).not.toHaveBeenCalled();
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("패널 위 mousedown은 모달을 닫지 않는다", () => {
    open();
    const { container } = render(<Comp />);
    const panel = container.querySelector(".pixel-panel")!;

    fireEvent.mouseDown(panel, { button: 0 });

    expect(useAppStore.getState().modal).toEqual(spec.modal);
  });

  it("backdrop 오른쪽 버튼 mousedown은 모달을 닫지 않는다", () => {
    open();
    const { container } = render(<Comp />);
    const backdrop = container.querySelector(".modal-backdrop")!;

    fireEvent.mouseDown(backdrop, { button: 2 });

    expect(useAppStore.getState().modal).toEqual(spec.modal);
  });
});

// ---- 종류별로 진짜 다른 동작 ----

describe("ConfirmResumeDialog", () => {
  it("확인 시 캡처된 native sessionId를 함께 넘긴다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-resume", agentId: "a1", sessionId: "abc-123" });

    render(<ConfirmResumeDialog />);
    fireEvent.click(screen.getByRole("button", { name: "이어하기" }));

    expect(resumeAgentSession).toHaveBeenCalledWith("a1", "abc-123");
  });
});

describe("ConfirmTerminateDialog", () => {
  it("경고에 탕비실 대기/재소환 안내가 들어간다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.setSessionState({ agentId: "a1", status: "running" });
    s.openModal({ kind: "confirm-terminate", agentId: "a1" });

    render(<ConfirmTerminateDialog />);

    expect(screen.getByText(/탕비실에서 대기/)).toBeTruthy();
  });
});

describe("ConfirmBotStartDialog", () => {
  it("맨 셸 경고와 함께 '그래도 켤까요?'를 묻는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-bot-start", agentId: "a1" });

    render(<ConfirmBotStartDialog />);

    expect(screen.getByText(/실행 중인지 확인할 수 없습니다/)).toBeTruthy();
    expect(screen.getByText("그래도 봇 모드를 켤까요?")).toBeTruthy();
  });
});

describe("ConfirmClockOutDialog", () => {
  it("전체 퇴근은 근무 중인 인원 수를 표시한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.addAgent(mkProfile("a2", "김전일"));
    s.addAgent(mkProfile("a3", "소년"));
    s.clockOut("a3"); // 이미 퇴근 -> 근무 중 카운트에서 제외
    s.openModal({ kind: "confirm-clock-out-all" });

    render(<ConfirmClockOutDialog />);

    expect(screen.getByText(/근무 중인 캐릭터 2명을 모두 퇴근시킬까요\?/)).toBeTruthy();
  });

  it("개별 퇴근은 clockOutAll을 호출하지 않는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-clock-out", agentId: "a1" });

    render(<ConfirmClockOutDialog />);
    fireEvent.click(screen.getByRole("button", { name: "퇴근" }));

    expect(clockOutAgent).toHaveBeenCalledWith("a1");
    expect(clockOutAll).not.toHaveBeenCalled();
  });

  it("전체 퇴근은 clockOutAgent를 호출하지 않는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1", "코난"));
    s.openModal({ kind: "confirm-clock-out-all" });

    render(<ConfirmClockOutDialog />);
    fireEvent.click(screen.getByRole("button", { name: "퇴근" }));

    expect(clockOutAll).toHaveBeenCalledTimes(1);
    expect(clockOutAgent).not.toHaveBeenCalled();
  });
});
