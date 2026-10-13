import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "@shared/types";
import { Events } from "@shared/ipc";

const { listen, emit, applyLanguageSetting } = vi.hoisted(() => {
  let handler: ((event: { payload: unknown }) => void) | undefined;
  return {
    listen: vi.fn((_event: string, callback: typeof handler) => {
      handler = callback;
      return Promise.resolve(vi.fn());
    }),
    emit: (payload: unknown) => handler?.({ payload }),
    applyLanguageSetting: vi.fn(),
  };
});

vi.mock("@tauri-apps/api/event", () => ({ listen }));
vi.mock("../../i18n", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../i18n")>()),
  applyLanguageSetting,
}));

import { useAppStore } from "../../store/appStore";
import { installSettingsSync } from "../settingsSyncBridge";

const initialState = useAppStore.getState();
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
let dispose = () => {};

beforeEach(async () => {
  useAppStore.setState(initialState, true);
  vi.clearAllMocks();
  dispose = installSettingsSync();
  await flush();
});
afterEach(() => dispose());

describe("installSettingsSync", () => {
  it("app-settings-changed로 받은 백엔드 설정으로 사본을 교체한다", () => {
    expect(listen).toHaveBeenCalledWith(Events.appSettingsChanged, expect.any(Function));
    const next: AppSettings = {
      ...useAppStore.getState().appSettings,
      gitStatusEnabled: false,
      language: "en",
    };
    emit(next);
    expect(useAppStore.getState().appSettings).toEqual(next);
    expect(applyLanguageSetting).toHaveBeenCalledWith("en");
  });

  it("CLI 변경 뒤 GUI 저장이 그 값을 되돌리지 않는다", async () => {
    const { tauriApi } = await import("../tauriApi");
    const save = vi.spyOn(tauriApi, "setAppSettings").mockResolvedValue(undefined);
    emit({ ...useAppStore.getState().appSettings, gitStatusEnabled: false });
    useAppStore.getState().updateAppSettings({ typingSoundEnabled: false });
    expect(save).toHaveBeenCalledWith({ typingSoundEnabled: false });
    expect(useAppStore.getState().appSettings.gitStatusEnabled).toBe(false);
    save.mockRestore();
  });
});
