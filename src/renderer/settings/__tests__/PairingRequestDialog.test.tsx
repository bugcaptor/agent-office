// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/PairingRequestDialog.test.tsx
//
// 페어링 승인 다이얼로그. 회귀 배경: 6자리 코드가 설정 다이얼로그의
// `peerShareEnabled` 게이트 안에서만 그려져서, **웹 호스팅만 켠 사용자에게는
// 코드가 어디에도 안 떴다**(브라우저는 통과할 수 없는 숫자를 기다린다).
// 그래서 여기서 확인하는 계약은 셋이다 — 설정과 무관하게 코드가 뜬다,
// 승인/거부가 실제 IPC를 부르고 목록에서 빠진다, 만료되면 스스로 사라진다.
import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const approvePairing = vi.fn<(id: string, p: string) => Promise<void>>(() => Promise.resolve());
const rejectPairing = vi.fn<(id: string) => Promise<void>>(() => Promise.resolve());

vi.mock("../../ipc/peerApi", () => ({
  peerApi: {
    approvePairing: (id: string, p: string) => approvePairing(id, p),
    rejectPairing: (id: string) => rejectPairing(id),
  },
}));

import { useAppStore } from "../../store/appStore";
import { PairingRequestDialog } from "../PairingRequestDialog";

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  approvePairing.mockClear();
  rejectPairing.mockClear();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("PairingRequestDialog", () => {
  it("대기 중인 페어링이 없으면 아무것도 그리지 않는다", () => {
    const { container } = render(<PairingRequestDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("브라우저 요청이 오면 설정을 열지 않아도 6자리 코드가 보인다", () => {
    useAppStore.getState().setPeerPending([
      { pairingId: "p1", code: "042317", viewerName: "휴대폰 브라우저", clientKind: "web" },
    ]);
    render(<PairingRequestDialog />);
    expect(screen.getByText("042317")).toBeTruthy();
    expect(screen.getByText(/웹 브라우저/)).toBeTruthy();
  });

  it("승인하면 권한과 함께 IPC를 부르고 목록에서 빠진다", () => {
    useAppStore
      .getState()
      .setPeerPending([{ pairingId: "p1", code: "111111", viewerName: "브라우저" }]);
    render(<PairingRequestDialog />);
    fireEvent.click(screen.getByText("승인 (읽기 전용)"));
    expect(approvePairing).toHaveBeenCalledWith("p1", "readOnly");
    expect(useAppStore.getState().peerPending).toHaveLength(0);
  });

  it("거부하면 IPC를 부르고 다음 대기 요청이 올라온다", () => {
    useAppStore.getState().setPeerPending([
      { pairingId: "p1", code: "111111", viewerName: "손님1" },
      { pairingId: "p2", code: "222222", viewerName: "손님2" },
    ]);
    render(<PairingRequestDialog />);
    expect(screen.getByText("111111")).toBeTruthy();
    fireEvent.click(screen.getByText("거부"));
    expect(rejectPairing).toHaveBeenCalledWith("p1");
    expect(screen.getByText("222222")).toBeTruthy();
  });

  it("코드가 만료되면 스스로 사라진다", () => {
    vi.useFakeTimers();
    useAppStore
      .getState()
      .setPeerPending([
        { pairingId: "p1", code: "333333", viewerName: "손님", expiresInMs: 1000 },
      ]);
    const { container } = render(<PairingRequestDialog />);
    expect(screen.getByText("333333")).toBeTruthy();
    act(() => {
      vi.advanceTimersByTime(1001);
    });
    expect(useAppStore.getState().peerPending).toHaveLength(0);
    expect(container.firstChild).toBeNull();
  });
});
