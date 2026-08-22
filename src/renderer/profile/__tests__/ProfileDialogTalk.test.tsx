// @vitest-environment jsdom
//
// src/renderer/profile/__tests__/ProfileDialogTalk.test.tsx
//
// 프로필 다이얼로그의 "동료 메시지 받기"(docs/agent-talk-design.md). 계약 셋:
//  1) 기본은 켜짐 — 필드가 없는 기존 프로필도 체크된 상태로 열린다.
//  2) 끄고 저장하면 `talkReceive: false`가 프로필에 남는다.
//  3) 다시 켜고 저장하면 필드가 사라진다(기본값은 저장하지 않는 관례).
//  4) 새 캐릭터도 기본은 켜짐 — 만들자마자 말을 걸 수 있다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../office/gen/characterFactory", () => ({
  generateSpritePreview: (seed: string) => `data:image/png;base64,${seed}`,
}));
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    createSession: vi.fn().mockResolvedValue({ sessionId: "s1", state: "starting" }),
    listAvailableShells: vi.fn().mockResolvedValue([]),
    setAppSettings: vi.fn().mockResolvedValue(undefined),
    saveState: vi.fn().mockResolvedValue(undefined),
    ttsListVoices: vi.fn().mockResolvedValue([]),
  },
}));
vi.mock("../../sound/soundManager", () => ({
  previewKeyboardSound: vi.fn(),
  previewVoice: vi.fn().mockResolvedValue(""),
}));
vi.mock("../../portrait/PortraitEditor", () => ({ PortraitEditor: () => null }));
vi.mock("../../sprite/SpriteEditor", () => ({ SpriteEditor: () => null }));

import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";
const { ProfileDialog } = await import("../ProfileDialog");

const initialState = useAppStore.getState();

const AGENT: AgentProfile = {
  id: "a1",
  name: "무지",
  role: "eng",
  note: "",
  seed: "seed-1",
  createdAt: 0,
  deskIndex: 0,
  archetype: "elf",
};

function openEdit(agent: AgentProfile = AGENT) {
  useAppStore.getState().addAgent(agent);
  useAppStore.getState().openModal({ kind: "profile-edit", agentId: agent.id });
}

const toggle = () => screen.getByRole("checkbox", { name: /동료 메시지 받기/ }) as HTMLInputElement;

beforeEach(() => {
  useAppStore.setState(initialState, true);
});
afterEach(() => cleanup());

describe("ProfileDialog · 동료 메시지 받기", () => {
  it("필드가 없는 기존 프로필은 켜진 상태로 열린다", () => {
    openEdit();
    render(<ProfileDialog />);
    expect(toggle().checked).toBe(true);
  });

  it("false로 저장된 프로필은 꺼진 상태로 열린다", () => {
    openEdit({ ...AGENT, talkReceive: false });
    render(<ProfileDialog />);
    expect(toggle().checked).toBe(false);
  });

  it("끄고 저장하면 talkReceive: false가 프로필에 남는다", () => {
    openEdit();
    render(<ProfileDialog />);
    fireEvent.click(toggle());
    fireEvent.click(screen.getByRole("button", { name: "저장" }));
    expect(useAppStore.getState().agents.a1.talkReceive).toBe(false);
  });

  it("다시 켜고 저장하면 필드가 사라진다(기본값은 저장하지 않는다)", () => {
    openEdit({ ...AGENT, talkReceive: false });
    render(<ProfileDialog />);
    fireEvent.click(toggle());
    fireEvent.click(screen.getByRole("button", { name: "저장" }));
    expect(useAppStore.getState().agents.a1.talkReceive).toBeUndefined();
  });

  it("새 캐릭터는 기본이 켜짐이고 필드 없이 저장된다", () => {
    useAppStore.getState().openModal({ kind: "profile-create" });
    render(<ProfileDialog />);
    expect(toggle().checked).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "저장" }));
    const created = Object.values(useAppStore.getState().agents);
    expect(created).toHaveLength(1);
    expect(created[0].talkReceive).toBeUndefined();
  });
});
