// src/renderer/markdown/markdownStore.ts
//
// 마크다운 문서 탐색·편집(이슈 #10) 전용 zustand 스토어. appStore와 분리한 이유:
// 이 상태(팔레트/편집기/목록 캐시)는 오피스 씬·세션과 무관한 독립 서브시스템이라
// 커플링을 피한다. 비동기 오케스트레이션(목록/읽기/쓰기)은 appStore.updateAppSettings
// 관례처럼 스토어 액션이 직접 tauriApi를 호출한다(테스트는 tauriApi 목으로 검증).
//
// 불변식:
// - 목록 캐시(`listing`)는 root별로 유지되어 재오픈 시 즉시 표시된다(백그라운드 재스캔).
// - 편집기 `dirty`는 `content !== baseline`으로 파생한다(isEditorDirty).
// - 저장 충돌("CONFLICT" 접두 reject)은 `conflict` 플래그로 다이얼로그를 띄운다.
import { create } from "zustand";
import { tauriApi } from "../ipc/tauriApi";
import { createInFlightTracker, isStale } from "../shared/createListingCache";
import type { MarkdownFileEntry } from "@shared/types";

/** root별 파일 목록 캐시 1건. */
export interface MarkdownListing {
  files: MarkdownFileEntry[];
  truncated: boolean;
  /** 이 캐시가 채워진 시각(Date.now()) — TTL 판정·"N분 전" 표시에 쓰인다. */
  fetchedAt: number;
}

/** 캐시 재사용 유효기간(이슈 #67): 이보다 오래되면 팔레트를 열 때 백그라운드로
 *  재스캔한다. 그 이내면 캐시를 그대로 쓰고 풀스캔을 건너뛴다. */
const LISTING_TTL_MS = 5 * 60_000;

/** root별 refreshListing 진행 상태(모듈 수준 — 스토어 재생성과 무관하게 유지). */
const listingInFlight = createInFlightTracker();

/** `.md`/`.mdx`/`.markdown` 확장자인지(대소문자 무시). workdirStore에도 동일
 *  판별이 있으나 그쪽이 이 스토어를 임포트하므로 순환을 피해 로컬로 둔다. */
function isMarkdownPath(relPath: string): boolean {
  return /\.(md|mdx|markdown)$/i.test(relPath);
}

/** 팔레트(Ctrl+P 유사) 상태. null = 닫힘. */
export interface PaletteState {
  /** 탐색 루트(해당 에이전트 cwd). */
  root: string;
  /** 진입점이 된 에이전트(표시용). */
  agentId: string;
  /** 퍼지 필터 쿼리. */
  query: string;
  /** 필터 결과 기준 선택 인덱스(컴포넌트가 결과 길이에 맞춰 클램프해 세팅). */
  selectedIndex: number;
}

/** 편집기 오버레이 상태. null = 닫힘. */
export interface EditorState {
  root: string;
  relPath: string;
  /** 진입 에이전트(팔레트 재오픈 시 root/표시에 재사용). */
  agentId: string;
  /** 현재 편집 중 내용. */
  content: string;
  /** 마지막 저장/로드 기준 내용(더티 판정 기준선). */
  baseline: string;
  /** 백엔드 버전 토큰(불투명). 저장 시 expectedVersion으로 사용. */
  version: string;
  mode: "source" | "preview";
  loading: boolean;
  saving: boolean;
  /** 읽기 실패 메시지(있으면 본문 대신 표시). */
  loadError: string | null;
  /** true면 저장 충돌 해결 다이얼로그 표시. */
  conflict: boolean;
  /** 편집기가 닫힐 때(어느 경로로든) 1회 호출되는 훅. 다른 서브시스템(예: 작업
   *  폴더 보기)에서 열었을 때 "닫으면 그 탐색 상태로 복귀"를 구현하는 데 쓴다. */
  onClose?: () => void;
}

/** 저장 결과 — 호출자(Cmd+S, Esc 저장후닫기)가 후속 동작을 정하기 위한 판별 유니온. */
export type SaveResult =
  | { ok: true }
  | { ok: false; conflict: true }
  | { ok: false; conflict: false; error: string };

interface MarkdownState {
  palette: PaletteState | null;
  editor: EditorState | null;
  /** 더티 상태에서 닫기 시도 시 뜨는 확인 다이얼로그 표시 여부. */
  discardConfirm: boolean;
  /** root별 목록 캐시(재오픈 즉시 표시용, 런타임 전용). */
  listing: Record<string, MarkdownListing>;

  // ---- 팔레트 ----
  /** 팔레트를 root로 연다(쿼리·선택 초기화) + 백그라운드 재스캔 트리거. */
  openPalette(root: string, agentId: string): void;
  closePalette(): void;
  setQuery(query: string): void;
  setSelectedIndex(index: number): void;
  /** 목록을 다시 읽어 캐시를 갱신한다(fire-and-forget 가능). 기본은 TTL을 따라
   *  캐시가 신선하면(5분 이내) 스킵한다. `force: true`는 TTL을 무시하고 항상
   *  스캔한다(수동 새로고침 버튼용). 같은 root에 대해 이미 진행 중이면 중복
   *  실행하지 않는다(in-flight dedupe). */
  refreshListing(root: string, opts?: { force?: boolean }): Promise<void>;

  // ---- 편집기 ----
  /** 파일을 읽어 편집기를 연다(성공 시 팔레트는 닫는다). `onClose`가 주어지면
   *  편집기가 닫히는 어느 경로에서든 1회 호출된다(호출자 탐색 상태 복귀용). */
  openFile(root: string, relPath: string, agentId: string, onClose?: () => void): Promise<void>;
  closeEditor(): void;
  setContent(content: string): void;
  setMode(mode: "source" | "preview"): void;
  /** 현재 내용을 저장한다. 충돌/실패는 SaveResult로 알린다. */
  save(): Promise<SaveResult>;

  // ---- 더티 가드 다이얼로그 ----
  requestClose(): void; // Esc 등: 더티면 확인 다이얼로그, 아니면 즉시 닫기
  cancelDiscard(): void;

  // ---- 충돌 해결 ----
  /** 다시 불러오기(내 변경 버림): 최신 내용/버전으로 교체. */
  reloadFromDisk(): Promise<void>;
  /** 내 내용으로 덮어쓰기: 최신 버전을 다시 읽어 그 버전으로 재저장. */
  overwrite(): Promise<SaveResult>;
  cancelConflict(): void;
}

/** 편집기 더티 여부(파생). content가 baseline과 다르면 더티. */
export function isEditorDirty(editor: EditorState | null): boolean {
  return editor !== null && editor.content !== editor.baseline;
}

/** reject 값에서 "CONFLICT" 접두 여부와 메시지 문자열을 뽑는다. */
function toErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}
function isConflict(err: unknown): boolean {
  return toErrorMessage(err).startsWith("CONFLICT");
}

export const useMarkdownStore = create<MarkdownState>()((set, get) => ({
  palette: null,
  editor: null,
  discardConfirm: false,
  listing: {},

  openPalette: (root, agentId) => {
    set({ palette: { root, agentId, query: "", selectedIndex: 0 } });
    // 캐시가 있으면 팔레트가 즉시 그것을 표시한다. 재스캔 여부(TTL)는
    // refreshListing 내부가 판단하므로 여기서는 그냥 호출만 한다.
    void get().refreshListing(root);
  },

  closePalette: () => set({ palette: null }),

  setQuery: (query) =>
    set((s) => (s.palette ? { palette: { ...s.palette, query, selectedIndex: 0 } } : s)),

  setSelectedIndex: (index) =>
    set((s) => (s.palette ? { palette: { ...s.palette, selectedIndex: index } } : s)),

  refreshListing: async (root, opts) => {
    const force = opts?.force ?? false;
    // TTL 이내면(force가 아닌 한) 스킵 — 캐시가 없으면 isStale이 true를 준다.
    if (!force && !isStale(get().listing[root], LISTING_TTL_MS)) return;
    if (!listingInFlight.begin(root)) return; // 이미 진행 중이면 중복 실행하지 않는다.
    try {
      const res = await tauriApi.markdownListFiles(root);
      set((s) => ({
        listing: {
          ...s.listing,
          [root]: { files: res.files, truncated: res.truncated, fetchedAt: Date.now() },
        },
      }));
    } catch (err) {
      // 실패 시 기존 캐시·fetchedAt은 그대로 유지한다.
      console.warn("markdown: 목록 재스캔 실패", err);
    } finally {
      listingInFlight.end(root);
    }
  },

  openFile: async (root, relPath, agentId, onClose) => {
    // 즉시 로딩 상태의 편집기를 띄우고 팔레트는 닫는다.
    set({
      palette: null,
      discardConfirm: false,
      editor: {
        root,
        relPath,
        agentId,
        content: "",
        baseline: "",
        version: "",
        // md류는 "읽기" 목적이 우선이므로 미리보기를 기본 뷰로 연다(이슈 #76).
        // 비-md가 유입돼도 안전하게 소스로 폴백.
        mode: isMarkdownPath(relPath) ? "preview" : "source",
        loading: true,
        saving: false,
        loadError: null,
        conflict: false,
        onClose,
      },
    });
    try {
      const res = await tauriApi.markdownReadFile(root, relPath);
      set((s) => {
        // 여는 도중 다른 파일로 바뀌었으면(경합) 무시.
        if (!s.editor || s.editor.root !== root || s.editor.relPath !== relPath) return s;
        return {
          editor: {
            ...s.editor,
            content: res.content,
            baseline: res.content,
            version: res.version,
            loading: false,
            loadError: null,
          },
        };
      });
    } catch (err) {
      set((s) => {
        if (!s.editor || s.editor.root !== root || s.editor.relPath !== relPath) return s;
        return { editor: { ...s.editor, loading: false, loadError: toErrorMessage(err) } };
      });
    }
  },

  closeEditor: () => {
    const cb = get().editor?.onClose;
    set({ editor: null, discardConfirm: false });
    cb?.();
  },

  setContent: (content) =>
    set((s) => (s.editor ? { editor: { ...s.editor, content } } : s)),

  setMode: (mode) => set((s) => (s.editor ? { editor: { ...s.editor, mode } } : s)),

  save: async () => {
    const editor = get().editor;
    if (!editor || editor.saving) return { ok: false, conflict: false, error: "no-editor" };
    set({ editor: { ...editor, saving: true } });
    try {
      const res = await tauriApi.markdownWriteFile(
        editor.root,
        editor.relPath,
        editor.content,
        editor.version,
      );
      set((s) =>
        s.editor
          ? {
              editor: {
                ...s.editor,
                saving: false,
                version: res.version,
                baseline: editor.content, // 저장 시점 내용이 새 기준선
                conflict: false,
              },
            }
          : s,
      );
      return { ok: true };
    } catch (err) {
      const conflict = isConflict(err);
      set((s) => (s.editor ? { editor: { ...s.editor, saving: false, conflict } } : s));
      return conflict
        ? { ok: false, conflict: true }
        : { ok: false, conflict: false, error: toErrorMessage(err) };
    }
  },

  requestClose: () => {
    const { editor } = get();
    if (!editor) return;
    if (isEditorDirty(editor)) set({ discardConfirm: true });
    else {
      const cb = editor.onClose;
      set({ editor: null, discardConfirm: false });
      cb?.();
    }
  },

  cancelDiscard: () => set({ discardConfirm: false }),

  reloadFromDisk: async () => {
    const editor = get().editor;
    if (!editor) return;
    try {
      const res = await tauriApi.markdownReadFile(editor.root, editor.relPath);
      set((s) =>
        s.editor
          ? {
              editor: {
                ...s.editor,
                content: res.content,
                baseline: res.content,
                version: res.version,
                conflict: false,
                loadError: null,
              },
            }
          : s,
      );
    } catch (err) {
      console.warn("markdown: 다시 불러오기 실패", err);
    }
  },

  overwrite: async () => {
    const editor = get().editor;
    if (!editor) return { ok: false, conflict: false, error: "no-editor" };
    try {
      // 최신 버전을 다시 읽어 그 버전으로 재저장한다(내 내용 유지).
      const latest = await tauriApi.markdownReadFile(editor.root, editor.relPath);
      const res = await tauriApi.markdownWriteFile(
        editor.root,
        editor.relPath,
        editor.content,
        latest.version,
      );
      set((s) =>
        s.editor
          ? {
              editor: {
                ...s.editor,
                version: res.version,
                baseline: editor.content,
                conflict: false,
              },
            }
          : s,
      );
      return { ok: true };
    } catch (err) {
      const conflict = isConflict(err);
      set((s) => (s.editor ? { editor: { ...s.editor, conflict } } : s));
      return conflict
        ? { ok: false, conflict: true }
        : { ok: false, conflict: false, error: toErrorMessage(err) };
    }
  },

  cancelConflict: () => set((s) => (s.editor ? { editor: { ...s.editor, conflict: false } } : s)),
}));
