// src/renderer/ipc/tauriApi.ts
//
// `AgentOfficeApi` (frozen contract, see src/shared/types.ts) implemented
// thinly over `@tauri-apps/api`. Adapter only — no UI/store logic here
// (that belongs to Phase 4).
//
// One `Channel<OutputChunk>` per agentId, fanned out to a JS-side callback
// Set for `onData` (the frozen API allows multiple `onData` subscribers per
// agent); `wrapListen` wraps `listen()`'s async `UnlistenFn` to match
// `onData`'s synchronous unsubscribe contract. Test coverage (vitest) mocks
// invoke/Channel to exercise onData fanout/unsubscribe refcounting and
// wrapListen's pre-resolution unsubscribe path. Deviations from the
// original reference implementation:
// - Command/event names come from `@shared/ipc`'s `Commands`/`Events`
//   constants rather than being re-typed as literals (keeps this file and
//   the Rust backend from silently drifting on a typo).
// - `onData` fanout and `wrapListen` both guard against a throwing
//   subscriber: one bad callback must not stop delivery to the others, or
//   kill the channel/listener for future events.
// - `wrapListen`'s returned unsubscribe is idempotent (safe to call more
//   than once) to match `onData`'s unsubscribe contract.

import { invoke, Channel } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { Commands, Events } from "@shared/ipc";
import type {
  ActivityEvent,
  AgentOfficeApi,
  AppSettings,
  DiaryEntry,
  NotificationClearedEvent,
  NotificationEvent,
  OutputChunk,
  PersistedState,
  SessionStateEvent,
  SessionTurnRecord,
  WorkLogItem,
} from "@shared/types";

/** One Channel per agentId, fanned out to however many onData callbacks are registered. */
interface OutputSub {
  channel: Channel<OutputChunk>;
  cbs: Set<(data: string, bytes: number) => void>;
}
const outputSubs = new Map<string, OutputSub>();

/** Invokes a subscriber and swallows/logs exceptions so one bad callback can't break the rest. */
function safeInvoke<T>(cb: (payload: T) => void, payload: T): void {
  try {
    cb(payload);
  } catch (err) {
    console.error("tauriApi: subscriber callback threw", err);
  }
}

export const tauriApi: AgentOfficeApi = {
  async createSession(agentId, opts) {
    // `autostartClaude` is a frozen backward-compat wire field, not part of
    // these frozen renderer options; omission defaults to false, and the
    // profile's startupCommand decides which CLI (if any) auto-launches.
    // Observation-enabled new terminals define both direct `claude`/`codex`
    // wrappers via Windows PowerShell/pwsh functions, a Git Bash rcfile, or the
    // supported zsh ZDOTDIR shim. WSL observer wrapping remains unsupported.
    return await invoke(Commands.createSession, { agentId, opts: opts ?? null });
  },

  async disposeSession(agentId) {
    await invoke(Commands.disposeSession, { agentId });
  },

  writeInput(agentId, data) {
    void invoke(Commands.writeInput, { agentId, data }); // fire-and-forget
  },

  resize(agentId, cols, rows) {
    void invoke(Commands.resize, { agentId, cols, rows });
  },

  clearNotifications(agentId, ids) {
    void invoke(Commands.clearNotifications, { agentId, ids: ids ?? null });
  },

  async listNotifications(agentId) {
    return await invoke(Commands.listNotifications, { agentId });
  },

  async loadState() {
    return await invoke(Commands.loadState);
  },

  async saveState(state: PersistedState) {
    await invoke(Commands.saveState, { state });
  },

  setBadgeCount(n) {
    void invoke(Commands.setBadgeCount, { count: n });
  },

  async savePortrait(agentId, pngBase64) {
    await invoke(Commands.savePortrait, { agentId, pngBase64 });
  },

  async loadPortrait(agentId) {
    return await invoke(Commands.loadPortrait, { agentId });
  },

  async deletePortrait(agentId) {
    await invoke(Commands.deletePortrait, { agentId });
  },

  async saveSprite(agentId, pngBase64) {
    await invoke(Commands.saveSprite, { agentId, pngBase64 });
  },

  async loadSprite(agentId) {
    return await invoke(Commands.loadSprite, { agentId });
  },

  async deleteSprite(agentId) {
    await invoke(Commands.deleteSprite, { agentId });
  },

  async summarizeText(provider, instruction, text, purpose) {
    return await invoke(Commands.summarizeText, { provider, instruction, text, purpose });
  },

  async generateSpriteImage(description) {
    return await invoke(Commands.generateSpriteImage, { description });
  },

  async getAppSettings() {
    return await invoke(Commands.getAppSettings);
  },

  async setAppSettings(settings: AppSettings) {
    await invoke(Commands.setAppSettings, { settings });
  },

  async controlStatus() {
    return await invoke(Commands.controlStatus);
  },

  async controlApprove() {
    await invoke(Commands.controlApprove);
  },

  async controlRevoke() {
    await invoke(Commands.controlRevoke);
  },

  async botStart(agentId: string) {
    return await invoke(Commands.botStart, { agentId });
  },

  async botStop(agentId: string) {
    await invoke(Commands.botStop, { agentId });
  },

  async botStatus() {
    return await invoke(Commands.botStatus);
  },

  async listAvailableShells() {
    return await invoke(Commands.listAvailableShells);
  },

  async openInVscode(path) {
    await invoke(Commands.openInVscode, { path });
  },

  async openInTerminal(path) {
    await invoke(Commands.openInTerminal, { path });
  },

  async exportTerminalOutput(agentName, content) {
    return await invoke(Commands.exportTerminalOutput, { agentName, content });
  },

  async pickDirectory(initialDir) {
    return await invoke(Commands.pickDirectory, { initialDir: initialDir ?? null });
  },

  appendSessionTurn(record: SessionTurnRecord) {
    void invoke(Commands.appendSessionTurn, { record }); // fire-and-forget
  },

  async loadSessionTurns() {
    return await invoke(Commands.loadSessionTurns);
  },

  async appendDiaryEntry(agentId: string, entry: DiaryEntry) {
    await invoke(Commands.appendDiaryEntry, { agentId, entry });
  },

  async loadDiary(agentId: string) {
    return await invoke(Commands.loadDiary, { agentId });
  },

  async saveWorkLog(agentId: string, items: WorkLogItem[]) {
    await invoke(Commands.saveWorkLog, { agentId, items });
  },

  async loadWorkLogs() {
    return await invoke(Commands.loadWorkLogs);
  },

  async loadSessionEvents(fromAt: number, toAt: number) {
    return await invoke(Commands.loadSessionEvents, { fromAt, toAt });
  },

  async handoffSupported() {
    return await invoke(Commands.handoffSupported);
  },

  async handoffSessions(snapshots: Record<string, string>, renderedBytes: Record<string, number>) {
    return await invoke(Commands.handoffSessions, { snapshots, renderedBytes });
  },

  async adoptDetachedSessions() {
    return await invoke(Commands.adoptDetachedSessions);
  },

  async sessionBrokerMode() {
    return await invoke(Commands.sessionBrokerMode);
  },

  async uploadSessionSnapshots(
    snapshots: Record<string, string>,
    renderedBytes: Record<string, number>
  ) {
    await invoke(Commands.uploadSessionSnapshots, { snapshots, renderedBytes });
  },

  async listClaudeResumeSessions() {
    return await invoke(Commands.listClaudeResumeSessions);
  },

  async loadUsageSnapshot() {
    return await invoke(Commands.loadUsageSnapshot);
  },

  async markdownListFiles(root) {
    return await invoke(Commands.markdownListFiles, { root });
  },

  async markdownReadFile(root, relPath) {
    return await invoke(Commands.markdownReadFile, { root, relPath });
  },

  async markdownWriteFile(root, relPath, content, expectedVersion) {
    return await invoke(Commands.markdownWriteFile, { root, relPath, content, expectedVersion });
  },

  async workdirListFiles(root) {
    return await invoke(Commands.workdirListFiles, { root });
  },

  async workdirSearchFiles(root, query) {
    return await invoke(Commands.workdirSearchFiles, { root, query });
  },

  async workdirGitStatus(root) {
    return await invoke(Commands.workdirGitStatus, { root });
  },

  async workdirDiffFile(root, relPath, mode) {
    return await invoke(Commands.workdirDiffFile, { root, relPath, mode });
  },

  async workdirFileHistory(root, relPath, limit, skip) {
    return await invoke(Commands.workdirFileHistory, { root, relPath, limit, skip });
  },

  async workdirDiffCommit(root, commit, relPath) {
    return await invoke(Commands.workdirDiffCommit, { root, commit, relPath });
  },

  async workdirCommitFiles(root, commit, limit, skip) {
    return await invoke(Commands.workdirCommitFiles, { root, commit, limit, skip });
  },

  async workdirRepoLog(root, limit, skip, allBranches, query) {
    return await invoke(Commands.workdirRepoLog, { root, limit, skip, allBranches, query });
  },

  async workdirDifftool(root, relPath, mode, commit) {
    return await invoke(Commands.workdirDifftool, { root, relPath, mode, commit });
  },

  onData(agentId, cb) {
    let sub = outputSubs.get(agentId);
    if (!sub) {
      const channel = new Channel<OutputChunk>();
      const created: OutputSub = { channel, cbs: new Set() };
      channel.onmessage = (chunk) => {
        // §#49: pass the raw stream byte count alongside the text so the
        // renderer can accumulate it on write to derive snapshot offsets.
        for (const f of created.cbs) {
          try {
            f(chunk.data, chunk.bytes);
          } catch (err) {
            console.error("tauriApi: subscriber callback threw", err);
          }
        }
      };
      outputSubs.set(agentId, created);
      // Subscribe before/alongside createSession settling — any output that
      // arrives early is buffered by the backend's backlog.
      void invoke(Commands.subscribeOutput, { agentId, channel });
      sub = created;
    }
    sub.cbs.add(cb);

    let unsubscribed = false;
    return () => {
      if (unsubscribed) return;
      unsubscribed = true;
      const s = outputSubs.get(agentId);
      if (!s) return;
      s.cbs.delete(cb);
      if (s.cbs.size === 0) {
        outputSubs.delete(agentId);
        void invoke(Commands.unsubscribeOutput, { agentId });
      }
    };
  },

  onSessionState(cb) {
    return wrapListen<SessionStateEvent>(Events.sessionState, cb);
  },

  onNotification(cb) {
    return wrapListen<NotificationEvent>(Events.notificationNew, cb);
  },

  onNotificationCleared(cb) {
    return wrapListen<NotificationClearedEvent>(Events.notificationCleared, cb);
  },

  onActivity(cb) {
    return wrapListen<ActivityEvent>(Events.activityEvent, cb);
  },
};

// `listen()` resolves asynchronously with an `UnlistenFn`, but the frozen API
// contract wants a synchronous unsubscribe. If the caller unsubscribes before
// the promise settles, tear the listener down as soon as it does (no leak).
function wrapListen<T>(event: string, cb: (payload: T) => void): () => void {
  let un: UnlistenFn | null = null;
  let disposed = false;
  listen<T>(event, (e) => safeInvoke(cb, e.payload)).then((f) => {
    if (disposed) f();
    else un = f;
  });
  return () => {
    if (disposed) return;
    disposed = true;
    if (un) {
      un();
      un = null;
    }
  };
}
