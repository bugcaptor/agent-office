// @vitest-environment jsdom
//
// src/renderer/profile/__tests__/ProfileDialogVoice.test.tsx
//
// 프로필 다이얼로그의 보이스(TTS) 선택. 계약 셋:
//  1) TTS가 꺼져 있으면 고를 수 없고 사유를 안내한다(목록 조회도 하지 않는다).
//  2) 켜져 있으면 백엔드 목록을 라벨 요약과 함께 보여주고, 고른 값이 저장된다.
//  3) 미리듣기는 **고른 보이스로** 발화한다(저장 전에 확인할 수 있어야 한다).
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, TtsSpeakRequest, TtsVoiceOption } from "@shared/types";

const ttsListVoices = vi.fn<() => Promise<TtsVoiceOption[]>>();
const previewVoice = vi.fn<(o?: Partial<TtsSpeakRequest>) => Promise<string>>();

vi.mock("../../office/gen/characterFactory", () => ({
  generateSpritePreview: (seed: string) => `data:image/png;base64,${seed}`,
}));
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    createSession: vi.fn().mockResolvedValue({ sessionId: "s1", state: "starting" }),
    listAvailableShells: vi.fn().mockResolvedValue([]),
    setAppSettings: vi.fn().mockResolvedValue(undefined),
    saveState: vi.fn().mockResolvedValue(undefined),
    ttsListVoices: () => ttsListVoices(),
  },
}));
vi.mock("../../sound/soundManager", () => ({
  previewKeyboardSound: vi.fn(),
  previewVoice: (o?: Partial<TtsSpeakRequest>) => previewVoice(o),
}));
vi.mock("../../portrait/PortraitEditor", () => ({ PortraitEditor: () => null }));
vi.mock("../../sprite/SpriteEditor", () => ({ SpriteEditor: () => null }));

import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";
const { ProfileDialog } = await import("../ProfileDialog");

const initialState = useAppStore.getState();

const VOICES: TtsVoiceOption[] = [
  { voiceId: "v-a", name: "Rachel", labels: "female · young" },
  { voiceId: "v-b", name: "Arnold", labels: "male · middle_aged" },
];

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

function open(settings: Partial<AppSettings> = {}, agent: AgentProfile = AGENT) {
  useAppStore.getState().addAgent(agent);
  useAppStore.getState().updateAppSettings(settings);
  useAppStore.getState().openModal({ kind: "profile-edit", agentId: agent.id });
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  ttsListVoices.mockReset().mockResolvedValue(VOICES);
  previewVoice.mockReset().mockResolvedValue("[curious] 이거 해도 될까요?");
});
afterEach(() => cleanup());

describe("ProfileDialog · 목소리(TTS) 선택", () => {
  it("TTS가 꺼져 있으면 비활성 + 사유 안내이고 목록을 조회하지 않는다", () => {
    open({ ttsEnabled: false });
    render(<ProfileDialog />);
    const select = screen.getByRole("combobox", { name: /목소리/ }) as HTMLSelectElement;
    expect(select.disabled).toBe(true);
    expect(screen.getByText(/‘알림 대사 읽어주기\(TTS\)’를 켜면/)).toBeTruthy();
    expect(ttsListVoices).not.toHaveBeenCalled();
  });

  it("켜져 있으면 이름과 라벨 요약을 함께 보여주고 선택이 프로필에 저장된다", async () => {
    open({ ttsEnabled: true });
    render(<ProfileDialog />);
    await waitFor(() => expect(ttsListVoices).toHaveBeenCalled());
    const select = screen.getByRole("combobox", { name: /목소리/ }) as HTMLSelectElement;
    await waitFor(() => expect(select.disabled).toBe(false));
    // 기본은 "자동" — 종족 기반 배정에 맡긴다.
    expect(select.value).toBe("");
    await screen.findByText("Arnold — male · middle_aged");

    fireEvent.change(select, { target: { value: "v-b" } });
    fireEvent.click(screen.getByRole("button", { name: "저장" }));
    expect(useAppStore.getState().agents.a1.voiceId).toBe("v-b");
  });

  it("자동으로 되돌리면 프로필에서 지정이 사라진다", async () => {
    open({ ttsEnabled: true }, { ...AGENT, voiceId: "v-b" });
    render(<ProfileDialog />);
    await waitFor(() => expect(ttsListVoices).toHaveBeenCalled());
    const select = screen.getByRole("combobox", { name: /목소리/ }) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("v-b"));
    fireEvent.change(select, { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "저장" }));
    expect(useAppStore.getState().agents.a1.voiceId).toBeUndefined();
  });

  it("목록에 없는 지정도 선택값으로 살려 둔다(저장 시 유실 방지)", async () => {
    open({ ttsEnabled: true }, { ...AGENT, voiceId: "v-gone" });
    render(<ProfileDialog />);
    await waitFor(() => expect(ttsListVoices).toHaveBeenCalled());
    const select = screen.getByRole("combobox", { name: /목소리/ }) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("v-gone"));
    await screen.findByText(/목록에 없음/);
  });

  it("미리듣기는 고른 보이스와 확정된 archetype으로 발화한다", async () => {
    open({ ttsEnabled: true });
    render(<ProfileDialog />);
    await waitFor(() => expect(ttsListVoices).toHaveBeenCalled());
    const select = screen.getByRole("combobox", { name: /목소리/ }) as HTMLSelectElement;
    await waitFor(() => expect(select.disabled).toBe(false));
    fireEvent.change(select, { target: { value: "v-a" } });
    fireEvent.click(screen.getByRole("button", { name: "미리듣기" }));
    await waitFor(() =>
      expect(previewVoice).toHaveBeenCalledWith(
        expect.objectContaining({ agentId: "a1", voiceId: "v-a", archetype: "elf", seed: "seed-1" })
      )
    );
    await screen.findByText(/발화: \[curious\]/);
  });

  it("키가 없으면(missing_elevenlabs_key) 설정으로 안내한다", async () => {
    ttsListVoices.mockRejectedValue("missing_elevenlabs_key: ElevenLabs API 키가 설정되지 않았습니다");
    open({ ttsEnabled: true });
    render(<ProfileDialog />);
    await screen.findByText(/ElevenLabs API 키를 저장하면/);
    const select = screen.getByRole("combobox", { name: /목소리/ }) as HTMLSelectElement;
    expect(select.disabled).toBe(true);
  });
});
