// Remote desktop adapter. Profiles, media and sessions belong to the host;
// portable file dialogs run on the client. No local state fallback is allowed.
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Commands } from "@shared/ipc";
import type {
  AgentOfficeApi,
  AppSettings,
  PersistedState,
} from "@shared/types";
import { useAppStore } from "../store/appStore";
import type { AgentProfile } from "../store/types";

export interface RemoteSnapshot {
  state: PersistedState;
  settings: AppSettings;
  sessions: { agentId: string; sessionId: string; cols: number; rows: number; state?: string }[];
  revision: string;
}

export interface RemoteConnection {
  hostName: string;
  permission: string;
}

export interface SavedRemoteConnection {
  url: string;
  token: string;
}

export async function loadRemoteConnection(): Promise<SavedRemoteConnection | null> {
  return await invoke(Commands.remoteLoadConnection);
}

export type RemoteTerminalMessage =
  | { type: "restore"; restoreId?: string; agentId: string; snapshot: string | null; baseOffset?: number; cols: number; rows: number; sessionId?: string }
  | { type: "output"; replay?: boolean; agentId: string; data: string; bytes: number; offset: number; seq?: number; sessionId?: string }
  | { type: "resized"; agentId: string; cols: number; rows: number; sessionId?: string };

async function rpc<T>(cmd: string, args?: unknown): Promise<T> {
  return await invoke(Commands.remoteRpc, { cmd, args: args ?? null });
}

export async function connectRemote(url: string, token: string): Promise<RemoteConnection> {
  const connection = await invoke<RemoteConnection>(Commands.remoteConnect, { url, token });
  remoteActive = true;
  return connection;
}

export async function loadRemoteSnapshot(): Promise<RemoteSnapshot> {
  return await rpc<RemoteSnapshot>("office.snapshot");
}

let updateSnapshotRevision: ((snapshot: RemoteSnapshot) => boolean) | null = null;
export function acceptRemoteSnapshot(snapshot: RemoteSnapshot): boolean {
  return updateSnapshotRevision?.(snapshot) ?? false;
}
export function currentRemoteState(): PersistedState {
  const store = useAppStore.getState();
  return { version: 1, agents: store.agentOrder.map((id) => store.agents[id]).filter((a): a is AgentProfile => a != null), vacationMode: store.vacationMode };
}
export function remoteStateKey(state: PersistedState): string {
  const normalized = { ...state, vacationMode: state.vacationMode ?? false, agents: state.agents.map((a) => ({...a, archetype: a.archetype ?? "human"})) };
  return JSON.stringify(normalized, (_key, value: unknown) => value && typeof value === "object" && !Array.isArray(value)
    ? Object.fromEntries(Object.entries(value).sort(([a],[b]) => a.localeCompare(b))) : value);
}
export function reportRemoteError(error: unknown): void {
  if (typeof window !== "undefined") window.dispatchEvent(new CustomEvent("remote-client-error", {detail: String(error)}));
}

export function disconnectRemote(): void {
  remoteActive = false;
  remoteOffsets.clear();
  void invoke(Commands.remoteDisconnect);
}

interface RemoteCursor { received: number; applied: number; generation: number; sessionId?: string; restoreId?: string }
const remoteOffsets = new Map<string, RemoteCursor>();
let remoteActive = false;
const replayingAgents = new Set<string>();
export function setRemoteReplaying(agentId: string, replaying: boolean): void {
  if (replaying) replayingAgents.add(agentId); else replayingAgents.delete(agentId);
}

export function setRemoteOffset(agentId: string, offset: number, sessionId?: string, restoreId?: string): number {
  const generation = (remoteOffsets.get(agentId)?.generation ?? 0) + 1;
  remoteOffsets.set(agentId, { received: offset, applied: offset, generation, sessionId, restoreId });
  return generation;
}

/** Called only after xterm's write callback; native reconnects from this ack. */
export function acknowledgeRemoteOffset(agentId: string, offset: number, generation: number): void {
  if (!remoteActive) return;
  const cursor = remoteOffsets.get(agentId);
  if (!cursor || cursor.generation !== generation) return;
  // A restored snapshot must ACK its base even when it equals applied.
  if (offset < cursor.applied) return;
  cursor.applied = offset;
  remoteOffsets.set(agentId, cursor);
  void invoke(Commands.remoteAck, { agentId, offset, restoreId: cursor.restoreId ?? null });
}

function remoteEvent<T>(name: string, cb: (payload: T) => void): () => void {
  let disposed = false;
  let unlisten: (() => void) | null = null;
  void listen<T>(`remote:${name}`, (event) => cb(event.payload)).then((un) => {
    if (disposed) un();
    else unlisten = un;
  });
  return () => {
    disposed = true;
    unlisten?.();
    unlisten = null;
  };
}

/** The snapshot revision lives only in this module; it is never persisted locally. */
export function createRemoteApi(snapshot: RemoteSnapshot): AgentOfficeApi {
  let revision = snapshot.revision;
  let saveChain: Promise<void> = Promise.resolve();
  let pendingSaves = 0;
  let savedState = remoteStateKey(snapshot.state);
  updateSnapshotRevision = (next) => {
    if (pendingSaves > 0 || remoteStateKey(currentRemoteState()) !== savedState) return false;
    revision = next.revision;
    savedState = remoteStateKey(next.state);
    return true;
  };
  const subs = new Map<string, {
    channel: Channel<unknown>;
    callbacks: Set<(data: string, bytes: number) => void>;
    queue: Promise<void>;
  }>();

  const unsupported = (name: string) => () => Promise.reject(new Error(`remote-unsupported: ${name}`));
  const api: Partial<AgentOfficeApi> = {
    async loadState() { return snapshot.state; },
    async saveState(state) {
      pendingSaves += 1;
      saveChain = saveChain.catch(() => {}).then(async () => {
        const result = await rpc<{ revision: string }>("office.saveState", { state, revision });
        revision = result.revision;
        savedState = remoteStateKey(state);
      }).finally(() => { pendingSaves -= 1; });
      try { return await saveChain; } catch (error) { reportRemoteError(error); throw error; }
    },
    async getAppSettings() { return { settings: snapshot.settings, firstRun: false }; },
    async createSession(agentId, opts) {
      // ProfileDialog updates the store synchronously but normal persistence is
      // debounced. Persist this exact store image before asking the host to
      // start the just-created profile.
      const store = useAppStore.getState();
      const state: PersistedState = {
        agents: store.agentOrder.map((id) => store.agents[id]).filter((a): a is AgentProfile => a != null),
        version: 1,
        vacationMode: store.vacationMode,
      };
      await api.saveState!(state);
      return await rpc("session.start", { agentId, opts: opts ?? null });
    },
    async disposeSession(agentId) { await rpc("session.dispose", { agentId }); },
    writeInput(agentId, data, source) {
      if (source === "terminalResponse" && replayingAgents.has(agentId)) return;
      void invoke(Commands.remoteInput, { agentId, data }).catch(reportRemoteError);
    },
    resize(agentId, cols, rows) { void rpc("session.resize", { agentId, cols, rows }).catch(reportRemoteError); },
    async listNotifications(agentId) { return await rpc("notifications.list", { agentId }); },
    clearNotifications(agentId, ids) { void rpc("notifications.clear", { agentId, ids: ids ?? null }).catch(reportRemoteError); },
    // Desktop-only indicators must be no-ops in a remote window. They are
    // deliberately synchronous because existing call sites fire-and-forget.
    setBadgeCount() {},
    async setKeepAwake() {},
    appendSessionTurn: async () => {},
    async listAvailableShells() { return await rpc("office.shells"); },
    async loadPortrait(agentId) { return await rpc("media.portrait", { agentId }); },
    async loadSprite(agentId) { return await rpc("office.media.load", { agentId, kind: "sprite" }); },
    async loadMinimi(agentId) { return await rpc("office.media.load", { agentId, kind: "minimi" }); },
    async savePortrait(agentId, pngBase64) { await rpc("office.media.save", { agentId, kind: "portrait", pngBase64 }); },
    async saveSprite(agentId, pngBase64) { await rpc("office.media.save", { agentId, kind: "sprite", pngBase64 }); },
    async saveMinimi(agentId, pngBase64) { await rpc("office.media.save", { agentId, kind: "minimi", pngBase64 }); },
    async deletePortrait(agentId) { await rpc("office.media.delete", { agentId, kind: "portrait" }); },
    async deleteSprite(agentId) { await rpc("office.media.delete", { agentId, kind: "sprite" }); },
    async deleteMinimi(agentId) { await rpc("office.media.delete", { agentId, kind: "minimi" }); },
    // File dialogs belong to the connected desktop; profile assets above
    // belong to the host. These commands only import/export a portable file.
    async exportCharacterFile(defaultName, content) { return await invoke(Commands.exportCharacterFile, { defaultName, content }); },
    async importCharacterFile() { return await invoke(Commands.importCharacterFile); },
    onData(agentId, cb) {
      let sub = subs.get(agentId);
      if (!sub) {
        const channel = new Channel<unknown>();
        const callbacks = new Set<(data: string, bytes: number) => void>();
        const created = { channel, callbacks, queue: Promise.resolve() };
        channel.onmessage = (raw) => {
          const msg = raw as RemoteTerminalMessage;
          if (msg.type !== "restore" && msg.type !== "output" && msg.type !== "resized") return;
          let generation: number;
          if (msg.type === "restore") {
            const cursor = remoteOffsets.get(agentId);
            if (msg.snapshot === null && cursor && cursor.sessionId === msg.sessionId) {
              // Output already queued before the socket dropped still belongs to
              // this terminal. Keep its receive watermark so replay cannot write
              // those bytes twice while the xterm callbacks are pending.
              cursor.restoreId = msg.restoreId;
              cursor.received = Math.max(cursor.received, msg.baseOffset ?? 0);
              generation = cursor.generation;
            } else {
              generation = setRemoteOffset(agentId, msg.baseOffset ?? 0, msg.sessionId, msg.restoreId);
            }
          } else if (msg.type === "output") {
            const cursor = remoteOffsets.get(agentId);
            const expected = cursor?.received ?? msg.offset;
            if (msg.offset < expected) return; // duplicate from a reconnect
            if (msg.offset > expected) {
              console.warn("remote output gap; requesting native replay", { agentId, expected, received: msg.offset });
              void invoke(Commands.remoteUnsubscribe, { agentId }).then(() =>
                invoke(Commands.remoteSubscribe, { agentId, channel }),
              );
              return;
            }
            generation = cursor?.generation ?? setRemoteOffset(agentId, msg.offset);
            remoteOffsets.set(agentId, { received: msg.offset + msg.bytes, applied: cursor?.applied ?? msg.offset, generation, sessionId: msg.sessionId, restoreId: cursor?.restoreId });
          } else {
            generation = remoteOffsets.get(agentId)?.generation ?? setRemoteOffset(agentId, 0);
          }
          // All frame kinds use one promise chain. In particular a restore
          // cannot race the first output while TerminalRegistry is loading.
          created.queue = created.queue.then(async () => {
            if (subs.get(agentId) !== created) return;
            const { terminalRegistry } = await import("../terminal/TerminalRegistry");
            await terminalRegistry.applyRemoteMessage(msg, generation);
          }).catch((error) => console.warn("remote terminal frame failed", error));
        };
        sub = created;
        subs.set(agentId, sub);
        void invoke(Commands.remoteSubscribe, { agentId, channel });
      }
      sub.callbacks.add(cb);
      return () => {
        const current = subs.get(agentId);
        if (!current) return;
        current.callbacks.delete(cb);
        if (current.callbacks.size === 0) {
          subs.delete(agentId);
          void invoke(Commands.remoteUnsubscribe, { agentId });
        }
      };
    },
    onSessionState(cb) { return remoteEvent("session-state", cb); },
    onNotification(cb) { return remoteEvent("notification-new", cb); },
    onNotificationCleared(cb) { return remoteEvent("notification-cleared", cb); },
    onActivity(cb) { return remoteEvent("activity-event", cb); },
    onTurnUsage(cb) { return remoteEvent("turn-usage", cb); },
    onTalkMessage(cb) { return remoteEvent("talk-message", cb); },
  };

  return new Proxy(api as AgentOfficeApi, {
    get(target, key) {
      const value = target[key as keyof AgentOfficeApi];
      if (value) return value.bind(target);
      // UI actions which only make sense on the host fail closed.  A rejected
      // promise leaves existing dialogs inert instead of touching this Mac.
      return unsupported(String(key));
    },
  });
}

export function remoteMessageFromUnknown(raw: unknown): RemoteTerminalMessage | null {
  if (!raw || typeof raw !== "object" || !("type" in raw)) return null;
  const msg = raw as RemoteTerminalMessage;
  return msg.type === "restore" || msg.type === "output" || msg.type === "resized" ? msg : null;
}
