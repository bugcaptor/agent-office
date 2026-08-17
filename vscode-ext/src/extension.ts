// vscode-ext/src/extension.ts
//
// 배선 담당. 순수 모듈(appData·controlClient·logFiles·poller)에 vscode를
// 물려 주고 명령·트리·가상 문서를 등록한다. 여기 말고는 vscode에 의존하는
// 코드가 tree.ts·logDoc.ts뿐이다.

import * as fs from "node:fs/promises";
import * as path from "node:path";
import * as vscode from "vscode";

import { appDataInputFromProcess, resolveAppData } from "./appData";
import { ControlClient, type NotificationEntry } from "./controlClient";
import { LOG_SCHEME, LogDocProvider, decodeLogUri, logUri, revealEnd } from "./logDoc";
import { logRootExists, sessionLogRoot, type LogFileInfo } from "./logFiles";
import { Poller, type AgentEntry, type Snapshot } from "./poller";
import { CharacterTreeProvider, type TreeNode } from "./tree";

/** editor/title의 "전체 불러오기" 버튼을 tail 문서에서만 보이게 하는 컨텍스트 키. */
const TAIL_ACTIVE_KEY = "agentOffice.logTailActive";

function config(): vscode.WorkspaceConfiguration {
  return vscode.workspace.getConfiguration("agentOffice");
}

function currentAppDataDir(): string | undefined {
  return resolveAppData(appDataInputFromProcess(config().get<string>("appDataDir", "")));
}

function tailKb(): number {
  return Math.max(8, config().get<number>("logTailKb", 512));
}

function makeClient(appDataDir: string | undefined): ControlClient {
  return new ControlClient(appDataDir);
}

export function activate(context: vscode.ExtensionContext): void {
  const tree = new CharacterTreeProvider();
  const view = vscode.window.createTreeView<TreeNode>("agentOffice.characters", {
    treeDataProvider: tree,
    showCollapseAll: true,
  });
  tree.setView(view);
  context.subscriptions.push(view);

  const logDocs = new LogDocProvider(tailKb);
  context.subscriptions.push(
    logDocs,
    vscode.workspace.registerTextDocumentContentProvider(LOG_SCHEME, logDocs),
  );

  const poller = new Poller({
    resolveAppDataDir: currentAppDataDir,
    createClient: makeClient,
    readProfiles: async (appDataDir) => {
      try {
        return await fs.readFile(path.join(appDataDir, "profiles.json"), "utf8");
      } catch {
        return undefined;
      }
    },
    logRootExists: (appDataDir) => logRootExists(sessionLogRoot(appDataDir)),
    pollIntervalMs: () => config().get<number>("pollIntervalMs", 2000),
    isFocused: () => vscode.window.state.focused,
  });
  context.subscriptions.push({ dispose: () => poller.dispose() });

  // 새 알림 감지용 — 첫 스냅샷은 기준선이므로 토스트를 띄우지 않는다.
  let seenNotificationIds: Set<string> | undefined;
  context.subscriptions.push(
    poller.onSnapshot((snapshot) => {
      tree.update(snapshot);
      const fresh = collectFresh(snapshot, seenNotificationIds);
      seenNotificationIds = new Set(
        snapshot.entries.flatMap((e) => e.notifications.map((n) => n.id)),
      );
      if (config().get<boolean>("notifyToast", false)) {
        for (const item of fresh) void showToast(item.entry, item.notification, tree);
      }
    }),
  );

  // 창이 다시 포커스를 얻으면 백오프를 기다리지 않고 곧바로 한 번 돈다.
  context.subscriptions.push(
    vscode.window.onDidChangeWindowState((state) => {
      if (state.focused) void poller.refresh();
    }),
  );
  // 설정이 바뀌면(간격·app_data) 즉시 반영.
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("agentOffice")) void poller.refresh();
    }),
  );

  const syncTailContext = () => {
    const active = vscode.window.activeTextEditor;
    const decoded = active ? decodeLogUri(active.document.uri) : undefined;
    void vscode.commands.executeCommand("setContext", TAIL_ACTIVE_KEY, decoded?.mode === "tail");
  };
  syncTailContext();
  context.subscriptions.push(
    vscode.window.onDidChangeActiveTextEditor(syncTailContext),
    vscode.window.onDidChangeVisibleTextEditors(syncTailContext),
  );

  context.subscriptions.push(
    vscode.commands.registerCommand("agentOffice.refresh", async () => {
      await poller.refresh();
      tree.forceRefresh();
    }),

    vscode.commands.registerCommand("agentOffice.openLog", async (node?: TreeNode) => {
      const file = fileOf(node);
      if (!file) return;
      const uri = logUri(file.path, "tail", file.fileName);
      const doc = await vscode.workspace.openTextDocument(uri);
      const editor = await vscode.window.showTextDocument(doc, { preview: false });
      revealEnd(editor);
    }),

    vscode.commands.registerCommand("agentOffice.openLogFile", async (node?: TreeNode) => {
      const file = fileOf(node);
      if (!file) return;
      await vscode.commands.executeCommand("vscode.open", vscode.Uri.file(file.path));
    }),

    vscode.commands.registerCommand("agentOffice.openLogFull", async (node?: TreeNode) => {
      // 트리에서 왔으면 그 파일, 에디터 버튼에서 왔으면 현재 tail 문서의 파일.
      const fromNode = fileOf(node);
      const active = vscode.window.activeTextEditor;
      const fromEditor = active ? decodeLogUri(active.document.uri) : undefined;
      const filePath = fromNode?.path ?? fromEditor?.filePath;
      if (!filePath) return;
      const doc = await vscode.workspace.openTextDocument(logUri(filePath, "full"));
      await vscode.window.showTextDocument(doc, { preview: false });
    }),

    vscode.commands.registerCommand("agentOffice.showNotification", async (node?: TreeNode) => {
      if (!node || node.kind !== "notification") return;
      const picked = await vscode.window.showInformationMessage(
        node.notification.message,
        "지우기",
      );
      if (picked === "지우기") {
        await clearNotifications(node.agentId, [node.notification.id]);
        await poller.refresh();
      }
    }),

    vscode.commands.registerCommand(
      "agentOffice.clearAgentNotifications",
      async (node?: TreeNode) => {
        const agentId = agentIdOf(node);
        if (!agentId) return;
        await clearNotifications(agentId);
        await poller.refresh();
      },
    ),

    vscode.commands.registerCommand("agentOffice.revealLogDir", async (node?: TreeNode) => {
      const agentId = agentIdOf(node);
      if (!agentId) return;
      const dir = tree.logDirFor(agentId);
      if (!dir) {
        void vscode.window.showWarningMessage(
          "앱 데이터 폴더를 찾지 못했습니다 — 설정 agentOffice.appDataDir을 지정하세요.",
        );
        return;
      }
      // 로그가 한 번도 없던 캐릭터는 디렉터리 자체가 없다 — 루트로 대신 연다.
      const appDataDir = currentAppDataDir();
      const fallback = appDataDir ? sessionLogRoot(appDataDir) : undefined;
      const target = (await logRootExists(dir)) ? dir : fallback;
      if (!target || !(await logRootExists(target))) {
        void vscode.window.showInformationMessage(
          "세션 로그 폴더가 아직 없습니다 — 앱 설정에서 세션 로그 기록이 켜져 있는지 확인하세요.",
        );
        return;
      }
      await vscode.commands.executeCommand("revealFileInOS", vscode.Uri.file(target));
    }),

    vscode.commands.registerCommand("agentOffice.toggleLogPage", (node?: TreeNode) => {
      if (node?.kind === "more") tree.toggleExpanded(node.agentId);
    }),
  );

  poller.start();
}

export function deactivate(): void {
  // 정리는 전부 context.subscriptions가 맡는다.
}

/** 알림 클리어 — 유일한 쓰기 경로. 실패는 사용자에게 알린다(토큰은 노출 없음). */
async function clearNotifications(agentId: string, ids?: string[]): Promise<void> {
  const res = await makeClient(currentAppDataDir()).clear(agentId, ids);
  if (res.status === "ok") return;
  const detail = res.status === "error" ? res.message : res.status;
  void vscode.window.showWarningMessage(`알림을 지우지 못했습니다 (${detail}).`);
}

function fileOf(node?: TreeNode): LogFileInfo | undefined {
  return node && node.kind === "log" ? node.file : undefined;
}

function agentIdOf(node?: TreeNode): string | undefined {
  if (!node) return undefined;
  if (node.kind === "agent") return node.entry.agentId;
  if (node.kind === "notification" || node.kind === "more") return node.agentId;
  return undefined;
}

interface FreshNotification {
  entry: AgentEntry;
  notification: NotificationEntry;
}

/** 직전 스냅샷에 없던 알림만 골라낸다(첫 스냅샷은 전부 기준선). */
function collectFresh(snapshot: Snapshot, seen: Set<string> | undefined): FreshNotification[] {
  if (!seen) return [];
  const fresh: FreshNotification[] = [];
  for (const entry of snapshot.entries) {
    for (const notification of entry.notifications) {
      if (!seen.has(notification.id)) fresh.push({ entry, notification });
    }
  }
  return fresh;
}

async function showToast(
  entry: AgentEntry,
  notification: NotificationEntry,
  tree: CharacterTreeProvider,
): Promise<void> {
  const picked = await vscode.window.showInformationMessage(
    `${entry.name}: ${notification.message}`,
    "지우기",
    "로그 열기",
  );
  if (picked === "지우기") {
    await clearNotifications(entry.agentId, [notification.id]);
    await vscode.commands.executeCommand("agentOffice.refresh");
    return;
  }
  if (picked === "로그 열기") {
    const newest = await tree.newestLogFor(entry.agentId);
    if (newest) {
      await vscode.commands.executeCommand("agentOffice.openLog", {
        kind: "log",
        file: newest,
      } satisfies TreeNode);
    } else {
      void vscode.window.showInformationMessage("이 캐릭터의 세션 로그가 아직 없습니다.");
    }
  }
}
