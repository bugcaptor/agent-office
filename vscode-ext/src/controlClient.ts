// vscode-ext/src/controlClient.ts
//
// 앱의 control 서버(docs/cli-control-design.md) 클라이언트. `ctl`과 동일하게
// `<app_data>/control-port`·`control-token`을 읽어
// `POST http://127.0.0.1:<port>/v1/<command>` + `x-agent-office-token` 헤더로
// 요청하고, 응답 봉투 `{ok:true,data}` / `{ok:false,error}`를 푼다.
//
// 토큰은 파일에서 읽어 이 모듈 안에서만 쓰고 어디에도 남기지 않는다(설정·로그·
// 에러 메시지 금지). `cliEnabled`를 켜려 들지도 않는다 — 앱의 2단계 옵트인 존중.
//
// vscode 모듈에 의존하지 않는다(fetch/파일읽기를 주입해 테스트한다).

import * as path from "node:path";

export const TOKEN_HEADER = "x-agent-office-token";
export const PORT_FILE = "control-port";
export const TOKEN_FILE = "control-token";

/** 기본 요청 타임아웃. 로컬 루프백이라 짧게 잡고 실패는 다음 틱에 맡긴다. */
const DEFAULT_TIMEOUT_MS = 2000;

/**
 * 연결 상태. `ok`/`error`만 서버에 실제로 닿은 경우다.
 * - `no-app-data`: app_data 경로 자체를 못 찾음
 * - `no-port`: 포트 파일 없음(앱 꺼짐 또는 설정 cliEnabled OFF)
 * - `no-token`: 토큰 파일 없음(미승인)
 * - `unauthorized`: HTTP 401(토큰 무효/승인 취소)
 * - `unreachable`: connect 실패·타임아웃(앱 꺼짐, 스테일 포트 파일)
 */
export type ConnectionStatus =
  | "ok"
  | "error"
  | "no-app-data"
  | "no-port"
  | "no-token"
  | "unauthorized"
  | "unreachable";

/** 서버에 닿지 못한(또는 인증 실패) 상태들 — 온보딩 안내가 필요한 갈래. */
export type OfflineStatus = Exclude<ConnectionStatus, "ok" | "error">;

export type ControlResponse<T> =
  | { status: "ok"; data: T }
  /** 서버가 `ok:false`로 거절 — 연결 자체는 정상이다. */
  | { status: "error"; message: string }
  | { status: OfflineStatus };

/** `/v1/ping` 응답. */
export interface PingResult {
  appVersion: string;
  agentCount: number;
  runningCount: number;
}

/** `/v1/list` 항목 — 프로필 + 실행 중 세션 상태의 병합(state 없으면 세션 없음). */
export interface ListEntry {
  agentId: string;
  name: string;
  role: string;
  cwd?: string;
  state?: string;
  sessionId?: string;
}

export type NotificationSource = "hook" | "stop" | "bell";

/** `/v1/notifications` 항목. `at`은 epoch ms. */
export interface NotificationEntry {
  id: string;
  sessionId: string;
  agentId: string;
  source: NotificationSource;
  message: string;
  dedupKey: string;
  at: number;
}

export type FetchLike = (
  url: string,
  init: { method: string; headers: Record<string, string>; body: string; signal: AbortSignal },
) => Promise<{ status: number; json(): Promise<unknown> }>;

export interface ControlClientDeps {
  /** 주입 가능한 fetch(테스트·대체 구현용). 기본값은 전역 fetch. */
  fetchFn?: FetchLike;
  /** 텍스트 파일 읽기. 없거나 읽을 수 없으면 undefined를 돌려준다. */
  readTextFile?: (filePath: string) => Promise<string | undefined>;
  timeoutMs?: number;
}

/** 기본 파일 리더 — 실패는 전부 undefined로 흡수한다(열람 경로를 막지 않는다). */
async function defaultReadTextFile(filePath: string): Promise<string | undefined> {
  try {
    const fs = await import("node:fs/promises");
    return await fs.readFile(filePath, "utf8");
  } catch {
    return undefined;
  }
}

const defaultFetch: FetchLike = (url, init) =>
  (globalThis.fetch as unknown as FetchLike)(url, init);

/** 포트 파일 본문 → 포트 번호. 형식이 깨졌으면 undefined. */
export function parsePort(text: string | undefined): number | undefined {
  if (text === undefined) return undefined;
  const n = Number.parseInt(text.trim(), 10);
  return Number.isInteger(n) && n > 0 && n < 65536 ? n : undefined;
}

export class ControlClient {
  private readonly fetchFn: FetchLike;
  private readonly readTextFile: (filePath: string) => Promise<string | undefined>;
  private readonly timeoutMs: number;

  constructor(
    /** app_data 경로. undefined면 모든 요청이 `no-app-data`가 된다. */
    private readonly appDataDir: string | undefined,
    deps: ControlClientDeps = {},
  ) {
    this.fetchFn = deps.fetchFn ?? defaultFetch;
    this.readTextFile = deps.readTextFile ?? defaultReadTextFile;
    this.timeoutMs = deps.timeoutMs ?? DEFAULT_TIMEOUT_MS;
  }

  /** 라우트 하나를 호출한다. `command`는 "ping"처럼 `/v1/` 뒤 부분. */
  async request<T>(command: string, body: unknown = {}): Promise<ControlResponse<T>> {
    if (!this.appDataDir) return { status: "no-app-data" };

    const port = parsePort(await this.readTextFile(path.join(this.appDataDir, PORT_FILE)));
    if (port === undefined) return { status: "no-port" };

    const token = (await this.readTextFile(path.join(this.appDataDir, TOKEN_FILE)))?.trim();
    if (!token) return { status: "no-token" };

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), this.timeoutMs);
    let status: number;
    let payload: unknown;
    try {
      const res = await this.fetchFn(`http://127.0.0.1:${port}/v1/${command}`, {
        method: "POST",
        headers: { "content-type": "application/json", [TOKEN_HEADER]: token },
        body: JSON.stringify(body ?? {}),
        signal: controller.signal,
      });
      status = res.status;
      // 401 본문은 봉투가 아닐 수 있으니 먼저 코드를 본다.
      payload = status === 401 ? undefined : await res.json().catch(() => undefined);
    } catch {
      // connect 실패·타임아웃·중단 — 토큰이 섞일 수 있으므로 원인 문자열은 버린다.
      return { status: "unreachable" };
    } finally {
      clearTimeout(timer);
    }

    if (status === 401) return { status: "unauthorized" };
    const envelope = payload as { ok?: unknown; data?: unknown; error?: unknown } | undefined;
    if (envelope && envelope.ok === true) return { status: "ok", data: envelope.data as T };
    if (envelope && typeof envelope.error === "string") {
      return { status: "error", message: envelope.error };
    }
    return { status: "error", message: `예기치 않은 응답 (HTTP ${status})` };
  }

  ping(): Promise<ControlResponse<PingResult>> {
    return this.request<PingResult>("ping");
  }

  list(): Promise<ControlResponse<ListEntry[]>> {
    return this.request<ListEntry[]>("list");
  }

  notifications(agentId: string): Promise<ControlResponse<NotificationEntry[]>> {
    return this.request<NotificationEntry[]>("notifications", { agentId });
  }

  /** ids 생략 = 그 캐릭터의 알림 전체 클리어. */
  clear(agentId: string, ids?: string[]): Promise<ControlResponse<unknown>> {
    return this.request("clear", ids && ids.length > 0 ? { agentId, ids } : { agentId });
  }
}

/** 온보딩 안내 문구 — 미연결 사유별로 사용자가 할 일을 알려 준다. */
export function offlineMessage(status: OfflineStatus): string {
  switch (status) {
    case "no-app-data":
      return "앱 데이터 폴더를 찾지 못했습니다 — 설정 agentOffice.appDataDir을 지정하세요.";
    case "no-port":
      return "앱이 꺼져 있거나 설정에서 CLI 제어가 꺼져 있습니다. 앱 설정 → CLI 제어 켜기. (로그 파일 열람은 계속 가능)";
    case "no-token":
      return "앱에서 CLI 제어 승인이 필요합니다. 앱 설정 → CLI 제어 → 승인.";
    case "unauthorized":
      return "승인이 취소되었습니다 — 앱에서 다시 승인하세요.";
    case "unreachable":
      return "앱이 꺼져 있습니다(포트 파일은 남아 있음). 로그 파일 열람은 계속 가능.";
  }
}
