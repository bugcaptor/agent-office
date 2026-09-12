// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { IdePersonaContext } from "@shared/types";

const mocks = vi.hoisted(() => ({
  get: vi.fn<(...args: unknown[]) => Promise<IdePersonaContext | null>>(),
  prepare: vi.fn<(...args: unknown[]) => Promise<IdePersonaContext>>(),
  save: vi.fn().mockResolvedValue(undefined),
  clipboard: vi.fn().mockResolvedValue(undefined),
  remote: false,
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: { getIdePersona: mocks.get, prepareIdePersona: mocks.prepare } }));
vi.mock("../../store/persist", () => ({ flushPersistence: () => mocks.save() }));
vi.mock("../../shared/remoteWindow", () => ({ isRemoteWindow: () => mocks.remote }));
import { useAppStore } from "../../store/appStore";
import { IdePersonaPanel } from "../IdePersonaPanel";
import { i18n } from "../../i18n";

const initialState = useAppStore.getState();
const prompt = '차분하게 "안녕"이라고 말해요 <&>';
let context: IdePersonaContext;
const label = (key: string) => i18n.t(`persona.${key}`, { ns: "terminal" });
const button = (key: string) => screen.getByRole("button", { name: label(key) }) as HTMLButtonElement;
async function mounted() { render(<IdePersonaPanel agentId="a1" />); await screen.findByRole("region", { name: label("title") }); }

beforeEach(() => {
  useAppStore.setState(initialState, true);
  useAppStore.getState().addAgent({ id: "a1", name: "나비", role: "개발", seed: "a", createdAt: 1, deskIndex: 0, cwd: "/work", personalityPrompt: prompt }, { startSession: false });
  context = { sessionId: "session-1", sourceSessionId: "original-1", provider: "claude", cwd: "/work", personalityPrompt: prompt,
    style: { name: "Agent Office voice", path: "/config/output-styles/voice.md", content: "---\nkeep-coding-instructions: true\n---\n" + prompt, exists: false } };
  mocks.get.mockReset(); mocks.get.mockImplementation(async () => structuredClone(context));
  mocks.prepare.mockReset(); mocks.prepare.mockImplementation(async () => { context.style!.exists = true; return structuredClone(context); });
  mocks.save.mockClear(); mocks.clipboard.mockReset(); mocks.clipboard.mockResolvedValue(undefined); mocks.remote = false;
  Object.defineProperty(navigator, "clipboard", { value: { writeText: mocks.clipboard }, configurable: true });
});
afterEach(cleanup);

describe("IdePersonaPanel", () => {
  it("separates creating the style from a user's explicit response check", async () => {
    await mounted();
    expect(button("confirm").disabled).toBe(true);
    expect(screen.getByText(label("unverified"))).toBeTruthy();
    expect(screen.getByText(/original-1/)).toBeTruthy();
    fireEvent.click(button("createStyle"));
    await waitFor(() => expect(button("confirm").disabled).toBe(false));
    expect(mocks.prepare).toHaveBeenCalledWith({ agentId: "a1", sessionId: "session-1", personalityPrompt: prompt });
    expect(mocks.save.mock.invocationCallOrder[1]).toBeLessThan(mocks.prepare.mock.invocationCallOrder[0]);
    expect(screen.queryByText(label("confirmed"))).toBeNull();
    fireEvent.click(button("confirm"));
    await screen.findByText(label("confirmed"));
    expect(screen.getByText(label("verificationNote"))).toBeTruthy();
  });

  it("a matching existing file is ready, never automatically confirmed", async () => {
    context.style!.exists = true;
    await mounted();
    expect(button("confirm").disabled).toBe(false);
    expect(screen.getByText(label("prepareDone"))).toBeTruthy();
    expect(screen.queryByText(label("confirmed"))).toBeNull();
    expect(mocks.prepare).not.toHaveBeenCalled();
  });

  it("Codex copies a normal user request without creating a style or sending input", async () => {
    context.provider = "codex"; context.style = null;
    await mounted();
    fireEvent.click(button("copy"));
    await waitFor(() => expect(button("confirm").disabled).toBe(false));
    expect(mocks.clipboard.mock.calls[0][0]).toContain(prompt);
    expect(mocks.prepare).not.toHaveBeenCalled();
    expect(screen.getByText(label("copied"))).toBeTruthy();
    expect(screen.queryByText(label("confirmed"))).toBeNull();
  });

  it("shows clipboard failures without enabling confirmation", async () => {
    context.provider = "codex"; context.style = null;
    mocks.clipboard.mockRejectedValueOnce(new Error("clipboard denied"));
    await mounted(); fireEvent.click(button("copy"));
    await screen.findByRole("alert");
    expect(button("confirm").disabled).toBe(true);
  });

  it("starts copying during the click but waits for connection validation before readiness", async () => {
    context.provider = "codex"; context.style = null;
    await mounted();
    let resolve!: (value: IdePersonaContext | null) => void;
    mocks.get.mockImplementationOnce(() => new Promise((r) => { resolve = r; }));
    fireEvent.click(button("copy"));
    expect(mocks.clipboard).toHaveBeenCalledOnce();
    expect(button("confirm").disabled).toBe(true);
    await act(async () => resolve(null));
    expect((await screen.findByRole("alert")).textContent).toBe(label("stale"));
    expect(button("confirm").disabled).toBe(true);
  });

  it("exposes load errors and permits retry without mutating provider settings", async () => {
    mocks.get.mockRejectedValueOnce(new Error("ide-persona-style-conflict"));
    render(<IdePersonaPanel agentId="a1" />);
    expect((await screen.findByRole("alert")).textContent).toBe(label("conflict"));
    fireEvent.click(button("retry"));
    await screen.findByRole("region", { name: label("title") });
    expect(mocks.prepare).not.toHaveBeenCalled();
  });

  it("rejects a removed style or changed logical connection at confirmation", async () => {
    context.style!.exists = true;
    await mounted(); context.sessionId = "session-2";
    fireEvent.click(button("confirm"));
    expect((await screen.findByRole("alert")).textContent).toBe(label("stale"));
    expect(screen.queryByText(label("confirmed"))).toBeNull();
  });

  it.each(["personalityPrompt", "name", "cwd"] as const)("invalidates confirmation when %s changes", async (field) => {
    context.style!.exists = true;
    await mounted(); fireEvent.click(button("confirm")); await screen.findByText(label("confirmed"));
    if (field === "personalityPrompt") context.personalityPrompt = "새로운 말투";
    if (field === "cwd") context.cwd = "/new";
    act(() => useAppStore.getState().updateAgent("a1", { [field]: field === "personalityPrompt" ? context.personalityPrompt : field === "cwd" ? "/new" : "새 이름" }));
    await screen.findByRole("region", { name: label("title") });
    expect(screen.queryByText(label("confirmed"))).toBeNull();
  });

  it("invalidates confirmation on a new backend session ID even if running stays running", async () => {
    context.style!.exists = true;
    await mounted(); fireEvent.click(button("confirm")); await screen.findByText(label("confirmed"));
    context.sessionId = "session-2";
    act(() => useAppStore.getState().noteUsageSession("a1", "session-2"));
    await screen.findByRole("region", { name: label("title") });
    expect(screen.queryByText(label("confirmed"))).toBeNull();
  });

  it("discards a late prepare result after a profile change", async () => {
    let resolve!: (value: IdePersonaContext) => void;
    mocks.prepare.mockImplementationOnce(() => new Promise((r) => { resolve = r; }));
    await mounted(); fireEvent.click(button("createStyle"));
    await waitFor(() => expect(mocks.prepare).toHaveBeenCalled());
    const old = structuredClone(context); old.style!.exists = true;
    context.personalityPrompt = "new voice";
    act(() => useAppStore.getState().updateAgent("a1", { personalityPrompt: "new voice" }));
    await screen.findByRole("region", { name: label("title") });
    await act(async () => resolve(old));
    expect(button("confirm").disabled).toBe(true);
    expect(button("createStyle").disabled).toBe(false);
  });

  it("discards a late clipboard result after a profile change", async () => {
    context.provider = "codex"; context.style = null;
    let resolve!: () => void;
    mocks.clipboard.mockImplementationOnce(() => new Promise<void>((r) => { resolve = r; }));
    await mounted(); fireEvent.click(button("copy"));
    await waitFor(() => expect(mocks.clipboard).toHaveBeenCalled());
    context.personalityPrompt = "new voice";
    act(() => useAppStore.getState().updateAgent("a1", { personalityPrompt: "new voice" }));
    await screen.findByRole("region", { name: label("title") });
    await act(async () => resolve());
    expect(button("confirm").disabled).toBe(true);
  });

  it("offers profile editing for an empty voice and keeps detach guidance visible when collapsed", async () => {
    context.personalityPrompt = "";
    act(() => useAppStore.getState().updateAgent("a1", { personalityPrompt: "" }));
    await mounted(); fireEvent.click(button("editProfile"));
    expect(useAppStore.getState().modal).toEqual({ kind: "profile-edit", agentId: "a1" });
    fireEvent.click(button("title"));
    expect(screen.getByText(label("detachNote"))).toBeTruthy();
  });

  it("hides unsupported external connections and does not query on remote windows", async () => {
    mocks.get.mockResolvedValueOnce(null);
    const view = render(<IdePersonaPanel agentId="a1" />);
    await waitFor(() => expect(screen.queryByText(label("loading"))).toBeNull());
    expect(screen.queryByRole("region")).toBeNull();
    view.unmount(); mocks.get.mockClear(); mocks.save.mockClear(); mocks.remote = true;
    render(<IdePersonaPanel agentId="a1" />);
    expect(mocks.get).not.toHaveBeenCalled(); expect(mocks.save).not.toHaveBeenCalled();
  });
});
