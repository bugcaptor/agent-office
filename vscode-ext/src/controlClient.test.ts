import * as path from "node:path";
import { describe, expect, it, vi } from "vitest";

import {
  ControlClient,
  TOKEN_HEADER,
  offlineMessage,
  parsePort,
  type FetchLike,
} from "./controlClient";

const APP_DATA = "/tmp/app-data";

/** 파일 이름 → 내용 맵으로 포트/토큰 파일을 흉내낸다. */
function files(map: Record<string, string | undefined>) {
  return async (filePath: string) => map[path.basename(filePath)];
}

const OK_FILES = files({ "control-port": "51234\n", "control-token": "tok-abc\n" });

function fetchOk(status: number, body: unknown): FetchLike {
  return vi.fn(async () => ({ status, json: async () => body })) as unknown as FetchLike;
}

describe("parsePort", () => {
  it("공백을 털고 숫자만 받는다", () => {
    expect(parsePort("51234\n")).toBe(51234);
    expect(parsePort("  80 ")).toBe(80);
    expect(parsePort("0")).toBeUndefined();
    expect(parsePort("70000")).toBeUndefined();
    expect(parsePort("abc")).toBeUndefined();
    expect(parsePort("")).toBeUndefined();
    expect(parsePort(undefined)).toBeUndefined();
  });
});

describe("ControlClient 상태 판별", () => {
  it("app_data가 없으면 no-app-data", async () => {
    const fetchFn = fetchOk(200, { ok: true, data: {} });
    const client = new ControlClient(undefined, { fetchFn, readTextFile: OK_FILES });
    expect(await client.ping()).toEqual({ status: "no-app-data" });
    expect(fetchFn).not.toHaveBeenCalled();
  });

  it("포트 파일이 없으면 no-port (앱 꺼짐 또는 cliEnabled OFF)", async () => {
    const client = new ControlClient(APP_DATA, {
      fetchFn: fetchOk(200, { ok: true, data: {} }),
      readTextFile: files({ "control-token": "tok" }),
    });
    expect(await client.ping()).toEqual({ status: "no-port" });
  });

  it("토큰 파일이 없으면 no-token (미승인)", async () => {
    const client = new ControlClient(APP_DATA, {
      fetchFn: fetchOk(200, { ok: true, data: {} }),
      readTextFile: files({ "control-port": "51234" }),
    });
    expect(await client.ping()).toEqual({ status: "no-token" });
  });

  it("401이면 unauthorized", async () => {
    const client = new ControlClient(APP_DATA, {
      fetchFn: fetchOk(401, { ok: false, error: "unauthorized" }),
      readTextFile: OK_FILES,
    });
    expect(await client.ping()).toEqual({ status: "unauthorized" });
  });

  it("connect 실패는 unreachable", async () => {
    const client = new ControlClient(APP_DATA, {
      fetchFn: (() => Promise.reject(new Error("ECONNREFUSED"))) as unknown as FetchLike,
      readTextFile: OK_FILES,
    });
    expect(await client.ping()).toEqual({ status: "unreachable" });
  });

  it("타임아웃도 unreachable", async () => {
    const client = new ControlClient(APP_DATA, {
      timeoutMs: 5,
      readTextFile: OK_FILES,
      fetchFn: ((_url: string, init: { signal: AbortSignal }) =>
        new Promise((_resolve, reject) => {
          init.signal.addEventListener("abort", () => reject(new Error("aborted")));
        })) as unknown as FetchLike,
    });
    expect(await client.ping()).toEqual({ status: "unreachable" });
  });

  it("ok:true면 data를 그대로 돌려준다", async () => {
    const client = new ControlClient(APP_DATA, {
      fetchFn: fetchOk(200, {
        ok: true,
        data: { appVersion: "0.6.2", agentCount: 6, runningCount: 1 },
      }),
      readTextFile: OK_FILES,
    });
    expect(await client.ping()).toEqual({
      status: "ok",
      data: { appVersion: "0.6.2", agentCount: 6, runningCount: 1 },
    });
  });

  it("ok:false는 연결은 됐지만 error", async () => {
    const client = new ControlClient(APP_DATA, {
      fetchFn: fetchOk(200, { ok: false, error: "알 수 없는 캐릭터" }),
      readTextFile: OK_FILES,
    });
    expect(await client.list()).toEqual({ status: "error", message: "알 수 없는 캐릭터" });
  });

  it("봉투가 아니면 error로 강등한다", async () => {
    const client = new ControlClient(APP_DATA, {
      fetchFn: fetchOk(500, "<html>"),
      readTextFile: OK_FILES,
    });
    expect(await client.list()).toEqual({
      status: "error",
      message: "예기치 않은 응답 (HTTP 500)",
    });
  });

  it("요청은 127.0.0.1 + 토큰 헤더 + JSON 본문으로 나간다", async () => {
    const fetchFn = fetchOk(200, { ok: true, data: [] });
    const client = new ControlClient(APP_DATA, { fetchFn, readTextFile: OK_FILES });
    await client.notifications("agent-1");
    expect(fetchFn).toHaveBeenCalledTimes(1);
    const [url, init] = (fetchFn as unknown as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(url).toBe("http://127.0.0.1:51234/v1/notifications");
    expect(init.method).toBe("POST");
    // 토큰은 파일에서 읽어 헤더에만 실린다(공백 제거).
    expect(init.headers[TOKEN_HEADER]).toBe("tok-abc");
    expect(JSON.parse(init.body)).toEqual({ agentId: "agent-1" });
  });

  it("clear는 ids가 있을 때만 ids를 싣는다", async () => {
    const fetchFn = fetchOk(200, { ok: true, data: null });
    const client = new ControlClient(APP_DATA, { fetchFn, readTextFile: OK_FILES });
    const calls = (fetchFn as unknown as ReturnType<typeof vi.fn>).mock.calls;
    await client.clear("a1");
    expect(JSON.parse(calls[0][1].body)).toEqual({ agentId: "a1" });
    await client.clear("a1", []);
    expect(JSON.parse(calls[1][1].body)).toEqual({ agentId: "a1" });
    await client.clear("a1", ["n1", "n2"]);
    expect(JSON.parse(calls[2][1].body)).toEqual({ agentId: "a1", ids: ["n1", "n2"] });
  });
});

describe("offlineMessage", () => {
  it("사유마다 사용자가 할 일을 알려 준다", () => {
    expect(offlineMessage("no-app-data")).toContain("agentOffice.appDataDir");
    expect(offlineMessage("no-port")).toContain("CLI 제어");
    expect(offlineMessage("no-token")).toContain("승인");
    expect(offlineMessage("unauthorized")).toContain("다시 승인");
    expect(offlineMessage("unreachable")).toContain("앱이 꺼져 있습니다");
  });
});
