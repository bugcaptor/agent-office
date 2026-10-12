import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStateEvent, SessionSummary } from "@shared/types";
const { load, append, callbacks } = vi.hoisted(() => ({ load: vi.fn(), append: vi.fn(), callbacks: new Set<(e: SessionStateEvent) => void>() }));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: {
  loadSessionSummaries: load, appendSessionSummaries: append,
  onSessionState: (cb: (e: SessionStateEvent) => void) => { callbacks.add(cb); return () => callbacks.delete(cb); },
} }));
import { useAppStore } from "../../store/appStore";
import { useSessionSummaryStore, installSessionSummaryRecorder, flushSessionSummaries } from "../sessionSummaryStore";
const initial = useAppStore.getState();
const now = 1_800_000_000_000;
let off: (() => void) | undefined;
function emit(state: SessionStateEvent["state"], at = now, sessionId = "s1") { callbacks.forEach((cb) => cb({state,at,sessionId,agentId:"a1"})); }
beforeEach(() => {
  vi.useFakeTimers(); vi.setSystemTime(now);
  load.mockReset().mockResolvedValue([]); append.mockReset().mockResolvedValue(undefined);
  useAppStore.setState({ ...initial, agents: { a1: { id:"a1",name:"Before",role:"",seed:"x",createdAt:now,deskIndex:0,cwd:"/work" } } }, true);
  useSessionSummaryStore.setState({summaries:{},liveIds:[],error:false});
});
afterEach(async () => { await flushSessionSummaries(); off?.(); off=undefined; await Promise.resolve(); vi.clearAllTimers(); vi.useRealTimers(); });
describe("상시 세션 요약 수집", () => {
  it("패널·일기·요약기가 꺼져 있어도 시작 스냅샷과 프롬프트를 저장한다", async () => {
    off = await installSessionSummaryRecorder(); emit("starting"); emit("running", now+1);
    useAppStore.setState({taskLabels:{a1:{sessionId:"s1",latestPromptText:"🙂".repeat(400),latestPromptAt:now+5}}});
    await vi.advanceTimersByTimeAsync(1000);
    const saved: SessionSummary = append.mock.calls[0][0][0];
    expect(saved).toMatchObject({agentName:"Before",cwd:"/work",startedAt:now});
    expect(Array.from(saved.prompts[0].text)).toHaveLength(300);
    expect(useSessionSummaryStore.getState().liveIds).toEqual(["s1"]);
  });
  it("프로필 변경에도 시작 이름을 보존하고 목표·종료를 합치며 disposed는 종료를 늦추지 않는다", async () => {
    off = await installSessionSummaryRecorder(); emit("running");
    useAppStore.setState({agents:{a1:{...useAppStore.getState().agents.a1,name:"After"}},taskLabels:{a1:{sessionId:"s1",goal:"Goal",latestPromptAt:now+5,latestPromptText:"First"}}});
    emit("exited",now+20); emit("disposed",now+40);
    await flushSessionSummaries();
    expect(append.mock.calls[0][0][0]).toMatchObject({agentName:"Before",goal:"Goal",endedAt:now+20,prompts:[{at:now+5,text:"First"}]});
    expect(useSessionSummaryStore.getState().liveIds).toEqual([]);
  });
  it("복원한 프롬프트를 중복 저장하지 않고 새 세션에서는 같은 시각도 기록한다", async () => {
    load.mockResolvedValueOnce([{sessionId:"s1",agentId:"a1",agentName:"Before",startedAt:now,prompts:[{at:now,text:"First"}]}]);
    off = await installSessionSummaryRecorder();
    useAppStore.setState({taskLabels:{a1:{sessionId:"s1",latestPromptAt:now,latestPromptText:"First"}}});
    await flushSessionSummaries(); expect(append).not.toHaveBeenCalled();
    useAppStore.setState({taskLabels:{a1:{sessionId:"s2",latestPromptAt:now,latestPromptText:"Second"}}});
    await flushSessionSummaries(); expect(append.mock.calls[0][0][0].sessionId).toBe("s2");
  });
  it("프롬프트를 50개로 제한하고 종료 flush에서 마지막 편집을 저장한다", async () => {
    off = await installSessionSummaryRecorder(); emit("running");
    for(let n=0;n<55;n++) useAppStore.setState({taskLabels:{a1:{sessionId:"s1",latestPromptAt:now+n,latestPromptText:`Prompt ${n}`}}});
    await flushSessionSummaries();
    expect(append.mock.calls[0][0][0].prompts).toHaveLength(50);
    expect(append.mock.calls[0][0][0].prompts[0].text).toBe("Prompt 0");
  });
  it("쓰기 실패는 버리지 않고 자동으로 재시도한다", async () => {
    off = await installSessionSummaryRecorder(); append.mockRejectedValueOnce(new Error("disk")); emit("running");
    await flushSessionSummaries(); expect(useSessionSummaryStore.getState().error).toBe(true);
    await vi.advanceTimersByTimeAsync(5000); expect(append).toHaveBeenCalledTimes(2); expect(useSessionSummaryStore.getState().error).toBe(false);
  });
});
