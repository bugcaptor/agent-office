// src/renderer/ipc/__tests__/osNotify.test.ts
//
// OS 데스크탑 알림 래퍼(이슈 #39)의 권한 흐름 검증. 플러그인 모듈을 모킹해
// 권한 확인/요청/발송이 최초 1회만 일어나는지, 거부/에러가 조용히 삼켜지는지 본다.
import { afterEach, describe, expect, it, vi } from "vitest";

const { isPermissionGranted, requestPermission, sendNotification } = vi.hoisted(() => ({
  isPermissionGranted: vi.fn(),
  requestPermission: vi.fn(),
  sendNotification: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted,
  requestPermission,
  sendNotification,
}));

import {
  maybeSendOsNotification,
  __resetOsNotifyPermissionCacheForTests,
} from "../osNotify";

afterEach(() => {
  __resetOsNotifyPermissionCacheForTests();
  vi.clearAllMocks();
});

describe("maybeSendOsNotification", () => {
  it("sends when permission is already granted and checks permission only once", async () => {
    isPermissionGranted.mockResolvedValue(true);

    await maybeSendOsNotification("Agent", "완료");
    expect(requestPermission).not.toHaveBeenCalled();
    expect(sendNotification).toHaveBeenCalledWith({ title: "Agent", body: "완료" });

    await maybeSendOsNotification("Agent", "또 완료");
    expect(isPermissionGranted).toHaveBeenCalledTimes(1); // 캐시됨
    expect(sendNotification).toHaveBeenCalledTimes(2);
  });

  it("requests permission once when not yet granted, then sends", async () => {
    isPermissionGranted.mockResolvedValue(false);
    requestPermission.mockResolvedValue("granted");

    await maybeSendOsNotification("Agent", "완료");
    expect(requestPermission).toHaveBeenCalledTimes(1);
    expect(sendNotification).toHaveBeenCalledTimes(1);
  });

  it("does not send when permission is denied", async () => {
    isPermissionGranted.mockResolvedValue(false);
    requestPermission.mockResolvedValue("denied");

    await maybeSendOsNotification("Agent", "완료");
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("swallows plugin/runtime errors without throwing", async () => {
    isPermissionGranted.mockRejectedValue(new Error("no runtime"));
    await expect(maybeSendOsNotification("Agent", "완료")).resolves.toBeUndefined();
    expect(sendNotification).not.toHaveBeenCalled();
  });
});
