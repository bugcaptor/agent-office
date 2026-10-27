// src/renderer/guiHarness/tauriStub.ts
//
// 헤드리스 GUI 하네스의 첫 모듈. `scripts/gui-shot.mjs`
// 가 index.html·mascot.html 맨 앞에 이 파일을 끼워 넣어, 앱 모듈이 평가되기 전에
// `window.__TAURI_INTERNALS__`를 심는다. 앱의 명령 호출은 Vite 플러그인이
// `ipc/tauriApi.ts`를 `fakeApi.ts`로 바꿔 받으므로 여기 오지 않는다. 여기는
// `@tauri-apps/api/window`·`event`·플러그인을 직접 부르는 소수 모듈만 받는다.
//
// 앱 번들에는 들어가지 않는다 — 하네스 서버만 이 파일을 로드한다.

type Callback = { cb: (payload: unknown) => void; once: boolean };

export interface StubLog {
  /** 호출된 명령 이름 순서대로(중복 포함). */
  calls: string[];
  /** 가짜 백엔드가 응답을 정의하지 않아 reject한 명령. */
  unfaked: string[];
}

declare global {
  interface Window {
    __TAURI_INTERNALS__?: unknown;
    __guiHarness?: Record<string, unknown> & { log: StubLog };
  }
}

const callbacks = new Map<number, Callback>();
const listeners = new Map<string, number[]>();
let nextId = 1;
const log: StubLog = { calls: [], unfaked: [] };

const params = new URLSearchParams(location.search);
const windowLabel = location.pathname.endsWith("mascot.html") ? "mascot" : "main";

function runCallback(id: number, payload: unknown): void {
  const c = callbacks.get(id);
  if (!c) return;
  if (c.once) callbacks.delete(id);
  c.cb(payload);
}

/** Tauri 이벤트를 렌더러에 흘린다(`listen()` 구독자에게 전달). */
export function emitTauriEvent(event: string, payload: unknown): void {
  for (const handler of listeners.get(event) ?? []) {
    runCallback(handler, { event, id: 0, payload });
  }
}

/** 창·플러그인 명령의 가짜 응답. 모르는 명령은 null(대부분 void 명령). */
function pluginResponse(cmd: string, args: Record<string, unknown>): unknown {
  switch (cmd) {
    case "plugin:event|listen": {
      const event = String(args.event);
      listeners.set(event, [...(listeners.get(event) ?? []), Number(args.handler)]);
      return args.handler;
    }
    case "plugin:event|unlisten": {
      const event = String(args.event);
      listeners.set(event, (listeners.get(event) ?? []).filter((h) => h !== args.eventId));
      return null;
    }
    case "plugin:event|emit":
    case "plugin:event|emit_to":
      emitTauriEvent(String(args.event), args.payload);
      return null;
    case "plugin:window|is_focused":
    case "plugin:window|is_visible":
      return true;
    case "plugin:window|scale_factor":
      return window.devicePixelRatio || 1;
    case "plugin:window|inner_size":
    case "plugin:window|outer_size":
      return { width: window.innerWidth, height: window.innerHeight };
    case "plugin:window|inner_position":
    case "plugin:window|outer_position":
      return { x: 0, y: 0 };
    case "plugin:notification|is_permission_granted":
      return false;
    default:
      return null;
  }
}

window.__TAURI_INTERNALS__ = {
  metadata: {
    currentWindow: { label: windowLabel },
    currentWebview: { windowLabel, label: windowLabel },
  },
  transformCallback(cb: (payload: unknown) => void, once = false) {
    const id = nextId++;
    callbacks.set(id, { cb, once });
    return id;
  },
  unregisterCallback(id: number) {
    callbacks.delete(id);
  },
  runCallback,
  callbacks,
  convertFileSrc: (path: string) => path,
  async invoke(cmd: string, args: Record<string, unknown> = {}) {
    log.calls.push(cmd);
    if (cmd.startsWith("plugin:")) return pluginResponse(cmd, args);
    // tauriApi.ts를 거치지 않은 앱 명령 — 하네스가 모르는 경로라 드러나게 한다.
    log.unfaked.push(cmd);
    throw new Error(`gui-harness: unfaked command ${cmd}`);
  },
};
window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener(event: string, id: number) {
    listeners.set(event, (listeners.get(event) ?? []).filter((h) => h !== id));
  },
};

window.__guiHarness = {
  log,
  scenario: params.get("scenario") ?? "office",
  emitTauriEvent,
};
