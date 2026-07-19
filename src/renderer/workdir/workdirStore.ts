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
// 파일 열기: .md는 기존 인앱 편집기(markdownStore.openFile)로 위임하고, 그 외는
// 절대경로를 만들어 open_in_vscode로 외부 에디터에 넘긴다(읽기 전용 인앱 뷰어는
// 바이너리/인코딩/대용량 처리 수렁이라 MVP 범위에서 제외 — 후속 과제).
import { create } from "zustand";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { useMarkdownStore } from "../markdown/markdownStore";
import type { GitStatusResult, WorkdirFileEntry } from "@shared/types";

/** root별 파일 목록 캐시 1건. */
export interface WorkdirListing {
  files: WorkdirFileEntry[];
  truncated: boolean;
}

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
}

interface WorkdirState {
  palette: WorkdirPaletteState | null;
  /** root별 목록 캐시(재오픈 즉시 표시용, 런타임 전용). */
  listing: Record<string, WorkdirListing>;
  /** root별 git 상태 캐시(런타임 전용). */
  git: Record<string, GitStatusResult>;
  /** git 조회 진행 중 여부(root별). 헤더 스피너/상태 표시용. */
  gitLoading: Record<string, boolean>;

  /** 팔레트를 root로 연다(쿼리·선택 초기화) + 목록/ git 백그라운드 갱신. */
  openPalette(root: string, agentId: string): void;
  closePalette(): void;
  setQuery(query: string): void;
  setSelectedIndex(index: number): void;
  setChangedOnly(changedOnly: boolean): void;
  /** 파일 목록을 다시 읽어 캐시를 갱신한다(fire-and-forget 가능). */
  refreshListing(root: string): Promise<void>;
  /** git 상태를 다시 읽어 캐시를 갱신한다. 설정이 꺼져 있으면 캐시를 비운다. */
  refreshGit(root: string): Promise<void>;
  /** 항목을 연다: .md는 인앱 편집기, 그 외는 외부 에디터(open_in_vscode). */
  openEntry(root: string, relPath: string, name: string): void;
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

export const useWorkdirStore = create<WorkdirState>()((set, get) => ({
  palette: null,
  listing: {},
  git: {},
  gitLoading: {},

  openPalette: (root, agentId) => {
    set({ palette: { root, agentId, query: "", selectedIndex: 0, changedOnly: false } });
    // 캐시가 있으면 즉시 표시되고, 여기서 백그라운드 갱신.
    void get().refreshListing(root);
    void get().refreshGit(root);
  },

  closePalette: () => set({ palette: null }),

  setQuery: (query) =>
    set((s) => (s.palette ? { palette: { ...s.palette, query, selectedIndex: 0 } } : s)),

  setSelectedIndex: (index) =>
    set((s) => (s.palette ? { palette: { ...s.palette, selectedIndex: index } } : s)),

  setChangedOnly: (changedOnly) =>
    set((s) => (s.palette ? { palette: { ...s.palette, changedOnly, selectedIndex: 0 } } : s)),

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
      set({ palette: null });
      void useMarkdownStore.getState().openFile(root, relPath, agentId);
      return;
    }
    // 그 외 파일은 외부 에디터(VS Code 등)로 절대경로를 넘겨 연다.
    void tauriApi
      .openInVscode(joinPath(root, relPath))
      .catch((err) => console.warn(`파일 열기 실패: ${name}`, err));
  },
}));
