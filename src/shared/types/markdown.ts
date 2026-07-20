// src/shared/types/markdown.ts
//
// Domain slice: markdown document browsing/editing (이슈 #10).
// See src/shared/types.ts for the frozen-contract overview.

/**
 * 마크다운 문서 탐색·편집(이슈 #10)의 renderer<->backend 계약.
 * `version`은 렌더러가 해석하지 않는 불투명 토큰(백엔드가 발급, 낙관적 잠금용)이라
 * 왕복만 한다 — 값 형식(해시·mtime 등)은 백엔드 소관이므로 `string`으로만 다룬다.
 */
export interface MarkdownFileEntry {
  /** root 기준 상대 경로(POSIX 구분자). 목록/열기의 키. */
  relPath: string;
  /** 표시·퍼지 매칭 가중치용 파일명(경로 마지막 세그먼트). */
  name: string;
}

/** `markdown_list_files` 응답. `truncated`면 상한을 넘어 일부만 담겼다. */
export interface MarkdownListResult {
  files: MarkdownFileEntry[];
  truncated: boolean;
}

/** `markdown_read_file` 응답. `version`은 이후 쓰기의 `expectedVersion`으로 되돌려준다. */
export interface MarkdownReadResult {
  content: string;
  version: string;
}

/** `markdown_write_file` 응답. 저장 성공 시 갱신된 `version`을 돌려준다. */
export interface MarkdownWriteResult {
  version: string;
}
