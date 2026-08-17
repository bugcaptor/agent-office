// vscode-ext/src/logDoc.ts
//
// 세션 로그를 읽기 전용 가상 문서로 보여 주는 TextDocumentContentProvider.
// scheme은 `agent-office-log`, 실제 파일 경로와 모드(tail|full)를 query에 담는다.
//
// 이 확장의 존재 이유는 자동 스크롤 규칙이다: 갱신 직전 그 문서를 보이는
// 에디터에서 **마지막 줄이 보이고 있었을 때만** 갱신 후 끝으로 스크롤한다.
// 위로 올려 읽고 있는 사용자를 절대 방해하지 않는다.

import * as fs from "node:fs";
import * as vscode from "vscode";

import { readFullText, readTailSlice } from "./logFiles";

export const LOG_SCHEME = "agent-office-log";

export type LogMode = "tail" | "full";

/** 마지막 줄이 이 줄 수 안에 보이면 "끝을 보고 있다"로 본다. */
const AT_END_TOLERANCE = 2;
/** append 폭주 시 갱신을 묶는 시간. */
const CHANGE_DEBOUNCE_MS = 250;
/** fs.watch가 못 붙는 환경의 폴백 폴링 간격. */
const STAT_POLL_MS = 2000;
/** 갱신 내용이 끝내 오지 않을 때 예약된 스크롤을 포기하는 시간. */
const REVEAL_EXPIRY_MS = 3000;

interface Decoded {
  filePath: string;
  mode: LogMode;
}

/** 가상 문서 URI를 만든다. 탭 제목이 되는 path에 모드를 눈에 보이게 남긴다. */
export function logUri(filePath: string, mode: LogMode, fileName?: string): vscode.Uri {
  const base = fileName ?? filePath.split(/[\\/]/).pop() ?? "session.log";
  return vscode.Uri.from({
    scheme: LOG_SCHEME,
    path: `/${mode === "full" ? `${base} (전체)` : base}`,
    query: JSON.stringify({ path: filePath, mode }),
  });
}

/** URI에서 파일 경로와 모드를 되꺼낸다. 형식이 아니면 undefined. */
export function decodeLogUri(uri: vscode.Uri): Decoded | undefined {
  if (uri.scheme !== LOG_SCHEME) return undefined;
  try {
    const parsed = JSON.parse(uri.query) as { path?: unknown; mode?: unknown };
    if (typeof parsed.path !== "string" || !parsed.path) return undefined;
    return { filePath: parsed.path, mode: parsed.mode === "full" ? "full" : "tail" };
  } catch {
    return undefined;
  }
}

/** 에디터가 문서의 마지막 줄을 보고 있는가(= 자동 스크롤 허용 조건). */
export function isViewingEnd(editor: vscode.TextEditor): boolean {
  const lastLine = editor.document.lineCount - 1;
  return editor.visibleRanges.some((r) => r.end.line >= lastLine - AT_END_TOLERANCE);
}

/** 문서 끝으로 스크롤한다(커서는 건드리지 않는다). */
export function revealEnd(editor: vscode.TextEditor): void {
  const lastLine = Math.max(0, editor.document.lineCount - 1);
  editor.revealRange(new vscode.Range(lastLine, 0, lastLine, 0), vscode.TextEditorRevealType.Default);
}

interface Follower {
  watcher?: fs.FSWatcher;
  poll?: ReturnType<typeof setInterval>;
  debounce?: ReturnType<typeof setTimeout>;
  lastSignature: string;
}

export class LogDocProvider implements vscode.TextDocumentContentProvider, vscode.Disposable {
  private readonly emitter = new vscode.EventEmitter<vscode.Uri>();
  readonly onDidChange = this.emitter.event;

  private readonly followers = new Map<string, Follower>();
  private readonly disposables: vscode.Disposable[] = [];
  /** 갱신 직전 끝을 보고 있어서, 새 내용이 들어오면 끝으로 따라갈 문서들. */
  private readonly pendingReveal = new Set<string>();

  constructor(private readonly tailKb: () => number) {
    this.disposables.push(
      // 문서가 닫히면(= 더 이상 보이지 않으면) watcher를 놓아 준다.
      vscode.workspace.onDidCloseTextDocument((doc) => {
        if (doc.uri.scheme === LOG_SCHEME) {
          this.stopFollowing(doc.uri.toString());
          this.pendingReveal.delete(doc.uri.toString());
        }
      }),
      // 새 내용이 실제로 반영된 뒤에 스크롤한다(시간 추측 없이 정확한 시점).
      vscode.workspace.onDidChangeTextDocument((e) => {
        if (e.document.uri.scheme !== LOG_SCHEME) return;
        const key = e.document.uri.toString();
        if (!this.pendingReveal.delete(key)) return;
        for (const editor of vscode.window.visibleTextEditors) {
          if (editor.document.uri.toString() === key) revealEnd(editor);
        }
      }),
    );
  }

  async provideTextDocumentContent(uri: vscode.Uri): Promise<string> {
    const decoded = decodeLogUri(uri);
    if (!decoded) return "잘못된 로그 URI입니다.";
    try {
      if (decoded.mode === "full") {
        return await readFullText(decoded.filePath);
      }
      const slice = await readTailSlice(decoded.filePath, this.tailKb());
      // tail 모드만 따라간다 — full은 그 시점 스냅샷이다.
      this.startFollowing(uri, decoded.filePath);
      return slice.text;
    } catch (err) {
      return `로그를 읽을 수 없습니다: ${err instanceof Error ? err.message : String(err)}`;
    }
  }

  dispose(): void {
    for (const key of [...this.followers.keys()]) this.stopFollowing(key);
    this.pendingReveal.clear();
    for (const d of this.disposables) d.dispose();
    this.emitter.dispose();
  }

  private startFollowing(uri: vscode.Uri, filePath: string): void {
    const key = uri.toString();
    if (this.followers.has(key)) return;
    const follower: Follower = { lastSignature: "" };
    this.followers.set(key, follower);

    const onTouched = () => this.scheduleChange(uri, filePath);
    try {
      const watcher = fs.watch(filePath, { persistent: false }, onTouched);
      watcher.on("error", () => {
        watcher.close();
        follower.watcher = undefined;
        this.startPolling(follower, filePath, onTouched);
      });
      follower.watcher = watcher;
    } catch {
      // 플랫폼·파일시스템에 따라 watch가 실패한다 — stat 폴링으로 대체.
      this.startPolling(follower, filePath, onTouched);
    }
  }

  private startPolling(follower: Follower, filePath: string, onTouched: () => void): void {
    if (follower.poll) return;
    follower.poll = setInterval(() => {
      fs.stat(filePath, (err, st) => {
        if (err) return;
        const signature = `${st.size}:${st.mtimeMs}`;
        if (signature !== follower.lastSignature) {
          follower.lastSignature = signature;
          onTouched();
        }
      });
    }, STAT_POLL_MS);
  }

  private stopFollowing(key: string): void {
    const follower = this.followers.get(key);
    if (!follower) return;
    follower.watcher?.close();
    if (follower.poll) clearInterval(follower.poll);
    if (follower.debounce) clearTimeout(follower.debounce);
    this.followers.delete(key);
  }

  private scheduleChange(uri: vscode.Uri, filePath: string): void {
    const key = uri.toString();
    const follower = this.followers.get(key);
    if (!follower) return;
    if (follower.debounce) clearTimeout(follower.debounce);
    follower.debounce = setTimeout(() => {
      follower.debounce = undefined;
      // 파일이 사라졌으면 조용히 따라가기를 멈춘다.
      fs.access(filePath, fs.constants.R_OK, (err) => {
        if (err) {
          this.stopFollowing(key);
          return;
        }
        this.fireChange(uri);
      });
    }, CHANGE_DEBOUNCE_MS);
  }

  /**
   * 갱신을 알린다. **갱신 직전** 끝을 보고 있었을 때만 따라가기를 예약한다 —
   * 위로 올려 읽는 중인 사용자는 건드리지 않는다(이 확장의 존재 이유).
   */
  private fireChange(uri: vscode.Uri): void {
    const key = uri.toString();
    const wasAtEnd = vscode.window.visibleTextEditors.some(
      (e) => e.document.uri.toString() === key && isViewingEnd(e),
    );
    if (wasAtEnd) {
      this.pendingReveal.add(key);
      // 내용이 실제로 같으면 변경 이벤트가 오지 않는다 — 예약을 무한정 두지 않는다.
      setTimeout(() => this.pendingReveal.delete(key), REVEAL_EXPIRY_MS);
    }
    this.emitter.fire(uri);
  }
}
