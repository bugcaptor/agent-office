// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const remote = vi.hoisted(() => ({
  load: vi.fn(), connect: vi.fn(), snapshot: vi.fn(), disconnect: vi.fn(), boot: vi.fn(),
}));
vi.mock("../../App", () => ({ default: () => <div>Connected office</div> }));
vi.mock("../../remoteBootstrap", () => ({ bootRemoteApp: remote.boot }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock("../../ipc/tauriApi", () => ({ setTauriApiDelegate: vi.fn(), setTauriApiFailClosed: vi.fn() }));
vi.mock("../../ipc/remoteApi", () => ({
  loadRemoteConnection: remote.load,
  connectRemote: remote.connect,
  loadRemoteSnapshot: remote.snapshot,
  disconnectRemote: remote.disconnect,
  createRemoteApi: vi.fn(() => ({})),
  acceptRemoteSnapshot: vi.fn(), currentRemoteState: vi.fn(), remoteStateKey: vi.fn(),
}));

import RemoteApp from "../RemoteApp";

beforeEach(() => {
  vi.clearAllMocks();
  remote.load.mockResolvedValue(null);
  remote.connect.mockResolvedValue({ hostName: "My server", permission: "input" });
  remote.snapshot.mockResolvedValue({ state: { agents: [], version: 1 }, sessions: [], settings: {}, revision: "r1" });
  remote.boot.mockResolvedValue(() => {});
});
afterEach(cleanup);

describe("remembered remote connection", () => {
  it("prefills credentials on each window opening and connects without retyping", async () => {
    remote.load.mockResolvedValue({ url: "http://office.test:47373", token: "saved-secret" });
    const first = render(<RemoteApp />);
    await waitFor(() => expect((screen.getByLabelText("접속 토큰") as HTMLInputElement).value).toBe("saved-secret"));
    expect((screen.getByLabelText("서버 주소") as HTMLInputElement).value).toBe("http://office.test:47373");
    expect((screen.getByLabelText("접속 토큰") as HTMLInputElement).type).toBe("password");
    expect(remote.connect).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    await screen.findByText("Connected office");
    expect(remote.connect).toHaveBeenCalledWith("http://office.test:47373", "saved-secret");
    first.unmount();
    render(<RemoteApp />);
    await waitFor(() => expect((screen.getByLabelText("접속 토큰") as HTMLInputElement).value).toBe("saved-secret"));
    expect(remote.load).toHaveBeenCalledTimes(2);
  });

  it("does not overwrite newly typed credentials when loading completes late", async () => {
    let resolve!: (value: { url: string; token: string }) => void;
    remote.load.mockReturnValue(new Promise((done) => { resolve = done; }));
    render(<RemoteApp />);
    fireEvent.change(screen.getByLabelText("서버 주소"), { target: { value: "http://new.test" } });
    fireEvent.change(screen.getByLabelText("접속 토큰"), { target: { value: "new-secret" } });
    await act(async () => resolve({ url: "http://old.test", token: "old-secret" }));
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    await screen.findByText("Connected office");
    expect(remote.connect).toHaveBeenCalledWith("http://new.test", "new-secret");
  });

  it("allows manual connection after a saved-file failure without exposing its contents", async () => {
    remote.load.mockRejectedValue(new Error("corrupt file includes secret-token"));
    render(<RemoteApp />);
    await screen.findByRole("alert");
    expect(document.body.textContent).not.toContain("secret-token");
    fireEvent.change(screen.getByLabelText("서버 주소"), { target: { value: "http://new.test" } });
    fireEvent.change(screen.getByLabelText("접속 토큰"), { target: { value: "replacement" } });
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    await screen.findByText("Connected office");
  });

  it("keeps credentials editable after a failed connection and shows a safe error", async () => {
    remote.load.mockResolvedValue({ url: "http://office.test", token: "saved-secret" });
    remote.connect.mockRejectedValueOnce(new Error("failed using saved-secret"));
    render(<RemoteApp />);
    await waitFor(() => expect((screen.getByLabelText("접속 토큰") as HTMLInputElement).value).toBe("saved-secret"));
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    await screen.findByRole("alert");
    expect(document.body.textContent).not.toContain("saved-secret");
    expect((screen.getByLabelText("접속 토큰") as HTMLInputElement).value).toBe("saved-secret");
    expect(screen.getByRole("button", { name: "연결" }).hasAttribute("disabled")).toBe(false);
  });

  it("explains a local save failure instead of showing the office as successfully connected", async () => {
    remote.load.mockResolvedValue({ url: "http://office.test", token: "saved-secret" });
    remote.connect.mockRejectedValueOnce("remote-connection-save-failed");
    render(<RemoteApp />);
    await waitFor(() => expect((screen.getByLabelText("접속 토큰") as HTMLInputElement).value).toBe("saved-secret"));
    fireEvent.click(screen.getByRole("button", { name: "연결" }));
    expect((await screen.findByRole("alert")).textContent).toContain("저장하지 못했습니다");
    expect(remote.boot).not.toHaveBeenCalled();
  });
});
