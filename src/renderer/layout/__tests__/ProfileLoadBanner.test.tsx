// @vitest-environment jsdom
//
// src/renderer/layout/__tests__/ProfileLoadBanner.test.tsx
//
// profiles.json 로드 실패 경고(수정 F2): 정상일 땐 아무것도 그리지 않고,
// 실패 상태면 백엔드 사유와 재시도 버튼을 보인다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const retryProfileLoad = vi.fn();

vi.mock("../../store/profileLoad", async () => {
  const state = await import("../../store/profileLoadState");
  return { ...state, retryProfileLoad: () => retryProfileLoad() };
});

const { useProfileLoadStore } = await import("../../store/profileLoadState");
const { ProfileLoadBanner } = await import("../ProfileLoadBanner");

beforeEach(() => {
  retryProfileLoad.mockClear();
  useProfileLoadStore.setState({ error: null, retrying: false });
});

afterEach(cleanup);

describe("ProfileLoadBanner", () => {
  it("renders nothing while profiles loaded normally", () => {
    const { container } = render(<ProfileLoadBanner />);
    expect(container.firstChild).toBeNull();
  });

  it("shows the backend reason and retries on click", () => {
    useProfileLoadStore.setState({ error: "profiles.json을 읽을 수 없습니다 (원본 보존: /x/profiles.json.corrupt-1)" });
    render(<ProfileLoadBanner />);
    expect(screen.getByRole("alert").textContent).toContain("profiles.json.corrupt-1");
    fireEvent.click(screen.getByRole("button"));
    expect(retryProfileLoad).toHaveBeenCalledTimes(1);
  });

  it("disables the button while a retry is in flight", () => {
    useProfileLoadStore.setState({ error: "x", retrying: true });
    render(<ProfileLoadBanner />);
    expect((screen.getByRole("button") as HTMLButtonElement).disabled).toBe(true);
  });
});
