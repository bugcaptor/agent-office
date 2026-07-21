// src-tauri/src/file_index/mod.rs
//
// Everything(es.exe) 백엔드(이슈 #67 Stage 4·서버사이드 검색은 Stage 후속) --
// markdown.rs::list_markdown_files와 workdir 팔레트의 서버사이드 검색이 함께
// 쓰는 옵트인 스캔 경로. 설정 `fileIndexBackend == everything`일 때만 시도되고,
// es.exe 부재/실패/타임아웃이면 조용히 `None`을 돌려줘 호출부가 기존
// 워커(WalkBuilder, file_scan::walk_files) 경로로 폴백하게 한다.
//
// - `es_runner`: es.exe 서브프로세스 실행(Windows 전용, CREATE_NO_WINDOW).
// - `gitignore_filter`: es.exe가 준 "부분일치 후보"에 WalkBuilder와 동등한
//   gitignore/숨김 규칙을 적용하는 순수 함수(es.exe 없이 테스트 가능,
//   WalkBuilder를 정답 오라클로 삼은 등가성 테스트 포함).

mod es_runner;
mod gitignore_filter;

use std::path::{Path, PathBuf};

use crate::file_scan::ScannedFile;

/// Windows `std::fs::canonicalize`가 붙이는 확장길이(verbatim) 프리픽스
/// `\\?\`(UNC는 `\\?\UNC\`)를 제거해 "평범한" 절대경로로 만든다. es.exe의
/// `-path` 필터는 인덱스에 `C:\...` 형태로 저장된 경로를 매칭하므로 `\\?\C:\...`
/// 를 그대로 넘기면 **0건**이 나오고(형제 필터 `starts_with`도 프리픽스 불일치로
/// 전부 탈락), 팔레트가 빈 목록이 된다. 그래서 Everything 경로 전체를 이
/// 평범한 경로로 일관되게 흘려보낸다. 비-Windows는 그대로 통과.
fn strip_verbatim_prefix(p: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = p.as_os_str().to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    p.to_path_buf()
}

/// Everything 백엔드로 root 아래 마크다운 파일을 스캔한다. es.exe 후보 조회나
/// `.gitignore` 목록 조회 중 하나라도 실패하면(`None`) 전체를 폴백 신호로
/// 돌려준다(부분적으로 필터 없이 후보만 반환하는 것은 gitignore 누락으로 이어질
/// 위험이 있어 하지 않는다 -- 안전한 쪽은 "이번엔 워커로").
///
/// canon_root는 보통 verbatim 프리픽스가 붙은 상태로 들어오므로, es.exe 쿼리·
/// 후보 필터·gitignore 매칭 전 구간을 평범한 절대경로로 정규화해 사용한다.
pub fn list_markdown_files_via_everything(canon_root: &Path) -> Option<(Vec<ScannedFile>, bool)> {
    let root = strip_verbatim_prefix(canon_root);
    let candidates = es_runner::find_markdown_candidates(&root)?;
    let gitignore_files = es_runner::find_gitignore_files(&root)?;
    Some(gitignore_filter::build_result(
        &root,
        &gitignore_files,
        candidates,
    ))
}

/// Everything 백엔드로 root 아래에서 `user_query`와 일치하는 파일을 검색한다
/// (이슈 #67 workdir 팔레트 서버사이드 검색). `list_markdown_files_via_everything`과
/// 대칭 구조 -- 후보 조회·`.gitignore` 목록 조회 중 하나라도 실패하면(`None`)
/// 전체를 폴백 신호로 돌려준다. `user_query`가 비어 있으면(공백뿐)
/// `find_files_matching`이 `None`을 주므로 여기서도 `None`이 된다(호출부가
/// "빈 쿼리"와 "es.exe 실패"를 같은 폴백 경로로 처리).
pub fn search_files_via_everything(
    canon_root: &Path,
    user_query: &str,
) -> Option<(Vec<ScannedFile>, bool)> {
    let root = strip_verbatim_prefix(canon_root);
    let candidates = es_runner::find_files_matching(&root, user_query)?;
    let gitignore_files = es_runner::find_gitignore_files(&root)?;
    Some(gitignore_filter::build_result(
        &root,
        &gitignore_files,
        candidates,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// es.exe가 설치된 머신에서만 의미 있는 실측 테스트: **실제 저장소**(인덱싱된
    /// 디스크 경로)에 대해 Everything 백엔드가 워커와 동일한 md 목록을 돌려주는지
    /// 확인한다. 과거에 Windows `canonicalize`의 `\\?\` 프리픽스를 es.exe에
    /// 그대로 넘겨 **0건**을 반환하던 회귀가 있었고(폴백도 안 타 팔레트가 빈
    /// 목록이 됨), 이 테스트가 그 회귀를 잡는다.
    ///
    /// es.exe가 없는 CI/개발 머신에서는 `list_markdown_files_via_everything`가
    /// `None`(폴백)을 주므로 조용히 건너뛴다 -- 즉 "es.exe 있으면 반드시
    /// 워커와 일치"라는 조건부 계약을 검증한다.
    #[test]
    fn everything_matches_walker_on_real_repo_when_es_available() {
        // CARGO_MANIFEST_DIR = .../agent-office/src-tauri → 부모 = 저장소 루트.
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let canon = std::fs::canonicalize(&repo_root).unwrap();

        let Some((files, _truncated)) = list_markdown_files_via_everything(&canon) else {
            eprintln!("es.exe 미설치/미가용 -- Everything 실측 테스트 건너뜀");
            return;
        };

        let walker = crate::markdown::list_markdown_files(canon.to_str().unwrap()).unwrap();
        let mut es_rel: Vec<_> = files.iter().map(|f| f.rel_path.clone()).collect();
        es_rel.sort();
        let mut walker_rel: Vec<_> = walker.files.iter().map(|f| f.rel_path.clone()).collect();
        walker_rel.sort();

        // 워커가 md를 찾는데 Everything이 0건이면 예전 `\\?\` 회귀다.
        assert!(
            walker_rel.is_empty() || !es_rel.is_empty(),
            "워커는 md {}개를 찾았는데 Everything은 0개 -- es.exe 경로 회귀 의심",
            walker_rel.len()
        );
        assert_eq!(
            es_rel, walker_rel,
            "Everything 백엔드 결과가 실제 저장소에서 워커와 달라야 함(회귀)"
        );
    }
}
