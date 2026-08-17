// vscode-ext/src/poller.ts
//
// 주기 폴링. 틱마다 `/v1/list` → 각 캐릭터의 `/v1/notifications`를 모아 스냅샷
// 하나를 방출한다. 알림 push 채널이 없으므로 폴링이지만, 창이 백그라운드거나
// 앱이 꺼진(unreachable) 동안에는 간격을 10초로 늘려 예산을 아낀다.
//
// control 연결이 실패하면 `<app_data>/profiles.json`(읽기 전용)으로 캐릭터
// 목록을 만들어 로그 열람만은 계속 가능하게 한다.
//
// vscode 모듈에 의존하지 않는다 — 포커스 상태·설정·타이머는 전부 주입받는다.

import {
  type ConnectionStatus,
  type ControlResponse,
  type ListEntry,
  type NotificationEntry,
  offlineMessage,
} from "./controlClient";

/** 창이 포커스를 잃었거나 앱이 꺼져 있을 때의 폴링 간격. */
export const BACKGROUND_INTERVAL_MS = 10_000;
/** 설정이 아무리 작아도 이보다 자주 돌지 않는다. */
export const MIN_INTERVAL_MS = 500;

export interface AgentEntry {
  agentId: string;
  name: string;
  role: string;
  cwd?: string;
  /** 실행 중 세션의 상태("running"/"starting"/…). 없으면 세션 없음. */
  state?: string;
  /** 실행 중 세션 id. 바뀌면 새 로그 파일이 생겼다는 뜻이다. */
  sessionId?: string;
  notifications: NotificationEntry[];
}

export interface Snapshot {
  status: ConnectionStatus;
  /** 미연결·거절 사유 안내(status가 ok면 undefined). */
  message?: string;
  entries: AgentEntry[];
  appDataDir?: string;
  /** 세션 로그 루트가 없음(sessionLogEnabled OFF이거나 앱을 쓴 적 없음). */
  logRootMissing: boolean;
  at: number;
}

/** Poller가 쓰는 control 클라이언트의 최소 표면. */
export interface ControlLike {
  list(): Promise<ControlResponse<ListEntry[]>>;
  notifications(agentId: string): Promise<ControlResponse<NotificationEntry[]>>;
}

export interface Timers {
  setTimeout(handler: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

export interface PollerDeps {
  /** 매 틱 다시 확인한다(설정이 바뀌었을 수 있다). */
  resolveAppDataDir(): string | undefined;
  createClient(appDataDir: string | undefined): ControlLike;
  /** `<app_data>/profiles.json` 원문. 없으면 undefined. */
  readProfiles(appDataDir: string): Promise<string | undefined>;
  /** 세션 로그 루트 존재 확인. */
  logRootExists(appDataDir: string): Promise<boolean>;
  pollIntervalMs(): number;
  isFocused(): boolean;
  now?(): number;
  timers?: Timers;
}

export interface ProfileLite {
  id: string;
  name: string;
  role: string;
  cwd?: string;
}

/**
 * profiles.json 폴백 파서. `version`이 1이 아니거나 형식이 깨졌으면 빈 목록
 * (앱의 ProfileStore와 같은 태도 — 버전 불일치는 조용히 빈 상태).
 */
export function parseProfilesJson(text: string | undefined): ProfileLite[] {
  if (!text) return [];
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return [];
  }
  const file = parsed as { version?: unknown; agents?: unknown };
  if (!file || file.version !== 1 || !Array.isArray(file.agents)) return [];
  const out: ProfileLite[] = [];
  for (const raw of file.agents) {
    const a = raw as { id?: unknown; name?: unknown; role?: unknown; cwd?: unknown };
    if (!a || typeof a.id !== "string" || a.id.length === 0) continue;
    out.push({
      id: a.id,
      name: typeof a.name === "string" && a.name ? a.name : a.id,
      role: typeof a.role === "string" ? a.role : "",
      cwd: typeof a.cwd === "string" && a.cwd ? a.cwd : undefined,
    });
  }
  return out;
}

export class Poller {
  private readonly listeners = new Set<(s: Snapshot) => void>();
  private readonly timers: Timers;
  private readonly now: () => number;
  private handle: unknown;
  private running = false;
  private ticking = false;
  private lastStatus: ConnectionStatus = "ok";
  private snapshot: Snapshot | undefined;

  constructor(private readonly deps: PollerDeps) {
    this.timers = deps.timers ?? {
      setTimeout: (h, ms) => setTimeout(h, ms),
      clearTimeout: (x) => clearTimeout(x as ReturnType<typeof setTimeout>),
    };
    this.now = deps.now ?? (() => Date.now());
  }

  get latest(): Snapshot | undefined {
    return this.snapshot;
  }

  onSnapshot(listener: (s: Snapshot) => void): { dispose(): void } {
    this.listeners.add(listener);
    return { dispose: () => this.listeners.delete(listener) };
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    void this.tick();
  }

  stop(): void {
    this.running = false;
    if (this.handle !== undefined) {
      this.timers.clearTimeout(this.handle);
      this.handle = undefined;
    }
  }

  dispose(): void {
    this.stop();
    this.listeners.clear();
  }

  /** 즉시 한 틱 돌리고 다음 예약을 다시 잡는다(새로고침 명령용). */
  async refresh(): Promise<void> {
    if (this.handle !== undefined) {
      this.timers.clearTimeout(this.handle);
      this.handle = undefined;
    }
    await this.tick();
  }

  /** 다음 틱까지의 지연. unreachable이면 백오프, 백그라운드면 느리게. */
  nextDelayMs(): number {
    const configured = Math.max(MIN_INTERVAL_MS, Math.floor(this.deps.pollIntervalMs()));
    if (this.lastStatus === "unreachable") return Math.max(BACKGROUND_INTERVAL_MS, configured);
    if (!this.deps.isFocused()) return Math.max(BACKGROUND_INTERVAL_MS, configured);
    return configured;
  }

  private schedule(): void {
    if (!this.running) return;
    this.handle = this.timers.setTimeout(() => {
      this.handle = undefined;
      void this.tick();
    }, this.nextDelayMs());
  }

  private async tick(): Promise<void> {
    // 앞 틱이 아직 돌고 있으면(느린 응답) 겹쳐 쏘지 않는다.
    if (this.ticking) return;
    this.ticking = true;
    try {
      const snapshot = await this.collect();
      this.lastStatus = snapshot.status;
      this.snapshot = snapshot;
      for (const listener of [...this.listeners]) listener(snapshot);
    } finally {
      this.ticking = false;
      this.schedule();
    }
  }

  private async collect(): Promise<Snapshot> {
    const appDataDir = this.deps.resolveAppDataDir();
    const logRootMissing = appDataDir ? !(await this.deps.logRootExists(appDataDir)) : true;
    const base = { appDataDir, logRootMissing, at: this.now() };

    const client = this.deps.createClient(appDataDir);
    const listed = await client.list();

    if (listed.status === "ok") {
      const rows = Array.isArray(listed.data) ? listed.data : [];
      const entries = await Promise.all(
        rows.map(async (row): Promise<AgentEntry> => {
          const res = await client.notifications(row.agentId);
          return {
            agentId: row.agentId,
            name: row.name || row.agentId,
            role: row.role ?? "",
            cwd: row.cwd,
            state: row.state,
            sessionId: row.sessionId,
            notifications: res.status === "ok" && Array.isArray(res.data) ? res.data : [],
          };
        }),
      );
      return { ...base, status: "ok", entries };
    }

    // 연결 실패 — profiles.json으로 목록만 세워 로그 열람을 살린다.
    const profiles = appDataDir
      ? parseProfilesJson(await this.deps.readProfiles(appDataDir))
      : [];
    return {
      ...base,
      status: listed.status,
      message:
        listed.status === "error" ? listed.message : offlineMessage(listed.status),
      entries: profiles.map((p) => ({
        agentId: p.id,
        name: p.name,
        role: p.role,
        cwd: p.cwd,
        notifications: [],
      })),
    };
  }
}
