// vscode-ext/src/tree.ts
//
// 캐릭터 트리. 폴러 스냅샷을 받아 캐릭터 → (알림 · 로그 파일) 2단 트리로 보여
// 준다. 캐릭터 정렬은 "알림 있는 것(최신 알림순) → 작업 중 → 이름순".
//
// 앱이 정본이므로 이 뷰는 읽기 전용이다(쓰기는 알림 클리어뿐).

import * as vscode from "vscode";

import { offlineMessage, type OfflineStatus, type NotificationEntry } from "./controlClient";
import {
  agentLogDir,
  formatBytes,
  listLogFiles,
  readLogHeader,
  sessionLogRoot,
  type LogFileInfo,
} from "./logFiles";
import type { AgentEntry, Snapshot } from "./poller";

/** 캐릭터 자식으로 한 번에 보여 주는 로그 파일 수. */
export const LOG_PAGE_SIZE = 20;

/** 로그 전사의 성격을 잊지 않게 툴팁에 같이 붙이는 주의 문구. */
const TRANSCRIPT_CAVEAT =
  "세션 로그는 2초 간격 tail + TUI 축약을 거친 '정리된 대화록'입니다 — 실시간 터미널 미러가 아닙니다.";

export type TreeNode =
  | { kind: "notice"; message: string }
  | { kind: "agent"; entry: AgentEntry; connected: boolean }
  | { kind: "notification"; agentId: string; notification: NotificationEntry }
  | { kind: "log"; file: LogFileInfo }
  | { kind: "more"; agentId: string; total: number }
  | { kind: "empty-logs" };

/** 알림 있는 캐릭터(최신 알림순) → 작업 중 → 이름순. */
export function sortAgents(entries: AgentEntry[]): AgentEntry[] {
  const latest = (e: AgentEntry) =>
    e.notifications.reduce((max, n) => (n.at > max ? n.at : max), 0);
  return [...entries].sort((a, b) => {
    const rank = (e: AgentEntry) => (e.notifications.length > 0 ? 0 : e.state ? 1 : 2);
    const ra = rank(a);
    const rb = rank(b);
    if (ra !== rb) return ra - rb;
    if (ra === 0) return latest(b) - latest(a);
    return a.name.localeCompare(b.name, "ko");
  });
}

/** 상대 시각 한 줄 표기. */
export function formatRelative(at: number, now: number): string {
  const diff = Math.max(0, now - at);
  const min = Math.floor(diff / 60_000);
  if (min < 1) return "방금";
  if (min < 60) return `${min}분 전`;
  const hour = Math.floor(min / 60);
  if (hour < 24) return `${hour}시간 전`;
  return `${Math.floor(hour / 24)}일 전`;
}

/** 알림 메시지를 한 줄 라벨로 줄인다. */
export function summarize(message: string, max = 60): string {
  const line = message.split("\n").find((l) => l.trim().length > 0)?.trim() ?? "";
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

/**
 * 트리 모양을 결정하는 값만 뽑은 서명. 같으면 다시 그릴 필요가 없다
 * (`at`처럼 매 틱 바뀌는 값은 제외 — 상대 시각은 다음 실제 변화에 따라온다).
 */
export function treeSignature(snapshot: Snapshot): string {
  const rows = snapshot.entries.map((e) =>
    [
      e.agentId,
      e.name,
      e.role,
      e.state ?? "",
      e.sessionId ?? "",
      e.notifications.map((n) => n.id).join(","),
    ].join("|"),
  );
  return [snapshot.status, snapshot.logRootMissing ? "no-logs" : "logs", ...rows].join("\n");
}

export class CharacterTreeProvider implements vscode.TreeDataProvider<TreeNode> {
  private readonly emitter = new vscode.EventEmitter<TreeNode | undefined>();
  readonly onDidChangeTreeData = this.emitter.event;

  private snapshot: Snapshot | undefined;
  private signature: string | undefined;
  private view: vscode.TreeView<TreeNode> | undefined;
  /** "더 보기"를 누른 캐릭터 — 로그 전체를 펼친다. */
  private readonly expanded = new Set<string>();

  setView(view: vscode.TreeView<TreeNode>): void {
    this.view = view;
  }

  update(snapshot: Snapshot): void {
    const signature = treeSignature(snapshot);
    const changed = signature !== this.signature;
    this.signature = signature;
    this.snapshot = snapshot;
    const pending = snapshot.entries.reduce((sum, e) => sum + e.notifications.length, 0);
    if (this.view) {
      this.view.badge =
        pending > 0 ? { value: pending, tooltip: `대기 중 알림 ${pending}개` } : undefined;
    }
    // 2초마다 트리를 통째로 다시 만들면 펼침·스크롤이 흔들린다 — 실제로
    // 달라진 틱에서만 갱신한다(강제 갱신은 refresh()가 한다).
    if (changed) this.emitter.fire(undefined);
  }

  /** 새로고침 명령 — 내용이 같아도 무조건 다시 그린다. */
  forceRefresh(): void {
    this.emitter.fire(undefined);
  }

  toggleExpanded(agentId: string): void {
    if (this.expanded.has(agentId)) this.expanded.delete(agentId);
    else this.expanded.add(agentId);
    this.emitter.fire(undefined);
  }

  /** 캐릭터의 로그 디렉터리(로그 폴더 열기 명령용). */
  logDirFor(agentId: string): string | undefined {
    const appDataDir = this.snapshot?.appDataDir;
    return appDataDir ? agentLogDir(sessionLogRoot(appDataDir), agentId) : undefined;
  }

  /** 캐릭터의 가장 최근 로그 파일(알림 토스트의 "로그 열기"용). */
  async newestLogFor(agentId: string): Promise<LogFileInfo | undefined> {
    const appDataDir = this.snapshot?.appDataDir;
    if (!appDataDir) return undefined;
    const files = await listLogFiles(sessionLogRoot(appDataDir), agentId);
    return files[0];
  }

  entryFor(agentId: string): AgentEntry | undefined {
    return this.snapshot?.entries.find((e) => e.agentId === agentId);
  }

  getTreeItem(node: TreeNode): vscode.TreeItem {
    switch (node.kind) {
      case "notice": {
        const item = new vscode.TreeItem(node.message);
        item.iconPath = new vscode.ThemeIcon("info");
        item.tooltip = `${node.message}\n\n${TRANSCRIPT_CAVEAT}`;
        item.command = { command: "agentOffice.refresh", title: "새로고침" };
        return item;
      }
      case "agent":
        return this.agentItem(node.entry, node.connected);
      case "notification": {
        const n = node.notification;
        const item = new vscode.TreeItem(`⚠ ${summarize(n.message)}`);
        item.description = formatRelative(n.at, this.snapshot?.at ?? Date.now());
        item.iconPath = new vscode.ThemeIcon("bell-dot", new vscode.ThemeColor("list.warningForeground"));
        item.contextValue = "agentOffice.notification";
        item.tooltip = n.message;
        item.command = {
          command: "agentOffice.showNotification",
          title: "알림 보기",
          arguments: [node],
        };
        return item;
      }
      case "log": {
        const item = new vscode.TreeItem(node.file.label);
        item.description = `${node.file.sid8 || "?"} · ${formatBytes(node.file.bytes)}`;
        item.iconPath = new vscode.ThemeIcon("output");
        item.contextValue = "agentOffice.log";
        item.command = {
          command: "agentOffice.openLog",
          title: "로그 열기",
          arguments: [node],
        };
        return item;
      }
      case "more": {
        const item = new vscode.TreeItem(
          this.expanded.has(node.agentId) ? "최근 것만 보기" : `더 보기… (총 ${node.total}개)`,
        );
        item.iconPath = new vscode.ThemeIcon("ellipsis");
        item.command = {
          command: "agentOffice.toggleLogPage",
          title: "더 보기",
          arguments: [node],
        };
        return item;
      }
      case "empty-logs": {
        const item = new vscode.TreeItem(
          "세션 로그가 없습니다 — 앱 설정에서 세션 로그 기록이 켜져 있는지 확인하세요.",
        );
        item.iconPath = new vscode.ThemeIcon("info");
        return item;
      }
    }
  }

  /** 로그 파일 툴팁(헤더의 cwd)은 호버할 때만 읽는다. */
  async resolveTreeItem(
    item: vscode.TreeItem,
    node: TreeNode,
  ): Promise<vscode.TreeItem> {
    if (node.kind === "log") {
      const header = await readLogHeader(node.file.path);
      const lines = [
        node.file.fileName,
        header.sessionId ? `세션: ${header.sessionId}` : undefined,
        header.cwd ? `폴더: ${header.cwd}` : undefined,
        `크기: ${formatBytes(node.file.bytes)}`,
        "",
        TRANSCRIPT_CAVEAT,
      ].filter((l): l is string => l !== undefined);
      item.tooltip = lines.join("\n");
    }
    return item;
  }

  async getChildren(node?: TreeNode): Promise<TreeNode[]> {
    const snapshot = this.snapshot;
    if (!snapshot) return [];

    if (!node) {
      const nodes: TreeNode[] = [];
      if (snapshot.status !== "ok") {
        nodes.push({
          kind: "notice",
          message:
            snapshot.message ??
            (snapshot.status === "error"
              ? "앱이 요청을 거절했습니다."
              : offlineMessage(snapshot.status as OfflineStatus)),
        });
      }
      const connected = snapshot.status === "ok";
      for (const entry of sortAgents(snapshot.entries)) {
        nodes.push({ kind: "agent", entry, connected });
      }
      return nodes;
    }

    if (node.kind !== "agent") return [];

    const children: TreeNode[] = node.entry.notifications
      .slice()
      .sort((a, b) => b.at - a.at)
      .map((notification) => ({
        kind: "notification" as const,
        agentId: node.entry.agentId,
        notification,
      }));

    if (snapshot.logRootMissing || !snapshot.appDataDir) {
      children.push({ kind: "empty-logs" });
      return children;
    }

    const files = await listLogFiles(sessionLogRoot(snapshot.appDataDir), node.entry.agentId);
    if (files.length === 0) {
      children.push({ kind: "empty-logs" });
      return children;
    }
    const showAll = this.expanded.has(node.entry.agentId);
    for (const file of showAll ? files : files.slice(0, LOG_PAGE_SIZE)) {
      children.push({ kind: "log", file });
    }
    if (files.length > LOG_PAGE_SIZE) {
      children.push({ kind: "more", agentId: node.entry.agentId, total: files.length });
    }
    return children;
  }

  private agentItem(entry: AgentEntry, connected: boolean): vscode.TreeItem {
    const item = new vscode.TreeItem(entry.name, vscode.TreeItemCollapsibleState.Collapsed);
    const hasNotifications = entry.notifications.length > 0;
    const state = !connected ? "앱 꺼짐" : entry.state ? "작업중" : "유휴";
    item.description = entry.role ? `${entry.role} · ${state}` : state;
    item.contextValue = hasNotifications
      ? "agentOffice.agentWithNotifications"
      : "agentOffice.agent";
    item.iconPath = hasNotifications
      ? new vscode.ThemeIcon("bell-dot", new vscode.ThemeColor("list.warningForeground"))
      : !connected
        ? new vscode.ThemeIcon("debug-disconnect")
        : entry.state
          ? new vscode.ThemeIcon("terminal")
          : new vscode.ThemeIcon("circle-large-outline");
    item.id = `agent:${entry.agentId}`;

    const tooltip = new vscode.MarkdownString();
    tooltip.appendMarkdown(`**${entry.name}**${entry.role ? ` — ${entry.role}` : ""}\n\n`);
    tooltip.appendMarkdown(`상태: ${state}\n\n`);
    if (entry.cwd) tooltip.appendMarkdown(`폴더: \`${entry.cwd}\`\n\n`);
    if (hasNotifications) {
      tooltip.appendMarkdown(`대기 중 알림 ${entry.notifications.length}개\n\n`);
    }
    tooltip.appendMarkdown(`_${TRANSCRIPT_CAVEAT}_`);
    item.tooltip = tooltip;
    return item;
  }
}
