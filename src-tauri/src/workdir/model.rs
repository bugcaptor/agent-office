// src-tauri/src/workdir/model.rs
//
// workdir 서브시스템의 결과 타입들. `#[tauri::command]` 반환값이자 파서 계열
// 함수들의 출력이다. 모두 mod.rs에서 `pub use model::*;`로 재수출되어
// `crate::workdir::GitDiffResult` 같은 기존 경로를 그대로 유지한다.

/// 목록 결과. `truncated`는 상한(MAX_LIST)에 걸려 일부만 담겼음을 뜻한다.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkdirListResult {
    pub files: Vec<WorkdirFileEntry>,
    pub truncated: bool,
}

/// 목록 항목 하나. `rel_path`는 root 기준 상대경로(구분자 '/'로 정규화),
/// `name`은 파일명(마지막 경로 요소).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkdirFileEntry {
    pub rel_path: String,
    pub name: String,
}

/// 서버사이드 검색(Everything) 결과(이슈 #67). `usedIndex`가 false면
/// Everything을 시도하지 않았거나(Walker 백엔드/빈 쿼리) es.exe가 실패해
/// 폴백했다는 뜻 -- 이 경우 프런트는 `files`(빈 목록)를 무시하고 기존
/// 클라이언트 fuzzy 필터(이미 가져온 목록 내 검색)로 되돌아간다.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkdirSearchResult {
    pub files: Vec<WorkdirFileEntry>,
    pub truncated: bool,
    pub used_index: bool,
}

/// git 상태 파일 항목 하나. `path`는 저장소 루트 기준 상대경로(git이 준 그대로,
/// '/' 구분), `status`는 표시용 단일 문자 뱃지, `xy`는 porcelain v2 원문 2글자
/// (스테이지 X + 워킹트리 Y, 툴팁용).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileStatus {
    pub path: String,
    pub status: String,
    pub xy: String,
}

/// git 상태 조회 결과. git 저장소가 아니거나(is_repo=false) 타임아웃
/// (timed_out=true)이면 entries는 비어 있고 프런트는 조용히 뱃지를 생략한다.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatusResult {
    /// git 저장소 여부. git 바이너리 부재/비저장소 모두 false.
    pub is_repo: bool,
    /// 현재 브랜치명(detached HEAD면 None).
    pub branch: Option<String>,
    /// upstream 대비 앞선 커밋 수.
    pub ahead: i64,
    /// upstream 대비 뒤처진 커밋 수.
    pub behind: i64,
    pub entries: Vec<GitFileStatus>,
    /// 타임아웃으로 조회를 중단했는지.
    pub timed_out: bool,
    /// 엔트리 상한(MAX_STATUS_ENTRIES)에 걸려 일부만 담겼는지(이슈 #70).
    pub truncated: bool,
}

impl GitStatusResult {
    /// git 저장소가 아닐 때의 빈 응답.
    pub(super) fn not_repo() -> Self {
        Self {
            is_repo: false,
            branch: None,
            ahead: 0,
            behind: 0,
            entries: Vec::new(),
            timed_out: false,
            truncated: false,
        }
    }

    /// 타임아웃 응답(브랜치/엔트리 없이 플래그만).
    pub(super) fn timed_out() -> Self {
        Self {
            is_repo: true,
            branch: None,
            ahead: 0,
            behind: 0,
            entries: Vec::new(),
            timed_out: true,
            truncated: false,
        }
    }
}

/// diff 조회 결과. `diff`는 unified diff 텍스트(변경 없으면 빈 문자열),
/// `binary`는 git이 바이너리로 판단했는지, `truncated`는 상한(바이트/줄)에 걸려
/// 잘렸는지, `timed_out`은 타임아웃으로 조회를 중단했는지.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffResult {
    pub diff: String,
    pub binary: bool,
    pub truncated: bool,
    pub timed_out: bool,
}

/// 파일 히스토리 커밋 1건. `hash`는 full 40-hex, `short_hash`는 축약,
/// `author`/`date`/`subject`는 표시용.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitEntry {
    pub hash: String,
    pub short_hash: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

/// `git_file_history` 결과. `has_more`는 요청 limit만큼 다 채웠는지(더 있을 수
/// 있음), `timed_out`은 타임아웃 여부.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFileHistoryResult {
    pub commits: Vec<GitCommitEntry>,
    pub has_more: bool,
    pub timed_out: bool,
}

/// 한 커밋이 바꾼 파일 1건. `path`는 저장소 루트 기준 상대경로(rename이면 새
/// 경로), `status`는 표시용 단일 문자(M/A/D/R/C/T…).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitFileEntry {
    pub path: String,
    pub status: String,
}

/// `git_commit_files` 결과. `has_more`면 이 페이지 뒤로 파일이 더 남았음(페이징),
/// `timed_out`은 타임아웃 여부.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitCommitFilesResult {
    pub files: Vec<GitCommitFileEntry>,
    pub has_more: bool,
    pub timed_out: bool,
}
