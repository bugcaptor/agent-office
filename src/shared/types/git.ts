// src/shared/types/git.ts
//
// Domain slice: workdir file browsing (이슈 #11) and git status/diff/history
// (이슈 #11, #54). See src/shared/types.ts for the frozen-contract overview.

/** 작업 폴더 보기(이슈 #11)의 파일 목록 항목. `relPath`가 목록/열기의 키. */
export interface WorkdirFileEntry {
  /** root 기준 상대 경로(POSIX 구분자). */
  relPath: string;
  /** 표시·퍼지 매칭용 파일명(경로 마지막 세그먼트). */
  name: string;
}

/** `workdir_list_files` 응답. `truncated`면 상한(5000)을 넘어 일부만 담겼다. */
export interface WorkdirListResult {
  files: WorkdirFileEntry[];
  truncated: boolean;
}

/** 파일 하나의 git 상태. `status`는 표시용 단일 문자(M/A/D/R/U/? 등),
 * `xy`는 porcelain v2 원문 2글자(스테이지 X + 워킹트리 Y, 툴팁용). */
export interface GitFileStatus {
  /** 저장소 루트 기준 상대 경로(POSIX 구분자). */
  path: string;
  status: string;
  xy: string;
}

/** `workdir_git_status` 응답. 저장소가 아니거나(isRepo=false) 타임아웃
 * (timedOut=true)이면 entries는 비고 프런트는 뱃지를 조용히 생략한다. */
export interface GitStatusResult {
  isRepo: boolean;
  /** 현재 브랜치명(detached HEAD면 null). */
  branch: string | null;
  ahead: number;
  behind: number;
  entries: GitFileStatus[];
  timedOut: boolean;
}

/** `workdir_diff_file`의 diff 관점(이슈 #11 후속).
 * - `worktreeVsIndex`: 미스테이지 변경(워킹트리↔인덱스)
 * - `indexVsHead`: 스테이지된 변경(인덱스↔HEAD)
 * - `worktreeVsHead`: 전체 변경 합본(워킹트리↔HEAD) — 기본
 * - `untracked`: 미추적 파일을 새 파일로(`git diff --no-index`) */
export type GitDiffMode = "worktreeVsIndex" | "indexVsHead" | "worktreeVsHead" | "untracked";

/** `workdir_diff_file`/`workdir_diff_commit` 응답. `diff`가 빈 문자열이면 변경
 * 없음. `binary`면 텍스트 diff 불가, `truncated`면 상한(1MiB·5000줄)에 걸려
 * 잘렸고, `timedOut`이면 조회가 시간 초과됐다. */
export interface GitDiffResult {
  diff: string;
  binary: boolean;
  truncated: boolean;
  timedOut: boolean;
}

/** 파일 히스토리 커밋 1건. `hash`는 full 40-hex, `shortHash`는 축약. */
export interface GitCommitEntry {
  hash: string;
  shortHash: string;
  author: string;
  date: string;
  subject: string;
}

/** `workdir_file_history`/`workdir_repo_log` 응답. `hasMore`면 요청 limit을 다
 * 채워 더 있을 수 있다. */
export interface GitFileHistoryResult {
  commits: GitCommitEntry[];
  hasMore: boolean;
  timedOut: boolean;
}

/** 한 커밋이 바꾼 파일 1건(이슈 #54). `path`는 root 기준 상대경로(rename이면 새
 * 경로), `status`는 표시용 단일 문자(M/A/D/R/C/T 등). */
export interface GitCommitFileEntry {
  path: string;
  status: string;
}

/** `workdir_commit_files` 응답. `hasMore`면 이 페이지 뒤로 파일이 더 남았다. */
export interface GitCommitFilesResult {
  files: GitCommitFileEntry[];
  hasMore: boolean;
  timedOut: boolean;
}
