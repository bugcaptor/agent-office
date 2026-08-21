// src/web/ws.ts
//
// 웹 클라이언트의 유일한 서버 연결. same-origin `ws(s)://…/webremote/v1/ws`로 붙고
// 인증은 **쿠키가 자동으로** 동반한다(브라우저 WebSocket API는 커스텀 헤더를
// 못 붙인다 — 그래서 서버가 쿠키 인증 경로를 갖는다).
//
// 이 클래스가 하는 일은 셋이다: 재접속(지수 백오프), RPC 상관(id ↔ Promise),
// 메시지 팬아웃. 상태는 갖지 않는다 — 서버 push가 유일한 진실이라는 원칙이
// 스토어가 두 벌 생기는 문제를 애초에 없앤다.

import type { HostMsg, ClientMsg } from "./protocol";

const RECONNECT_MIN_MS = 1000;
const RECONNECT_MAX_MS = 30_000;
const RPC_TIMEOUT_MS = 10_000;
const PING_EVERY_MS = 20_000;

export type ConnState = "connecting" | "open" | "closed";

interface Pending {
  resolve: (value: unknown) => void;
  reject: (err: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

export class WebRemoteSocket {
  private ws: WebSocket | null = null;
  private backoff = RECONNECT_MIN_MS;
  private nextId = 1;
  private pending = new Map<number, Pending>();
  private listeners = new Set<(msg: HostMsg) => void>();
  private stateListeners = new Set<(s: ConnState) => void>();
  private pingTimer: ReturnType<typeof setInterval> | null = null;
  private closed = false;
  private queue: ClientMsg[] = [];

  state: ConnState = "closed";

  connect(): void {
    this.closed = false;
    this.open();
  }

  dispose(): void {
    this.closed = true;
    this.stopPing();
    this.ws?.close();
    this.ws = null;
    for (const [, p] of this.pending) {
      clearTimeout(p.timer);
      p.reject(new Error("연결이 닫혔습니다"));
    }
    this.pending.clear();
  }

  onMessage(cb: (msg: HostMsg) => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }

  onState(cb: (s: ConnState) => void): () => void {
    this.stateListeners.add(cb);
    cb(this.state);
    return () => this.stateListeners.delete(cb);
  }

  send(msg: ClientMsg): void {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    } else {
      // 재접속 중이면 큐에 담아 두었다가 open 시 흘린다(attach 유실 방지).
      this.queue.push(msg);
    }
  }

  /** 요청/응답 상관 RPC. 서버 allowlist 밖이면 `unknownCmd`로 거절된다. */
  rpc<T = unknown>(cmd: string, args?: unknown): Promise<T> {
    const id = this.nextId++;
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`응답이 없습니다: ${cmd}`));
      }, RPC_TIMEOUT_MS);
      this.pending.set(id, {
        resolve: resolve as (v: unknown) => void,
        reject,
        timer,
      });
      this.send({ type: "rpc", id, cmd, args });
    });
  }

  private setState(s: ConnState): void {
    this.state = s;
    for (const cb of this.stateListeners) cb(s);
  }

  private open(): void {
    if (this.closed) return;
    this.setState("connecting");
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const ws = new WebSocket(`${proto}//${location.host}/webremote/v1/ws`);
    this.ws = ws;

    ws.onopen = () => {
      this.backoff = RECONNECT_MIN_MS;
      this.setState("open");
      const queued = this.queue;
      this.queue = [];
      for (const msg of queued) ws.send(JSON.stringify(msg));
      this.startPing();
    };

    ws.onmessage = (ev) => {
      let msg: HostMsg;
      try {
        msg = JSON.parse(String(ev.data)) as HostMsg;
      } catch {
        return;
      }
      if (msg.type === "rpcResult") {
        const p = this.pending.get(msg.id);
        if (p) {
          this.pending.delete(msg.id);
          clearTimeout(p.timer);
          if (msg.ok) p.resolve(msg.data);
          else p.reject(new Error(msg.error?.message ?? "요청이 거부됐습니다"));
        }
        return;
      }
      for (const cb of this.listeners) {
        try {
          cb(msg);
        } catch (err) {
          console.error("web remote message handler threw", err);
        }
      }
    };

    ws.onclose = () => {
      this.stopPing();
      this.setState("closed");
      if (this.closed) return;
      setTimeout(() => this.open(), this.backoff);
      this.backoff = Math.min(this.backoff * 2, RECONNECT_MAX_MS);
    };

    ws.onerror = () => {
      // onclose가 이어서 온다 — 재접속은 거기서 한 번만 건다.
      ws.close();
    };
  }

  private startPing(): void {
    this.stopPing();
    this.pingTimer = setInterval(() => this.send({ type: "ping" }), PING_EVERY_MS);
  }

  private stopPing(): void {
    if (this.pingTimer !== null) {
      clearInterval(this.pingTimer);
      this.pingTimer = null;
    }
  }
}

/** 페어링이 끝났는지(=쿠키가 유효한지) 확인한다. WS가 401이면 페어링 화면. */
export async function probeAuth(): Promise<boolean> {
  return await new Promise<boolean>((resolve) => {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const ws = new WebSocket(`${proto}//${location.host}/webremote/v1/ws`);
    const done = (ok: boolean) => {
      ws.onopen = ws.onclose = ws.onerror = null;
      try {
        ws.close();
      } catch {
        /* 이미 닫힘 */
      }
      resolve(ok);
    };
    ws.onopen = () => done(true);
    ws.onerror = () => done(false);
    ws.onclose = () => done(false);
  });
}
