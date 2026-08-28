// src/renderer/workdir/workdirTypes.ts
//
// "작업 폴더 보기" 스토어의 **상태 모양**과 축 경계.
//
// 액션 구현은 축마다 별도 파일(`slices/`)에 있고, 여기 있는 것은 그 셋이
// 공유하는 타입과 경로 유틸뿐이다. 세 슬라이스는 전부 같은 `WorkdirState`를
// 보므로 서로의 액션을 `get()`으로 부를 수 있다 — 축을 나눈 것은 파일을
// 나누기 위해서지 상태를 격리하기 위해서가 아니다.
import type { StateCreator } from "zustand";
import type {
  GitCommitEntry,
  GitCommitFileEntry,
  GitDiffMode,
  GitDiffResult,
  GitStatusResult,
  WorkdirFileEntry,
} from "@shared/types";

export interface WorkdirListing {
  files: WorkdirFileEntry[];
  truncated: boolean;
  /** 이 캐시가 채워진 시각(Date.now()) — TTL 판정·"N분 전" 표시에 쓰인다. */
  fetchedAt: number;
  /** 이 캐시를 만든 스캔이 무시·숨김 파일까지 담았는지. 지금 설정과 다르면
   *  TTL이 남아 있어도 캐시를 못 쓴다(다른 조건으로 뜬 목록이므로). */
  includeIgnored: boolean;
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
  /** 진행 중인 diff 조회의 opId(취소 버튼이 쓴다). undefined = 진행 중 아님. */
  diffOpId?: string;
  history?: GitCommitEntry[];
  historyLoading: boolean;
  /** 진행 중인 히스토리 조회의 opId. */
  historyOpId?: string;
  /** 히스토리 조회가 취소로 끝났는지("다시 시도" 안내 — 빈 목록과 구분). */
  historyCanceled: boolean;
  historyHasMore: boolean;
  /** 히스토리에서 선택해 하단 diff를 보고 있는 커밋 해시. */
  selectedCommit?: string;
  /** 하단 diff가 보여주는 파일 경로. 기본은 이 상세의 파일(relPath)이지만,
   *  커밋을 펼쳐 다른 파일을 고르면 그 파일 경로가 된다(이슈 #54). */
  selectedCommitFile?: string;
  commitDiff?: GitDiffResult;
  commitDiffLoading: boolean;
  /** 진행 중인 커밋 diff 조회의 opId. */
  commitDiffOpId?: string;
  /** 변경파일 목록을 인라인으로 펼친 커밋 해시(이슈 #54). undefined = 안 펼침. */
  expandedCommit?: string;
  /** 펼친 커밋의 변경파일 목록(페이징 누적). */
  commitFiles?: GitCommitFileEntry[];
  commitFilesLoading: boolean;
  /** 진행 중인 변경파일 조회의 opId. */
  commitFilesOpId?: string;
  /** 변경파일 조회가 취소로 끝났는지(빈 목록과 구분해 "다시 시도"를 띄운다). */
  commitFilesCanceled: boolean;
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
  /** 진행 중인 로그 조회의 opId(취소 버튼용). */
  opId?: string;
  hasMore: boolean;
  /** 이미 로드한 커밋 수(다음 페이지 skip). */
  loaded: number;
  timedOut: boolean;
  /** 로그 조회가 취소로 끝났는지("다시 시도" 안내). */
  canceled: boolean;
  /** 선택된 커밋(그 커밋의 변경파일 목록을 로드). */
  selectedCommit?: string;
  files?: GitCommitFileEntry[];
  filesLoading: boolean;
  /** 진행 중인 변경파일 조회의 opId. */
  filesOpId?: string;
  /** 변경파일 조회가 취소로 끝났는지. */
  filesCanceled: boolean;
  filesHasMore: boolean;
  filesLoaded: number;
  /** 변경파일 중 선택돼 diff를 보고 있는 파일 경로. */
  selectedFile?: string;
  fileDiff?: GitDiffResult;
  fileDiffLoading: boolean;
  /** 진행 중인 파일 diff 조회의 opId. */
  fileDiffOpId?: string;
  /** 조회 세대 카운터(검색/브랜치 전환 시 증가 → stale 응답 폐기). */
  gen: number;
}

/** 서버사이드(Everything) 검색 결과(이슈 #67). `root`/`query`가 현재 팔레트와
 *  일치할 때만 유효 — 팔레트 컴포넌트가 이 둘을 대조해 stale 응답을 걸러낸다. */
export interface WorkdirSearchState {
  root: string;
  query: string;
  files: WorkdirFileEntry[];
  truncated: boolean;
}

/** 팔레트·목록·검색·git 상태 축. */
export interface WorkdirPaletteSlice {
  palette: WorkdirPaletteState | null;
  /** root별 목록 캐시(재오픈 즉시 표시용, 런타임 전용). */
  listing: Record<string, WorkdirListing>;
  /** 서버사이드(Everything) 검색 결과(이슈 #67). null = 비활성 — 팔레트는
   *  기존 클라이언트 fuzzy 필터로 표시한다. `setQuery`가 디바운스 후 채운다. */
  search: WorkdirSearchState | null;
  /** 서버사이드 검색 조회 진행 중(디바운스 대기~응답 도착까지) 여부. 기존
   *  결과는 유지한 채 팔레트가 옅게 "검색 중…"을 얹는 용도. */
  searchLoading: boolean;
  /** root별 git 상태 캐시(런타임 전용). */
  git: Record<string, GitStatusResult>;
  /** git 조회 진행 중 여부(root별). 헤더 스피너/상태 표시용. */
  gitLoading: Record<string, boolean>;
  /** 진행 중인 git status 조회의 opId(root별). 헤더 "취소" 버튼이 쓴다. */
  gitOpId: Record<string, string>;

  /** 팔레트를 root로 연다(쿼리·선택 초기화) + 목록/ git 백그라운드 갱신. */
  openPalette(root: string, agentId: string): void;
  closePalette(): void;
  /** 팔레트 쿼리를 갱신한다. 백엔드가 Everything이고 "files" 뷰·"전체"
   *  필터(!changedOnly)·쿼리 2글자 이상이면 250ms 디바운스 후 서버사이드
   *  검색(`search`)을 시도한다(이슈 #67) — 조건 미달이면 `search`를 비운다. */
  setQuery(query: string): void;
  setSelectedIndex(index: number): void;
  setChangedOnly(changedOnly: boolean): void;
  /** 파일 목록 뷰 ↔ 커밋 로그 브라우저 뷰 전환(이슈 #54). log 최초 진입 시 로드. */
  setViewMode(mode: WorkdirViewMode): void;
  /** 파일 목록을 다시 읽어 캐시를 갱신한다(fire-and-forget 가능). 기본은 TTL을
   *  따라 캐시가 신선하면(5분 이내) 스킵한다. `force: true`는 TTL을 무시하고
   *  항상 스캔한다(수동 새로고침 버튼용). 같은 root에 대해 이미 진행 중이면
   *  중복 실행하지 않는다(in-flight dedupe). */
  refreshListing(root: string, opts?: { force?: boolean }): Promise<void>;
  /** git 상태를 다시 읽어 캐시를 갱신한다. 설정이 꺼져 있으면 캐시를 비운다.
   *  같은 root의 조회가 이미 진행 중이면 스킵한다 — 타임아웃이 분 단위라
   *  중복 호출이 쌓이면 git 프로세스가 겹겹이 남는다. */
  refreshGit(root: string): Promise<void>;
  /** 진행 중인 git 조회를 취소한다(fire-and-forget). `opId`가 없으면 no-op. */
  cancelOp(opId?: string): void;
  /** 빠른 열기(⌘-클릭/더블클릭): .md는 인앱 편집기, 그 외는 외부 에디터. */
  openEntry(root: string, relPath: string, name: string): void;
}

/** 우측 상세 페인(변경점/히스토리) 축. */
export interface WorkdirDetailSlice {
  /** 우측 상세 페인(변경점/히스토리). null = 목록만. */
  detail: WorkdirDetail | null;

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
}

/** 저장소 전체 커밋 로그 브라우저 축(이슈 #54, 2단계). */
export interface WorkdirRepoLogSlice {
  /** root별 커밋 로그 브라우저 상태(이슈 #54, 런타임 전용 캐시). */
  repoLog: Record<string, WorkdirRepoLog>;

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

export type WorkdirState = WorkdirPaletteSlice & WorkdirDetailSlice & WorkdirRepoLogSlice;

/** 슬라이스 하나의 생성자 시그니처. 셋 다 같은 `WorkdirState`를 보므로
 *  `get()`으로 다른 축의 액션을 그대로 부를 수 있다(뷰 전환이 로그를
 *  로드하고, 팔레트를 닫을 때 상세·로그의 진행 중 조회를 끊는 식). */
export type WorkdirSlice<T> = StateCreator<WorkdirState, [], [], T>;

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
