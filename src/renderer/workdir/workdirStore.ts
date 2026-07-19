// src/renderer/workdir/workdirStore.ts
//
// "작업 폴더 보기"(이슈 #11) 전용 zustand 스토어. markdownStore와 같은 관례로
// appStore에서 분리했다 — 이 상태(파일 목록/ git 상태 캐시/팔레트)는 오피스
// 씬·세션과 무관한 독립 서브시스템이라 커플링을 피한다.
//
// 불변식:
// - 목록·git 캐시는 root별로 유지되어 재오픈 시 즉시 표시되고 백그라운드 갱신된다.
// - git 상태는 앱 설정 `gitStatusEnabled`가 켜졌을 때만 조회한다(거대 저장소
//   대비 off 스위치). 백엔드는 저장소 아님/타임아웃도 정상 응답으로 주므로
//   여기서는 성공/실패만 신경 쓴다.
// - git status 경로는 git을 실행한 cwd(=팔레트 root) 기준이라 파일 목록의
//   relPath와 그대로 매칭된다(root 밖 파일만 "../" 접두 — 목록엔 없고 "변경만"
//   뷰에만 나타난다).
//
// 인터랙션 모델(이슈 #54): **모든** 파일 클릭은 곧장 열지 않고 우측 상세(메뉴)
// 페인을 띄운다 — 변경 파일은 기본 "변경점" 탭, 변경 없는 파일은 기본 "히스토리"
// 탭으로 열려 깃 로그를 항상 볼 수 있다. 페인에서 "외부 프로그램으로 열기"·(마크
// 다운 등) "인앱 뷰어로 열기" 버튼으로 명시적으로 연다. 빠른 열기는 ⌘-클릭/더블
// 클릭으로 기존 자동 라우팅(openEntry)을 그대로 쓴다.
//
// 히스토리 탭의 커밋은 펼치면(toggleCommitExpand) 그 커밋이 바꾼 파일 목록을
// 인라인으로 보여주고(페이징), 파일을 고르면 그 커밋의 해당 파일 diff를 띄운다.
// 펼치지 않고 커밋만 고르면(selectCommit) 지금 파일의 그 커밋 시점 diff를 본다.
//
// 팔레트는 두 뷰 모드를 갖는다: "files"(파일 목록) / "log"(저장소 전체 커밋 로그
// 브라우저 — 파일 지목 없이 로그를 탐색, 커밋→변경파일→diff, 검색·전체브랜치).
//
// 파일 열기(openEntry, 빠른 열기용): .md는 인앱 편집기(markdownStore.openFile)로
// 위임하고, 그 외는 절대경로를 만들어 open_in_vscode로 외부 에디터에 넘긴다.
import { create, type StoreApi } from "zustand";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { useMarkdownStore } from "../markdown/markdownStore";
import type {
  GitCommitEntry,
  GitCommitFileEntry,
  GitDiffMode,
  GitDiffResult,
  GitStatusResult,
  WorkdirFileEntry,
} from "@shared/types";

/** root별 파일 목록 캐시 1건. */
export interface WorkdirListing {
  files: WorkdirFileEntry[];
  truncated: boolean;
}

/** 팔레트 뷰 모드: 파일 목록 / 저장소 전체 커밋 로그 브라우저(이슈 #54). */
export type WorkdirViewMode = "files" | "log";

/** 팔레트 상태. null = 닫힘. */
export interface WorkdirPaletteState {
  /** 탐색 루트(해당 에이전트 cwd). */
  root: string;
  /** 진입점이 된 에이전트(표시용). */
  agentId: string;
  /** 퍼지 필터 쿼리. */
  query: string;
  /** 필터 결과 기준 선택 인덱스(컴포넌트가 결과 길이에 맞춰 클램프). */
  selectedIndex: number;
  /** true면 git 변경 파일만 보여준다(전체 목록 대신 git 엔트리 기준). */
  changedOnly: boolean;
  /** 파일 목록 뷰 / 커밋 로그 브라우저 뷰(이슈 #54). */
  viewMode: WorkdirViewMode;
}

/** 우측 상세 페인 상태(변경점/히스토리). null = 상세 닫힘(목록만). */
export interface WorkdirDetail {
  /** 상세가 속한 root(팔레트 root와 동일하지만 stale 가드용으로 함께 보관). */
  root: string;
  relPath: string;
  name: string;
  /** git 뱃지 문자(M/A/D/?/…). 없으면 변경 없는 파일(상세로 오지 않음). */
  status?: string;
  /** 미추적(? ) 파일이면 diff 모드가 untracked 하나로 고정된다. */
  isUntracked: boolean;
  /** 활성 탭. */
  tab: "diff" | "history";
  /** 현재 diff 관점(untracked 파일은 항상 "untracked"). */
  diffMode: GitDiffMode;
  diff?: GitDiffResult;
  diffLoading: boolean;
  history?: GitCommitEntry[];
  historyLoading: boolean;
  historyHasMore: boolean;
  /** 히스토리에서 선택해 하단 diff를 보고 있는 커밋 해시. */
  selectedCommit?: string;
  /** 하단 diff가 보여주는 파일 경로. 기본은 이 상세의 파일(relPath)이지만,
   *  커밋을 펼쳐 다른 파일을 고르면 그 파일 경로가 된다(이슈 #54). */
  selectedCommitFile?: string;
  commitDiff?: GitDiffResult;
  commitDiffLoading: boolean;
  /** 변경파일 목록을 인라인으로 펼친 커밋 해시(이슈 #54). undefined = 안 펼침. */
  expandedCommit?: string;
  /** 펼친 커밋의 변경파일 목록(페이징 누적). */
  commitFiles?: GitCommitFileEntry[];
  commitFilesLoading: boolean;
  commitFilesHasMore: boolean;
  /** 다음 페이지 조회를 위한 skip(=이미 담긴 개수). */
  commitFilesSkip: number;
  /** diff 로드 세대 카운터(모드 전환 시 증가 → 늦게 도착한 stale 응답 폐기). */
  gen: number;
}

/** 저장소 전체 커밋 로그 브라우저 상태(이슈 #54, 2단계). 파일을 먼저 지목하지
 *  않고 로그→커밋→변경파일→diff 순으로 훑는다. root별로 유지(재오픈 즉시 표시). */
export interface WorkdirRepoLog {
  root: string;
  /** 커밋 메시지 검색어(대소문자 무시·부분일치). 빈 문자열 = 전체. */
  query: string;
  /** true면 `--all`로 모든 브랜치/참조의 커밋을 함께 본다. */
  allBranches: boolean;
  commits?: GitCommitEntry[];
  loading: boolean;
  hasMore: boolean;
  /** 이미 로드한 커밋 수(다음 페이지 skip). */
  loaded: number;
  timedOut: boolean;
  /** 선택된 커밋(그 커밋의 변경파일 목록을 로드). */
  selectedCommit?: string;
  files?: GitCommitFileEntry[];
  filesLoading: boolean;
  filesHasMore: boolean;
  filesLoaded: number;
  /** 변경파일 중 선택돼 diff를 보고 있는 파일 경로. */
  selectedFile?: string;
  fileDiff?: GitDiffResult;
  fileDiffLoading: boolean;
  /** 조회 세대 카운터(검색/브랜치 전환 시 증가 → stale 응답 폐기). */
  gen: number;
}

interface WorkdirState {
  palette: WorkdirPaletteState | null;
  /** root별 목록 캐시(재오픈 즉시 표시용, 런타임 전용). */
  listing: Record<string, WorkdirListing>;
  /** root별 git 상태 캐시(런타임 전용). */
  git: Record<string, GitStatusResult>;
  /** git 조회 진행 중 여부(root별). 헤더 스피너/상태 표시용. */
  gitLoading: Record<string, boolean>;
  /** 우측 상세 페인(변경점/히스토리). null = 목록만. */
  detail: WorkdirDetail | null;
  /** root별 커밋 로그 브라우저 상태(이슈 #54, 런타임 전용 캐시). */
  repoLog: Record<string, WorkdirRepoLog>;

  /** 팔레트를 root로 연다(쿼리·선택 초기화) + 목록/ git 백그라운드 갱신. */
  openPalette(root: string, agentId: string): void;
  closePalette(): void;
  setQuery(query: string): void;
  setSelectedIndex(index: number): void;
  setChangedOnly(changedOnly: boolean): void;
  /** 파일 목록 뷰 ↔ 커밋 로그 브라우저 뷰 전환(이슈 #54). log 최초 진입 시 로드. */
  setViewMode(mode: WorkdirViewMode): void;
  /** 파일 목록을 다시 읽어 캐시를 갱신한다(fire-and-forget 가능). */
  refreshListing(root: string): Promise<void>;
  /** git 상태를 다시 읽어 캐시를 갱신한다. 설정이 꺼져 있으면 캐시를 비운다. */
  refreshGit(root: string): Promise<void>;
  /** 빠른 열기(⌘-클릭/더블클릭): .md는 인앱 편집기, 그 외는 외부 에디터. */
  openEntry(root: string, relPath: string, name: string): void;

  /** 파일의 상세(메뉴) 페인을 연다. 변경 파일은 기본 "변경점" 탭, 변경 없는
   *  파일은 기본 "히스토리" 탭으로 열어 로그를 항상 노출한다(이슈 #54). */
  openDetail(root: string, relPath: string, name: string, status?: string): void;
  closeDetail(): void;
  setDetailTab(tab: "diff" | "history"): void;
  setDiffMode(mode: GitDiffMode): void;
  /** 현재 상세 파일을 외부 프로그램(open_in_vscode)으로 연다. .md도 강제 외부. */
  openExternal(): void;
  /** 인앱 뷰어로 연다(마크다운만 지원 — 그 외는 no-op). */
  openInApp(): void;
  /** 현재 상세의 diff를 (재)로드한다. */
  loadDiff(): Promise<void>;
  /** 현재 상세 파일의 커밋 히스토리를 로드한다(첫 페이지). */
  loadHistory(): Promise<void>;
  /** 히스토리에서 커밋을 선택해 지금 파일의 그 커밋 시점 diff를 로드한다. */
  selectCommit(hash: string): Promise<void>;
  /** 커밋 행을 펼쳐/접어 그 커밋이 바꾼 파일 목록을 인라인 표시한다(이슈 #54). */
  toggleCommitExpand(hash: string): Promise<void>;
  /** 펼친 커밋의 변경파일 다음 페이지를 이어 로드한다. */
  loadMoreCommitFiles(): Promise<void>;
  /** 펼친 커밋에서 파일을 골라 그 커밋의 해당 파일 diff를 하단에 로드한다. */
  selectCommitFile(hash: string, path: string): Promise<void>;
  /** 외부 비교 도구를 띄운다(fire-and-forget). commit 지정 시 그 커밋의 변경.
   *  현재 하단 diff가 보고 있는 파일(selectedCommitFile)을 대상으로 한다. */
  openDifftool(commit?: string): void;

  // ---- 커밋 로그 브라우저(이슈 #54, 2단계) ----
  /** 로그를 로드한다. reset이면 첫 페이지로 교체, 아니면 다음 페이지를 잇는다. */
  loadRepoLog(reset: boolean): Promise<void>;
  /** 검색어를 바꾸고 첫 페이지부터 재조회한다. */
  setRepoLogQuery(query: string): void;
  /** 전체 브랜치(--all) 토글 후 재조회한다. */
  setRepoLogAllBranches(all: boolean): void;
  /** 로그에서 커밋을 골라 그 커밋의 변경파일 목록을 로드한다(첫 페이지). */
  selectRepoCommit(hash: string): Promise<void>;
  /** 선택 커밋의 변경파일 다음 페이지를 잇는다. */
  loadMoreRepoFiles(): Promise<void>;
  /** 변경파일을 골라 그 커밋의 해당 파일 diff를 로드한다. */
  selectRepoFile(hash: string, path: string): Promise<void>;
  /** 로그 브라우저에서 외부 비교 도구를 띄운다(선택 커밋+파일). */
  openRepoDifftool(): void;
}

/** `.md`/`.mdx`/`.markdown` 확장자인지(대소문자 무시). */
export function isMarkdownPath(relPath: string): boolean {
  return /\.(md|mdx|markdown)$/i.test(relPath);
}

/** root와 상대경로로 절대경로를 만든다. 구분자는 '/'로 통일(VS Code·open은
 * Windows에서도 '/'를 받아준다). root의 후행 '/'는 중복을 피해 제거한다. */
export function joinPath(root: string, relPath: string): string {
  const base = root.replace(/[/\\]+$/, "");
  return `${base}/${relPath}`;
}

/** status 뱃지가 "변경된 파일"(상세로 보낼 대상)인지. 빈/없음은 변경 없음. */
export function isChangedStatus(status?: string): boolean {
  return !!status && status.length > 0;
}

/** 히스토리/로그 한 페이지 크기. */
const HISTORY_PAGE = 50;
/** 커밋 변경파일 인라인 목록 한 페이지 크기(more…로 이어 로드). */
const COMMIT_FILES_PAGE = 100;

/** 커밋 로그 브라우저 초기 상태(root 바인딩). */
function emptyRepoLog(root: string): WorkdirRepoLog {
  return {
    root,
    query: "",
    allBranches: false,
    commits: undefined,
    loading: false,
    hasMore: false,
    loaded: 0,
    timedOut: false,
    selectedCommit: undefined,
    files: undefined,
    filesLoading: false,
    filesHasMore: false,
    filesLoaded: 0,
    selectedFile: undefined,
    fileDiff: undefined,
    fileDiffLoading: false,
    gen: 0,
  };
}

export const useWorkdirStore = create<WorkdirState>()((set, get) => ({
  palette: null,
  listing: {},
  git: {},
  gitLoading: {},
  detail: null,
  repoLog: {},

  openPalette: (root, agentId) => {
    set({
      palette: { root, agentId, query: "", selectedIndex: 0, changedOnly: false, viewMode: "files" },
      detail: null,
    });
    // 캐시가 있으면 즉시 표시되고, 여기서 백그라운드 갱신.
    void get().refreshListing(root);
    void get().refreshGit(root);
  },

  closePalette: () => set({ palette: null, detail: null }),

  setQuery: (query) =>
    set((s) => (s.palette ? { palette: { ...s.palette, query, selectedIndex: 0 } } : s)),

  setSelectedIndex: (index) =>
    set((s) => (s.palette ? { palette: { ...s.palette, selectedIndex: index } } : s)),

  setChangedOnly: (changedOnly) =>
    set((s) => (s.palette ? { palette: { ...s.palette, changedOnly, selectedIndex: 0 } } : s)),

  setViewMode: (mode) => {
    const p = get().palette;
    if (!p) return;
    set({ palette: { ...p, viewMode: mode } });
    // 로그 뷰 최초 진입 시 한 번 로드(캐시에 커밋이 아직 없으면).
    if (mode === "log") {
      const rl = get().repoLog[p.root];
      if (!rl || rl.commits === undefined) void get().loadRepoLog(true);
    }
  },

  refreshListing: async (root) => {
    try {
      const res = await tauriApi.workdirListFiles(root);
      set((s) => ({
        listing: { ...s.listing, [root]: { files: res.files, truncated: res.truncated } },
      }));
    } catch (err) {
      console.warn("workdir: 파일 목록 조회 실패", err);
    }
  },

  refreshGit: async (root) => {
    // 설정이 꺼져 있으면 조회하지 않고 캐시를 비운다(뱃지 미표시).
    if (!useAppStore.getState().appSettings.gitStatusEnabled) {
      set((s) => {
        if (!(root in s.git)) return s;
        const next = { ...s.git };
        delete next[root];
        return { git: next };
      });
      return;
    }
    set((s) => ({ gitLoading: { ...s.gitLoading, [root]: true } }));
    try {
      const res = await tauriApi.workdirGitStatus(root);
      set((s) => ({
        git: { ...s.git, [root]: res },
        gitLoading: { ...s.gitLoading, [root]: false },
      }));
    } catch (err) {
      console.warn("workdir: git 상태 조회 실패", err);
      set((s) => ({ gitLoading: { ...s.gitLoading, [root]: false } }));
    }
  },

  openEntry: (root, relPath, name) => {
    const agentId = get().palette?.agentId ?? "";
    if (isMarkdownPath(relPath)) {
      // 인앱 마크다운 편집기로 위임하고 이 팔레트는 닫는다.
      set({ palette: null, detail: null });
      void useMarkdownStore.getState().openFile(root, relPath, agentId);
      return;
    }
    // 그 외 파일은 외부 에디터(VS Code 등)로 절대경로를 넘겨 연다.
    void tauriApi
      .openInVscode(joinPath(root, relPath))
      .catch((err) => console.warn(`파일 열기 실패: ${name}`, err));
  },

  openDetail: (root, relPath, name, status) => {
    const isUntracked = status === "?";
    // 미추적은 untracked 모드 고정, 그 외는 전체 변경 합본(worktreeVsHead)이 기본.
    const diffMode: GitDiffMode = isUntracked ? "untracked" : "worktreeVsHead";
    // 변경 파일은 변경점을 먼저 보여주고, 변경 없는(clean) 파일은 볼 변경점이
    // 없으므로 히스토리 탭으로 열어 깃 로그를 바로 노출한다(이슈 #54).
    const tab: "diff" | "history" = isChangedStatus(status) ? "diff" : "history";
    set((s) => ({
      detail: {
        root,
        relPath,
        name,
        status,
        isUntracked,
        tab,
        diffMode,
        diff: undefined,
        diffLoading: false,
        history: undefined,
        historyLoading: false,
        historyHasMore: false,
        selectedCommit: undefined,
        selectedCommitFile: undefined,
        commitDiff: undefined,
        commitDiffLoading: false,
        expandedCommit: undefined,
        commitFiles: undefined,
        commitFilesLoading: false,
        commitFilesHasMore: false,
        commitFilesSkip: 0,
        gen: (s.detail?.gen ?? 0) + 1,
      },
    }));
    // 변경점 탭이면 diff를, 히스토리 탭이면 로그를 즉시 로드(보이는 탭 우선).
    if (tab === "diff") void get().loadDiff();
    else void get().loadHistory();
  },

  closeDetail: () => set({ detail: null }),

  openExternal: () => {
    const d = get().detail;
    if (!d) return;
    // 마크다운 포함 항상 외부 에디터로. 팔레트는 유지(참조용).
    void tauriApi
      .openInVscode(joinPath(d.root, d.relPath))
      .catch((err) => console.warn(`외부 열기 실패: ${d.name}`, err));
  },

  openInApp: () => {
    const d = get().detail;
    if (!d) return;
    if (!isMarkdownPath(d.relPath)) return; // 인앱 지원 형식만.
    const agentId = get().palette?.agentId ?? "";
    set({ palette: null, detail: null });
    void useMarkdownStore.getState().openFile(d.root, d.relPath, agentId);
  },

  setDetailTab: (tab) => {
    set((s) => (s.detail ? { detail: { ...s.detail, tab } } : s));
    // 아직 로드 안 된 탭을 처음 열면 지연 로드(변경 없는 파일은 diff 탭이,
    // 변경 파일은 history 탭이 최초 진입 시 비어 있다).
    const d = get().detail;
    if (!d) return;
    if (tab === "history" && d.history === undefined && !d.historyLoading) {
      void get().loadHistory();
    } else if (tab === "diff" && d.diff === undefined && !d.diffLoading) {
      void get().loadDiff();
    }
  },

  setDiffMode: (mode) => {
    // gen을 올려 진행 중이던 이전 모드의 응답을 폐기하고 새로 로드.
    set((s) =>
      s.detail ? { detail: { ...s.detail, diffMode: mode, diff: undefined, gen: s.detail.gen + 1 } } : s,
    );
    void get().loadDiff();
  },

  loadDiff: async () => {
    const d = get().detail;
    if (!d) return;
    const { root, relPath, diffMode, gen } = d;
    set((s) => (s.detail ? { detail: { ...s.detail, diffLoading: true } } : s));
    try {
      const res = await tauriApi.workdirDiffFile(root, relPath, diffMode);
      set((s) =>
        s.detail && s.detail.gen === gen && s.detail.relPath === relPath
          ? { detail: { ...s.detail, diff: res, diffLoading: false } }
          : s,
      );
    } catch (err) {
      console.warn("workdir: diff 조회 실패", err);
      set((s) =>
        s.detail && s.detail.gen === gen ? { detail: { ...s.detail, diffLoading: false } } : s,
      );
    }
  },

  loadHistory: async () => {
    const d = get().detail;
    if (!d) return;
    const { root, relPath } = d;
    set((s) => (s.detail ? { detail: { ...s.detail, historyLoading: true } } : s));
    try {
      const res = await tauriApi.workdirFileHistory(root, relPath, HISTORY_PAGE, 0);
      set((s) =>
        s.detail && s.detail.relPath === relPath
          ? {
              detail: {
                ...s.detail,
                history: res.commits,
                historyHasMore: res.hasMore,
                historyLoading: false,
              },
            }
          : s,
      );
    } catch (err) {
      console.warn("workdir: 히스토리 조회 실패", err);
      set((s) =>
        s.detail && s.detail.relPath === relPath
          ? { detail: { ...s.detail, historyLoading: false } }
          : s,
      );
    }
  },

  selectCommit: async (hash) => {
    const d = get().detail;
    if (!d) return;
    // 펼치지 않고 커밋만 고르면 "이 파일"의 그 커밋 시점 diff를 본다.
    await get().selectCommitFile(hash, d.relPath);
  },

  selectCommitFile: async (hash, path) => {
    const d = get().detail;
    if (!d) return;
    const { root } = d;
    set((s) =>
      s.detail
        ? {
            detail: {
              ...s.detail,
              selectedCommit: hash,
              selectedCommitFile: path,
              commitDiff: undefined,
              commitDiffLoading: true,
            },
          }
        : s,
    );
    try {
      const res = await tauriApi.workdirDiffCommit(root, hash, path);
      set((s) =>
        s.detail && s.detail.selectedCommit === hash && s.detail.selectedCommitFile === path
          ? { detail: { ...s.detail, commitDiff: res, commitDiffLoading: false } }
          : s,
      );
    } catch (err) {
      console.warn("workdir: 커밋 diff 조회 실패", err);
      set((s) =>
        s.detail && s.detail.selectedCommit === hash
          ? { detail: { ...s.detail, commitDiffLoading: false } }
          : s,
      );
    }
  },

  toggleCommitExpand: async (hash) => {
    const d = get().detail;
    if (!d) return;
    // 이미 펼친 커밋을 다시 누르면 접는다.
    if (d.expandedCommit === hash) {
      set((s) => (s.detail ? { detail: { ...s.detail, expandedCommit: undefined } } : s));
      return;
    }
    const { root } = d;
    set((s) =>
      s.detail
        ? {
            detail: {
              ...s.detail,
              expandedCommit: hash,
              commitFiles: undefined,
              commitFilesLoading: true,
              commitFilesHasMore: false,
              commitFilesSkip: 0,
            },
          }
        : s,
    );
    try {
      const res = await tauriApi.workdirCommitFiles(root, hash, COMMIT_FILES_PAGE, 0);
      set((s) =>
        s.detail && s.detail.expandedCommit === hash
          ? {
              detail: {
                ...s.detail,
                commitFiles: res.files,
                commitFilesHasMore: res.hasMore,
                commitFilesLoading: false,
                commitFilesSkip: res.files.length,
              },
            }
          : s,
      );
    } catch (err) {
      console.warn("workdir: 커밋 변경파일 조회 실패", err);
      set((s) =>
        s.detail && s.detail.expandedCommit === hash
          ? { detail: { ...s.detail, commitFilesLoading: false } }
          : s,
      );
    }
  },

  loadMoreCommitFiles: async () => {
    const d = get().detail;
    if (!d || !d.expandedCommit || d.commitFilesLoading || !d.commitFilesHasMore) return;
    const { root, expandedCommit, commitFilesSkip } = d;
    set((s) => (s.detail ? { detail: { ...s.detail, commitFilesLoading: true } } : s));
    try {
      const res = await tauriApi.workdirCommitFiles(root, expandedCommit, COMMIT_FILES_PAGE, commitFilesSkip);
      set((s) =>
        s.detail && s.detail.expandedCommit === expandedCommit
          ? {
              detail: {
                ...s.detail,
                commitFiles: [...(s.detail.commitFiles ?? []), ...res.files],
                commitFilesHasMore: res.hasMore,
                commitFilesLoading: false,
                commitFilesSkip: commitFilesSkip + res.files.length,
              },
            }
          : s,
      );
    } catch (err) {
      console.warn("workdir: 커밋 변경파일 추가 조회 실패", err);
      set((s) =>
        s.detail && s.detail.expandedCommit === expandedCommit
          ? { detail: { ...s.detail, commitFilesLoading: false } }
          : s,
      );
    }
  },

  openDifftool: (commit) => {
    const d = get().detail;
    if (!d) return;
    // 하단 diff가 보고 있는 파일(없으면 이 상세 파일)을 대상으로.
    const rel = d.selectedCommitFile ?? d.relPath;
    void tauriApi
      .workdirDifftool(d.root, rel, d.diffMode, commit)
      .catch((err) => console.warn("외부 비교 도구 실행 실패", err));
  },

  // ================= 커밋 로그 브라우저(이슈 #54, 2단계) =================

  loadRepoLog: async (reset) => {
    const p = get().palette;
    if (!p) return;
    const root = p.root;
    const prev = get().repoLog[root] ?? emptyRepoLog(root);
    const skip = reset ? 0 : prev.loaded;
    const gen = reset ? prev.gen + 1 : prev.gen;
    setRepoLog(set, root, {
      ...prev,
      loading: true,
      ...(reset ? { commits: undefined, selectedCommit: undefined, files: undefined, selectedFile: undefined, fileDiff: undefined, gen } : {}),
    });
    try {
      const res = await tauriApi.workdirRepoLog(root, HISTORY_PAGE, skip, prev.allBranches, prev.query);
      const cur = get().repoLog[root];
      if (!cur || cur.gen !== gen) return; // stale(검색/브랜치 전환됨).
      const commits = reset ? res.commits : [...(cur.commits ?? []), ...res.commits];
      setRepoLog(set, root, {
        ...cur,
        commits,
        hasMore: res.hasMore,
        loaded: commits.length,
        timedOut: res.timedOut,
        loading: false,
      });
    } catch (err) {
      console.warn("workdir: 리포 로그 조회 실패", err);
      const cur = get().repoLog[root];
      if (cur && cur.gen === gen) setRepoLog(set, root, { ...cur, loading: false });
    }
  },

  setRepoLogQuery: (query) => {
    const p = get().palette;
    if (!p) return;
    const prev = get().repoLog[p.root] ?? emptyRepoLog(p.root);
    setRepoLog(set, p.root, { ...prev, query });
    void get().loadRepoLog(true);
  },

  setRepoLogAllBranches: (all) => {
    const p = get().palette;
    if (!p) return;
    const prev = get().repoLog[p.root] ?? emptyRepoLog(p.root);
    setRepoLog(set, p.root, { ...prev, allBranches: all });
    void get().loadRepoLog(true);
  },

  selectRepoCommit: async (hash) => {
    const p = get().palette;
    if (!p) return;
    const root = p.root;
    const prev = get().repoLog[root];
    if (!prev) return;
    setRepoLog(set, root, {
      ...prev,
      selectedCommit: hash,
      files: undefined,
      filesLoading: true,
      filesHasMore: false,
      filesLoaded: 0,
      selectedFile: undefined,
      fileDiff: undefined,
    });
    try {
      const res = await tauriApi.workdirCommitFiles(root, hash, COMMIT_FILES_PAGE, 0);
      const cur = get().repoLog[root];
      if (!cur || cur.selectedCommit !== hash) return;
      setRepoLog(set, root, {
        ...cur,
        files: res.files,
        filesHasMore: res.hasMore,
        filesLoaded: res.files.length,
        filesLoading: false,
      });
    } catch (err) {
      console.warn("workdir: 로그 커밋 변경파일 조회 실패", err);
      const cur = get().repoLog[root];
      if (cur && cur.selectedCommit === hash) setRepoLog(set, root, { ...cur, filesLoading: false });
    }
  },

  loadMoreRepoFiles: async () => {
    const p = get().palette;
    if (!p) return;
    const root = p.root;
    const prev = get().repoLog[root];
    if (!prev || !prev.selectedCommit || prev.filesLoading || !prev.filesHasMore) return;
    const hash = prev.selectedCommit;
    setRepoLog(set, root, { ...prev, filesLoading: true });
    try {
      const res = await tauriApi.workdirCommitFiles(root, hash, COMMIT_FILES_PAGE, prev.filesLoaded);
      const cur = get().repoLog[root];
      if (!cur || cur.selectedCommit !== hash) return;
      const files = [...(cur.files ?? []), ...res.files];
      setRepoLog(set, root, {
        ...cur,
        files,
        filesHasMore: res.hasMore,
        filesLoaded: files.length,
        filesLoading: false,
      });
    } catch (err) {
      console.warn("workdir: 로그 변경파일 추가 조회 실패", err);
      const cur = get().repoLog[root];
      if (cur && cur.selectedCommit === hash) setRepoLog(set, root, { ...cur, filesLoading: false });
    }
  },

  selectRepoFile: async (hash, path) => {
    const p = get().palette;
    if (!p) return;
    const root = p.root;
    const prev = get().repoLog[root];
    if (!prev) return;
    setRepoLog(set, root, { ...prev, selectedFile: path, fileDiff: undefined, fileDiffLoading: true });
    try {
      const res = await tauriApi.workdirDiffCommit(root, hash, path);
      const cur = get().repoLog[root];
      if (!cur || cur.selectedFile !== path || cur.selectedCommit !== hash) return;
      setRepoLog(set, root, { ...cur, fileDiff: res, fileDiffLoading: false });
    } catch (err) {
      console.warn("workdir: 로그 파일 diff 조회 실패", err);
      const cur = get().repoLog[root];
      if (cur && cur.selectedFile === path) setRepoLog(set, root, { ...cur, fileDiffLoading: false });
    }
  },

  openRepoDifftool: () => {
    const p = get().palette;
    if (!p) return;
    const rl = get().repoLog[p.root];
    if (!rl || !rl.selectedCommit || !rl.selectedFile) return;
    void tauriApi
      .workdirDifftool(p.root, rl.selectedFile, "worktreeVsHead", rl.selectedCommit)
      .catch((err) => console.warn("외부 비교 도구 실행 실패", err));
  },
}));

/** repoLog[root]를 갱신하는 헬퍼(레코드 불변 갱신). `set`은 zustand 스토어의
 *  실제 setState 타입을 그대로 받아 배리언스 문제 없이 넘길 수 있다. */
function setRepoLog(
  set: StoreApi<WorkdirState>["setState"],
  root: string,
  next: WorkdirRepoLog,
): void {
  set((s) => ({ repoLog: { ...s.repoLog, [root]: next } }));
}
