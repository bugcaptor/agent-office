// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/SystemTabOpencodeKey.test.tsx
//
// 시스템 탭의 OpenCode API 키 칸: OpenCode Go 구독 사용량을 하단 뱃지·상세
// 창에 싣는 키다. 저장소는 공용 API 키 저장소(`setApiKeys`의 셋째 칸)이고,
// 앞의 두 칸(Anthropic·OpenRouter)은 건드리지 않는다.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ApiKeyStatus } from "@shared/types";

const apiKeyStatus = vi.fn<() => Promise<ApiKeyStatus>>();
const setApiKeys = vi.fn<(a?: string, o?: string, c?: string) => Promise<ApiKeyStatus>>();

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    apiKeyStatus: () => apiKeyStatus(),
    setApiKeys: (a?: string, o?: string, c?: string) => setApiKeys(a, o, c),
  },
}));

import { useAppStore } from "../../store/appStore";
import { SystemTab } from "../SystemTab";

const initialState = useAppStore.getState();

const STATUS: ApiKeyStatus = {
  anthropicSet: false,
  anthropicFromEnv: false,
  openrouterSet: false,
  openrouterFromEnv: false,
  opencodeSet: false,
  opencodeFromEnv: false,
};

beforeEach(() => {
  useAppStore.setState(initialState, true);
  apiKeyStatus.mockReset().mockResolvedValue(STATUS);
  setApiKeys.mockReset().mockResolvedValue({ ...STATUS, opencodeSet: true });
});

afterEach(() => cleanup());

describe("SystemTab · OpenCode API 키", () => {
  it("상태를 보여주고, 저장은 셋째 칸만 채운다", async () => {
    render(<SystemTab />);
    await screen.findByText(/OpenCode 키 없음/);
    expect(screen.getByText("OpenCode API 키 (사용량 표시)")).toBeTruthy();

    const input = screen.getByPlaceholderText("sk-…") as HTMLInputElement;
    expect(input.type).toBe("password");
    fireEvent.change(input, { target: { value: "sk-oc-1" } });
    fireEvent.click(screen.getByText("키 저장"));
    await waitFor(() => expect(setApiKeys).toHaveBeenCalledWith(undefined, undefined, "sk-oc-1"));
    await screen.findByText(/OpenCode 키 있음/);
  });

  it("삭제는 빈 문자열을 셋째 칸에 보낸다", async () => {
    apiKeyStatus.mockResolvedValue({ ...STATUS, opencodeSet: true });
    setApiKeys.mockResolvedValue(STATUS);
    render(<SystemTab />);
    fireEvent.click(await screen.findByText("키 삭제"));
    await waitFor(() => expect(setApiKeys).toHaveBeenCalledWith(undefined, undefined, ""));
  });
});
