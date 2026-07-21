// src-tauri/src/file_index/gitignore_filter.rs
//
// es.exe가 돌려준 후보 목록에 gitignore/숨김 규칙을 적용해 `ignore::WalkBuilder`
// (markdown.rs::list_markdown_files, file_scan::walk_files)와 동일한 결과를
// 만드는 순수 함수. es.exe 없이도(테스트에서) 동작해야 하므로 입력은
// `.gitignore` 파일 경로 목록 + 후보 파일 경로 목록뿐이다.
//
// 알고리즘 개요(자세한 근거는 이슈 #67 설계 노트):
// 1. `.gitignore` 파일들을 디렉터리 깊이 오름차순으로 처리해 디렉터리별
//    매처(Gitignore) 맵을 만든다. 이미 무시된(조상 어딘가가 ignore) 디렉터리
//    안의 `.gitignore`는 스킵한다 -- git이 무시된 디렉터리 내부를 읽지 않는
//    것과 동일.
// 2. root 디렉터리는 항상 매처를 하나 갖는다(root/.gitignore가 있으면 추가,
//    root/.git/info/exclude가 있으면 추가) -- 그래야 최상위 규칙이 다른
//    `.gitignore` 존재 여부와 무관하게 항상 적용된다.
// 3. 각 후보에 대해 (a) hidden 컴포넌트 없음, (b) 조상 디렉터리 중 ignore된
//    것이 없음(디렉터리 가지치기), (c) 후보 자신이 ignore 아님을 모두
//    만족해야 포함.
// 4. 포함된 항목을 relPath 오름차순 정렬한다. MAX_LIST 절단은 `build_result`가
//    담당(순수 필터 함수 자체는 절단하지 않아 등가성 테스트에서 전체 집합을
//    비교할 수 있게 한다).

use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::Match;

use crate::file_scan::{ScannedFile, MAX_LIST};

/// 디렉터리별 매처. 삽입 순서는 항상 얕은 depth -> 깊은 depth로 유지한다
/// (뒤에 온 것이 더 깊은 규칙 = git 우선순위와 동일하게 마지막 non-None이
/// 승리하게 하기 위함).
struct MatcherMap(Vec<(PathBuf, Gitignore)>);

impl MatcherMap {
    /// path(파일 또는 디렉터리) 자체가 이 맵의 매처들로 ignore되는지. path의
    /// "조상"인 매처 디렉터리만 평가 대상 -- path와 매처 디렉터리가 같으면
    /// (자기 자신의 .gitignore가 자신을 가리키는 경우는 없으므로) 건너뛴다.
    fn is_ignored_at(&self, path: &Path, is_dir: bool) -> bool {
        let mut verdict = false;
        for (dir, matcher) in &self.0 {
            if path == dir || !path.starts_with(dir) {
                continue;
            }
            let Ok(rel) = path.strip_prefix(dir) else {
                continue;
            };
            match matcher.matched(rel, is_dir) {
                Match::None => {}
                Match::Ignore(_) => verdict = true,
                Match::Whitelist(_) => verdict = false,
            }
        }
        verdict
    }

    /// dir 자신부터 canon_root 바로 아래까지의 디렉터리 체인 중 하나라도
    /// ignore면 true(가지치기 -- git은 무시된 디렉터리 안을 보지 않는다).
    fn chain_ignored(&self, canon_root: &Path, dir: &Path) -> bool {
        let mut cur = Some(dir);
        while let Some(d) = cur {
            if d == canon_root {
                break;
            }
            if self.is_ignored_at(d, true) {
                return true;
            }
            cur = d.parent();
        }
        false
    }
}

/// path(캐노니컬, canon_root 하위)의 root 이후 컴포넌트 중 `.`로 시작하는
/// 것이 있는지(숨김 파일/폴더, `.git` 포함).
fn has_hidden_component(rel: &Path) -> bool {
    rel.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| s.starts_with('.'))
            .unwrap_or(false)
    })
}

fn normalize_separators(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// `gitignore_files`(깊이 무관 순서로 들어와도 됨)로 디렉터리별 매처 맵을
/// 만든다. root 매처는 항상 생성(있으면 root/.gitignore + root/.git/info/exclude
/// 반영). 이미 무시된 디렉터리 안의 `.gitignore`는 건너뛴다.
fn build_matchers(canon_root: &Path, gitignore_files: &[PathBuf]) -> MatcherMap {
    let mut matchers: Vec<(PathBuf, Gitignore)> = Vec::new();

    // root 매처는 항상 먼저 만든다 -- 다른 .gitignore 존재 여부와 무관하게
    // root/.gitignore·root/.git/info/exclude를 적용하기 위함.
    {
        let mut builder = GitignoreBuilder::new(canon_root);
        let root_gitignore = canon_root.join(".gitignore");
        if root_gitignore.is_file() {
            let _ = builder.add(&root_gitignore);
        }
        let exclude = canon_root.join(".git").join("info").join("exclude");
        if exclude.is_file() {
            let _ = builder.add(&exclude);
        }
        if let Ok(m) = builder.build() {
            matchers.push((canon_root.to_path_buf(), m));
        }
    }

    // 깊이(컴포넌트 수) 오름차순 -- 얕은 디렉터리의 매처가 먼저 map에 들어가야
    // 그 아래 .gitignore의 "이미 무시됐나" 판정이 올바르게 그 매처들을 본다.
    let mut sorted: Vec<&PathBuf> = gitignore_files.iter().collect();
    sorted.sort_by_key(|p| p.components().count());

    for g in sorted {
        let Some(dir) = g.parent() else { continue };
        if dir == canon_root {
            continue; // 위에서 이미 처리.
        }
        if !dir.starts_with(canon_root) {
            continue; // canon_root 밖 -- 있을 수 없지만 방어적으로 스킵.
        }
        let map_so_far = MatcherMap(matchers.clone());
        if map_so_far.chain_ignored(canon_root, dir) {
            continue; // git은 무시된 디렉터리 안의 .gitignore를 읽지 않는다.
        }
        let mut builder = GitignoreBuilder::new(dir);
        let _ = builder.add(g);
        if let Ok(m) = builder.build() {
            matchers.push((dir.to_path_buf(), m));
        }
    }

    MatcherMap(matchers)
}

/// es.exe 후보 + `.gitignore` 목록으로 `ignore::WalkBuilder`와 동등한 결과를
/// 만든다(정렬은 하되 MAX_LIST 절단은 하지 않음 -- 등가성 테스트가 전체
/// 집합을 비교할 수 있도록). 반환은 (rel_path, name) 목록.
pub fn filter_candidates(
    canon_root: &Path,
    gitignore_files: &[PathBuf],
    candidates: Vec<PathBuf>,
) -> Vec<(String, String)> {
    let matchers = build_matchers(canon_root, gitignore_files);

    let mut out = Vec::new();
    for f in candidates {
        if f == canon_root || !f.starts_with(canon_root) {
            continue;
        }
        let Ok(rel) = f.strip_prefix(canon_root) else {
            continue;
        };
        if has_hidden_component(rel) {
            continue;
        }
        if let Some(parent) = f.parent() {
            if matchers.chain_ignored(canon_root, parent) {
                continue;
            }
        }
        if matchers.is_ignored_at(&f, false) {
            continue;
        }
        let rel_path = normalize_separators(rel);
        let name = f
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        out.push((rel_path, name));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `filter_candidates` + MAX_LIST 절단. `markdown.rs`의 Everything 백엔드가
/// `MarkdownListResult`로 감싸기 좋은 형태(파일 목록, truncated)로 쓴다.
pub fn build_result(
    canon_root: &Path,
    gitignore_files: &[PathBuf],
    candidates: Vec<PathBuf>,
) -> (Vec<ScannedFile>, bool) {
    let mut filtered = filter_candidates(canon_root, gitignore_files, candidates);
    let truncated = filtered.len() > MAX_LIST;
    filtered.truncate(MAX_LIST);
    let files = filtered
        .into_iter()
        .map(|(rel_path, name)| ScannedFile { rel_path, name })
        .collect();
    (files, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// canon_root 아래(.git 제외) 모든 파일을 전수 열거해 (모든 후보,
    /// .gitignore 목록)을 돌려준다 -- es.exe를 대신하는 테스트 전용 열거자.
    /// filter_candidates가 이 "정제 전" 원시 목록에서 WalkBuilder와 동일한
    /// 결과를 뽑아내야 등가성이 성립한다.
    fn enumerate_all(root: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
        let mut candidates = Vec::new();
        let mut gitignores = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let file_type = entry.file_type().unwrap();
                if file_type.is_dir() {
                    if path.file_name().and_then(|n| n.to_str()) == Some(".git") {
                        continue; // .git 디렉터리 자체는 열거 대상에서 제외.
                    }
                    stack.push(path);
                } else if file_type.is_file() {
                    if path.file_name().and_then(|n| n.to_str()) == Some(".gitignore") {
                        gitignores.push(path.clone());
                    }
                    candidates.push(path);
                }
            }
        }
        (candidates, gitignores)
    }

    fn md_only(candidates: Vec<PathBuf>) -> Vec<PathBuf> {
        candidates
            .into_iter()
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect()
    }

    /// filter_candidates 결과와 markdown::list_markdown_files(WalkBuilder,
    /// 정답 오라클) 결과의 rel_path 집합이 완전히 같은지 단언한다.
    fn assert_equivalent_to_walkbuilder(root: &std::path::Path) {
        let canon_root = fs::canonicalize(root).unwrap();
        let (all_candidates, gitignores) = enumerate_all(&canon_root);
        let md_candidates = md_only(all_candidates);

        let filtered = filter_candidates(&canon_root, &gitignores, md_candidates);
        let mut ours: Vec<String> = filtered.into_iter().map(|(rel, _)| rel).collect();
        ours.sort();

        let oracle = crate::markdown::list_markdown_files(canon_root.to_str().unwrap()).unwrap();
        let mut theirs: Vec<String> = oracle.files.into_iter().map(|f| f.rel_path).collect();
        theirs.sort();

        assert_eq!(
            ours, theirs,
            "gitignore_filter 결과가 WalkBuilder(list_markdown_files)와 달라야 함:\nours={ours:?}\ntheirs={theirs:?}"
        );
    }

    #[test]
    fn equivalence_flat_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "ignored.md\n").unwrap();
        fs::write(root.join("visible.md"), "x").unwrap();
        fs::write(root.join("ignored.md"), "x").unwrap();
        assert_equivalent_to_walkbuilder(root);
    }

    #[test]
    fn equivalence_nested_gitignore_overrides_parent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "*.md\n").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        // 자식 .gitignore가 부모 규칙을 재포함(negate)한다.
        fs::write(root.join("sub").join(".gitignore"), "!kept.md\n").unwrap();
        fs::write(root.join("sub").join("kept.md"), "x").unwrap();
        fs::write(root.join("sub").join("dropped.md"), "x").unwrap();
        fs::write(root.join("top-dropped.md"), "x").unwrap();
        assert_equivalent_to_walkbuilder(root);
    }

    #[test]
    fn equivalence_whitelist_reinclusion() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "*.md\n!keep-me.md\n").unwrap();
        fs::write(root.join("keep-me.md"), "x").unwrap();
        fs::write(root.join("drop-me.md"), "x").unwrap();
        assert_equivalent_to_walkbuilder(root);
    }

    #[test]
    fn equivalence_directory_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "build/\n").unwrap();
        fs::create_dir(root.join("build")).unwrap();
        fs::write(root.join("build").join("inside.md"), "x").unwrap();
        fs::create_dir(root.join("build").join("nested")).unwrap();
        fs::write(root.join("build").join("nested").join("deep.md"), "x").unwrap();
        fs::write(root.join("normal.md"), "x").unwrap();
        assert_equivalent_to_walkbuilder(root);
    }

    #[test]
    fn equivalence_hidden_files_and_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".hidden.md"), "x").unwrap();
        fs::create_dir(root.join(".hiddendir")).unwrap();
        fs::write(root.join(".hiddendir").join("inside.md"), "x").unwrap();
        fs::write(root.join("visible.md"), "x").unwrap();
        assert_equivalent_to_walkbuilder(root);
    }

    #[test]
    fn equivalence_gitignore_inside_ignored_dir_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "ignored-dir/\n").unwrap();
        fs::create_dir(root.join("ignored-dir")).unwrap();
        // 이 하위 .gitignore가 재포함을 시도해도, 디렉터리 자체가 이미
        // 무시됐으므로 git은 이 파일을 읽지 않는다(등가성 테스트의 핵심 케이스).
        fs::write(root.join("ignored-dir").join(".gitignore"), "!reincluded.md\n").unwrap();
        fs::write(root.join("ignored-dir").join("reincluded.md"), "x").unwrap();
        fs::write(root.join("ignored-dir").join("other.md"), "x").unwrap();
        fs::write(root.join("kept.md"), "x").unwrap();
        assert_equivalent_to_walkbuilder(root);
    }

    #[test]
    fn equivalence_subfolder_normal_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("a/b/c")).unwrap();
        fs::write(root.join("a/b/c/deep.md"), "x").unwrap();
        fs::write(root.join("top.md"), "x").unwrap();
        assert_equivalent_to_walkbuilder(root);
    }

    #[test]
    fn equivalence_mixed_realistic_tree() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "node_modules/\n*.log.md\n!important.log.md\n").unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        fs::write(root.join("node_modules").join("pkg.md"), "x").unwrap();
        fs::write(root.join("debug.log.md"), "x").unwrap();
        fs::write(root.join("important.log.md"), "x").unwrap();
        fs::create_dir(root.join("docs")).unwrap();
        fs::write(root.join("docs").join(".gitignore"), "draft.md\n").unwrap();
        fs::write(root.join("docs").join("draft.md"), "x").unwrap();
        fs::write(root.join("docs").join("readme.md"), "x").unwrap();
        fs::create_dir(root.join("docs").join(".secret")).unwrap();
        fs::write(root.join("docs").join(".secret").join("x.md"), "x").unwrap();
        assert_equivalent_to_walkbuilder(root);
    }

    #[test]
    fn build_result_truncates_and_flags() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let canon_root = fs::canonicalize(root).unwrap();
        let mut candidates = Vec::new();
        for i in 0..(MAX_LIST + 5) {
            let name = format!("f{i:05}.md");
            fs::write(root.join(&name), "x").unwrap();
            candidates.push(canon_root.join(&name));
        }
        let (files, truncated) = build_result(&canon_root, &[], candidates);
        assert!(truncated);
        assert_eq!(files.len(), MAX_LIST);
    }
}
