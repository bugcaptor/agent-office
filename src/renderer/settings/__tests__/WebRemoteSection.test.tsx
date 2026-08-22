// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/WebRemoteSection.test.tsx
//
// 웹 원격 설정 섹션 — tailscale serve HTTPS 블록의 노출 조건(bind=tailnet +
// tailnet 탐지)과 켜기/끄기 대행, 그리고 접속주소 복사 버튼이 **전체 URL**을
// clipboard에 쓰는지 확인한다. webRemoteApi를 모킹해 실 IPC 없이 검증한다.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TailscaleServeStatus, WebRemoteStatus } from "../../ipc/webRemoteApi";

const hostStatus = vi.fn<() => Promise<WebRemoteStatus>>();
const serveStatus = vi.fn<() => Promise<TailscaleServeStatus>>();
const serveEnable = vi.fn<() => Promise<void>>(() => Promise.resolve());
const serveDisable = vi.fn<() => Promise<void>>(() => Promise.resolve());

vi.mock("../../ipc/webRemoteApi", () => ({
  webRemoteApi: {
    hostStatus: () => hostStatus(),
    serveStatus: () => serveStatus(),
    serveEnable: () => serveEnable(),
    serveDisable: () => serveDisable(),
    revokeClient: () => Promise.resolve(),
    setClientPermission: () => Promise.resolve(),
  },
}));

import { useAppStore } from "../../store/appStore";
import { WebRemoteSection } from "../WebRemoteSection";

const initialState = useAppStore.getState();

function host(patch: Partial<WebRemoteStatus> = {}): WebRemoteStatus {
  return {
    enabled: true,
    running: true,
    port: 47800,
    hostName: "zm4mini",
    addressHint: "100.88.236.3",
    bind: "tailnet",
    tailnetFound: true,
    clients: [],
    pending: [],
    ...patch,
  };
}

function serve(patch: Partial<TailscaleServeStatus> = {}): TailscaleServeStatus {
  return {
    cliFound: true,
    backendRunning: true,
    dnsName: "zm4mini.tailc90d0d.ts.net",
    httpsPort: 47443,
    registered: false,
    conflict: false,
    httpsUrl: "https://zm4mini.tailc90d0d.ts.net:47443/web/",
    ...patch,
  };
}

/** 섹션은 `webRemoteEnabled`가 켜져 있어야 본문을 그린다. */
function hydrate(bind: "tailnet" | "all" = "tailnet") {
  useAppStore.setState({
    appSettings: {
      ...initialState.appSettings,
      webRemoteEnabled: true,
      talkEnabled: false,
      talkMaxTurns: 6,
      talkIdleQuietMs: 3000,
      webRemoteBind: bind,
      webRemotePort: 47800,
    },
  });
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  hostStatus.mockReset();
  serveStatus.mockReset();
  serveEnable.mockClear();
  serveDisable.mockClear();
  hostStatus.mockResolvedValue(host());
  serveStatus.mockResolvedValue(serve());
});

afterEach(() => cleanup());

describe("WebRemoteSection · tailscale serve HTTPS", () => {
  it("tailnet 바인드 + 탐지 성공이면 HTTPS 블록과 켜기 버튼을 보인다", async () => {
    hydrate();
    render(<WebRemoteSection />);

    expect(await screen.findByRole("button", { name: "HTTPS 켜기" })).toBeTruthy();
    await waitFor(() => expect(serveStatus).toHaveBeenCalled());
  });

  it("tailnet을 못 찾으면 HTTPS 블록을 아예 조회하지 않는다", async () => {
    hostStatus.mockResolvedValue(host({ tailnetFound: false }));
    hydrate();
    render(<WebRemoteSection />);

    // 미탐지 안내가 뜨는 것이 이 상태의 표식이다.
    await screen.findByText(/Tailscale이 감지되지 않았습니다/);
    expect(screen.queryByRole("button", { name: "HTTPS 켜기" })).toBeNull();
    expect(serveStatus).not.toHaveBeenCalled();
  });

  it("bind가 tailnet이 아니면 HTTPS 블록이 없다", async () => {
    hydrate("all");
    render(<WebRemoteSection />);

    await waitFor(() => expect(hostStatus).toHaveBeenCalled());
    expect(screen.queryByRole("button", { name: "HTTPS 켜기" })).toBeNull();
    expect(serveStatus).not.toHaveBeenCalled();
  });

  it("CLI 미탐지면 버튼 없이 안내문만 띄운다", async () => {
    serveStatus.mockResolvedValue(
      serve({ cliFound: false, dnsName: null, httpsUrl: null, backendRunning: false }),
    );
    hydrate();
    render(<WebRemoteSection />);

    await screen.findByText(/Tailscale 명령줄 도구를 찾지 못했습니다/);
    expect(screen.queryByRole("button", { name: "HTTPS 켜기" })).toBeNull();
    expect(screen.queryByRole("button", { name: "HTTPS 끄기" })).toBeNull();
  });

  it("등록돼 있으면 https 주소와 끄기 버튼을 보이고, 끄면 상태를 다시 읽는다", async () => {
    serveStatus.mockResolvedValue(
      serve({ registered: true, upstream: "http://100.88.236.3:47800" }),
    );
    hydrate();
    render(<WebRemoteSection />);

    const off = await screen.findByRole("button", { name: "HTTPS 끄기" });
    expect(screen.getByText("https://zm4mini.tailc90d0d.ts.net:47443/web/")).toBeTruthy();

    fireEvent.click(off);
    await waitFor(() => expect(serveDisable).toHaveBeenCalledTimes(1));
    // 끈 뒤 재조회 — 첫 조회 + 재조회 2회.
    await waitFor(() => expect(serveStatus).toHaveBeenCalledTimes(2));
  });

  it("켜기 버튼이 serveEnable을 부르고 상태를 다시 읽는다", async () => {
    hydrate();
    render(<WebRemoteSection />);

    fireEvent.click(await screen.findByRole("button", { name: "HTTPS 켜기" }));
    await waitFor(() => expect(serveEnable).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(serveStatus).toHaveBeenCalledTimes(2));
  });

  it("포트 충돌이면 켜기를 막고 점유 중인 업스트림을 알린다", async () => {
    serveStatus.mockResolvedValue(
      serve({ conflict: true, upstream: "http://127.0.0.1:4173" }),
    );
    hydrate();
    render(<WebRemoteSection />);

    const on = await screen.findByRole("button", { name: "HTTPS 켜기" });
    expect((on as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/http:\/\/127\.0\.0\.1:4173/)).toBeTruthy();
    fireEvent.click(on);
    expect(serveEnable).not.toHaveBeenCalled();
  });

  it("충돌이면 직접 정리할 명령을 복사할 수 있다", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.assign(navigator, { clipboard: { writeText } });
    serveStatus.mockResolvedValue(serve({ conflict: true, upstream: "http://127.0.0.1:4173" }));
    hydrate();
    render(<WebRemoteSection />);

    fireEvent.click(await screen.findByRole("button", { name: "정리 명령 복사" }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith("tailscale serve --https=47443 off"),
    );
  });

  it("켜기 실패 사유를 화면에 남긴다", async () => {
    serveEnable.mockRejectedValueOnce("error: invalid port");
    hydrate();
    render(<WebRemoteSection />);

    fireEvent.click(await screen.findByRole("button", { name: "HTTPS 켜기" }));
    await screen.findByText(/invalid port/);
  });
});

describe("WebRemoteSection · 접속 주소 복사", () => {
  it("http 주소 복사 버튼이 전체 URL을 clipboard에 쓰고 '복사됨'으로 바뀐다", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.assign(navigator, { clipboard: { writeText } });
    hydrate();
    render(<WebRemoteSection />);

    const copy = await screen.findByRole("button", { name: "접속 주소 복사" });
    fireEvent.click(copy);
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith("http://100.88.236.3:47800/web/"),
    );
    await waitFor(() => expect(copy.textContent).toBe("복사됨"));
  });

  it("serve가 켜져 있으면 https 주소도 복사할 수 있다", async () => {
    const writeText = vi.fn(() => Promise.resolve());
    Object.assign(navigator, { clipboard: { writeText } });
    serveStatus.mockResolvedValue(serve({ registered: true }));
    hydrate();
    render(<WebRemoteSection />);

    fireEvent.click(await screen.findByRole("button", { name: "HTTPS 주소 복사" }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith("https://zm4mini.tailc90d0d.ts.net:47443/web/"),
    );
  });

  it("주소를 아직 모르면 복사 버튼을 내놓지 않는다", async () => {
    hostStatus.mockResolvedValue(host({ addressHint: null, running: false }));
    hydrate();
    render(<WebRemoteSection />);

    await waitFor(() => expect(hostStatus).toHaveBeenCalled());
    expect(screen.queryByRole("button", { name: "접속 주소 복사" })).toBeNull();
  });
});
