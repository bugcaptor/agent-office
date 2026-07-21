// src-tauri/src/file_index/mod.rs
//
// Everything(es.exe) 백엔드(이슈 #67 Stage 4) -- markdown.rs::list_markdown_files
// 전용 옵트인 스캔 경로. 설정 `fileIndexBackend == everything`일 때만 시도되고,
// es.exe 부재/실패/타임아웃이면 조용히 `None`을 돌려줘 호출부가 기존
// 워커(WalkBuilder, file_scan::walk_files) 경로로 폴백하게 한다.
//
// - `es_runner`: es.exe 서브프로세스 실행(Windows 전용, CREATE_NO_WINDOW).
// - `gitignore_filter`: es.exe가 준 "부분일치 후보"에 WalkBuilder와 동등한
//   gitignore/숨김 규칙을 적용하는 순수 함수(es.exe 없이 테스트 가능,
//   WalkBuilder를 정답 오라클로 삼은 등가성 테스트 포함).

mod es_runner;
mod gitignore_filter;

use std::path::Path;

use crate::file_scan::ScannedFile;

/// Everything 백엔드로 root 아래 마크다운 파일을 스캔한다. es.exe 후보 조회나
/// `.gitignore` 목록 조회 중 하나라도 실패하면(`None`) 전체를 폴백 신호로
/// 돌려준다(부분적으로 필터 없이 후보만 반환하는 것은 gitignore 누락으로 이어질
/// 위험이 있어 하지 않는다 -- 안전한 쪽은 "이번엔 워커로").
pub fn list_markdown_files_via_everything(canon_root: &Path) -> Option<(Vec<ScannedFile>, bool)> {
    let candidates = es_runner::find_markdown_candidates(canon_root)?;
    let gitignore_files = es_runner::find_gitignore_files(canon_root)?;
    Some(gitignore_filter::build_result(
        canon_root,
        &gitignore_files,
        candidates,
    ))
}
