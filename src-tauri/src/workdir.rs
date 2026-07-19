// src-tauri/src/workdir.rs
//
// 에이전트 작업 폴더(이슈 #11)를 앱에서 직접 들여다보기 위한 IPC 커맨드 2종
// (`list_workdir_files`/`workdir_git_status`)의 구현부. markdown.rs와 같은 골격
// -- `#[tauri::command]` 얇은 래퍼가 테스트 가능한 순수 함수에 위임하고, 에러는
// 사용자에게 그대로 보여줄 수 있는 한국어 문자열이다.
//
// `list_workdir_files`는 markdown.rs의 목록 스캐너를 확장자 필터만 빼고 그대로
// 재현한다: `ignore` 크레이트(WalkBuilder)로 .gitignore를 존중하고 hidden을
// 스킵하며 심링크는 따라가지 않고, MAX_LIST 상한에 걸리면 truncated=true.
//
// `workdir_git_status`는 시스템 `git`을 `status --porcelain=v2 --branch -z`로 딱
// 한 번 호출해 파일별 상태 뱃지와 브랜치 요약을 뽑는다. libgit2(git2 크레이트)를
// 쓰지 않는 이유: 의존성이 무겁고 거대 저장소에서 오히려 느릴 수 있어, 사용자
// 환경의 git 바이너리를 그대로 쓰는 편이 가볍고 예측 가능하다. "거대 저장소일 수
// 있다"는 이슈의 우려는 (1) 프런트/설정의 on/off 토글과 (2) 여기서 거는 타임아웃
// 가드 두 겹으로 막는다 -- 타임아웃을 넘기면 자식 프로세스를 죽이고 timed_out을
// 세워 정상 응답으로 돌려준다(에러가 아니라 "조회 시간 초과" 상태).
//
// git 바이너리 부재·비(非) git 저장소·타임아웃은 모두 에러가 아니라 정상 응답의
// 필드(is_repo=false / timed_out=true)로 표현한다 -- 작업 폴더 보기 자체는 git과
// 무관하게 항상 성공해야 하기 때문.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ignore::WalkBuilder;

/// 목록 결과 상한 -- 이 수에 도달하면 스캔을 멈추고 `truncated=true`.
const MAX_LIST: usize = 5000;

/// git status subprocess 타임아웃. 거대 저장소에서 UI가 멈추지 않도록 이 시간을
/// 넘기면 자식을 죽이고 `timed_out`을 세운다.
const GIT_STATUS_TIMEOUT: Duration = Duration::from_secs(3);

/// diff/log/show subprocess 타임아웃. status보다 넉넉하되(대용량 diff·긴 로그)
/// UI가 무한정 멈추지 않도록 상한을 둔다.
const GIT_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

/// diff 텍스트 상한(바이트). 이 크기를 넘으면 잘라내고 `truncated=true`.
const MAX_DIFF_BYTES: usize = 1024 * 1024;
/// diff 텍스트 상한(줄 수). 이 줄 수를 넘으면 잘라내고 `truncated=true`.
const MAX_DIFF_LINES: usize = 5000;
/// 파일 히스토리 조회 상한(요청 limit을 이 값으로 클램프).
const HISTORY_MAX_LIMIT: usize = 200;

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
}

impl GitStatusResult {
    /// git 저장소가 아닐 때의 빈 응답.
    fn not_repo() -> Self {
        Self {
            is_repo: false,
            branch: None,
            ahead: 0,
            behind: 0,
            entries: Vec::new(),
            timed_out: false,
        }
    }

    /// 타임아웃 응답(브랜치/엔트리 없이 플래그만).
    fn timed_out() -> Self {
        Self {
            is_repo: true,
            branch: None,
            ahead: 0,
            behind: 0,
            entries: Vec::new(),
            timed_out: true,
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

/// git 서브프로세스 1회 실행 결과(제네릭). `spawn_failed`는 git 바이너리 부재
/// 등 실행 자체 실패, `timed_out`은 타임아웃으로 kill, `success`는 exit 0 여부,
/// `stdout`은 종료 코드와 무관하게 리더 스레드가 끝까지 읽은 표준출력.
struct GitRun {
    spawn_failed: bool,
    timed_out: bool,
    success: bool,
    stdout: Vec<u8>,
}

/// git을 root에서 `args`로 한 번 실행한다. stdout은 별도 스레드로 끝까지 읽어
/// 파이프 교착을 막고(거대 diff는 수 MB), 타임아웃을 넘기면 자식을 죽인다.
/// stderr는 버린다(에러 메시지는 종료 코드/빈 stdout으로 판별).
fn run_git(root: &Path, args: &[&str], timeout: Duration) -> GitRun {
    let mut cmd = Command::new("git");
    cmd.current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            return GitRun {
                spawn_failed: true,
                timed_out: false,
                success: false,
                stdout: Vec::new(),
            }
        }
    };

    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return GitRun {
                spawn_failed: true,
                timed_out: false,
                success: false,
                stdout: Vec::new(),
            };
        }
    };
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        let _ = tx.send(buf);
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    let buf = rx.recv().unwrap_or_default();
    let _ = reader.join();

    match status {
        Some(s) => GitRun {
            spawn_failed: false,
            timed_out: false,
            success: s.success(),
            stdout: buf,
        },
        None => GitRun {
            spawn_failed: false,
            timed_out: true,
            success: false,
            stdout: buf,
        },
    }
}

/// git 커맨드에 넘길 상대경로를 검증·정규화한다. 절대경로·`..`·루트 컴포넌트를
/// 거부해 root 밖 접근을 막고, 반환값은 '/'로 정규화된 상대경로다. 이 값은 항상
/// `--` 뒤에 pathspec으로 넘겨(옵션 주입 차단) 선행 '-'가 있어도 안전하다.
fn sanitize_rel_path(rel: &str) -> Result<String, String> {
    if rel.is_empty() {
        return Err("경로가 비어 있습니다".to_string());
    }
    let p = Path::new(rel);
    let mut parts: Vec<String> = Vec::new();
    for comp in p.components() {
        use std::path::Component;
        match comp {
            Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("작업 폴더 밖의 경로는 접근할 수 없습니다: {rel}"));
            }
        }
    }
    if parts.is_empty() {
        return Err(format!("잘못된 경로입니다: {rel}"));
    }
    Ok(parts.join("/"))
}

/// 커밋 인자가 안전한지(hex 7~40자) 검증한다. git rev로 넘기기 전 옵션·경로
/// 주입을 원천 차단하기 위함.
fn valid_commit(commit: &str) -> bool {
    let n = commit.len();
    (7..=40).contains(&n) && commit.bytes().all(|b| b.is_ascii_hexdigit())
}

/// git diff 출력 바이트를 상한(바이트/줄)에 맞춰 잘라 GitDiffResult로 만든다.
/// 바이너리 판정은 git이 내는 "Binary files ..." / "GIT binary patch" 표식으로.
fn finalize_diff(bytes: &[u8]) -> GitDiffResult {
    let text = String::from_utf8_lossy(bytes);
    let binary = text.contains("Binary files ") || text.contains("GIT binary patch");
    let mut out = String::new();
    let mut truncated = false;
    for (idx, line) in text.lines().enumerate() {
        if idx >= MAX_DIFF_LINES || out.len() + line.len() + 1 > MAX_DIFF_BYTES {
            truncated = true;
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    GitDiffResult {
        diff: out,
        binary,
        truncated,
        timed_out: false,
    }
}

/// canonical root를 얻고 디렉터리인지 확인한다(diff/history 진입 공통 전처리).
fn canon_dir(root: &str) -> Result<std::path::PathBuf, String> {
    let canon_root = std::fs::canonicalize(root)
        .map_err(|e| format!("작업 폴더를 찾을 수 없습니다: {root} ({e})"))?;
    if !canon_root.is_dir() {
        return Err(format!("작업 폴더가 디렉터리가 아닙니다: {root}"));
    }
    Ok(canon_root)
}

/// root 기준 `rel_path`의 diff를 `mode`에 맞춰 뽑는다.
/// - `worktreeVsIndex`: 워킹트리↔인덱스(미스테이지 변경) `git diff`
/// - `indexVsHead`: 인덱스↔HEAD(스테이지된 변경) `git diff --cached`
/// - `worktreeVsHead`: 워킹트리↔HEAD(전체 변경 합본) `git diff HEAD`
/// - `untracked`: 미추적 파일을 새 파일로 `git diff --no-index /dev/null <path>`
///
/// 미추적 파일은 일반 `git diff`가 아무것도 내지 않으므로 `untracked` 모드가
/// 필요하다(프런트가 뱃지 '?'를 보고 이 모드로 요청).
pub fn git_diff_file(root: &str, rel_path: &str, mode: &str) -> Result<GitDiffResult, String> {
    let canon_root = canon_dir(root)?;
    let rel = sanitize_rel_path(rel_path)?;
    let args: Vec<&str> = match mode {
        "worktreeVsIndex" => vec!["diff", "--", &rel],
        "indexVsHead" => vec!["diff", "--cached", "--", &rel],
        "worktreeVsHead" => vec!["diff", "HEAD", "--", &rel],
        // /dev/null은 git이 diff --no-index에서 크로스플랫폼으로 특별 처리한다.
        "untracked" => vec!["diff", "--no-index", "--", "/dev/null", &rel],
        other => return Err(format!("알 수 없는 diff 모드: {other}")),
    };
    let run = run_git(&canon_root, &args, GIT_QUERY_TIMEOUT);
    if run.spawn_failed {
        return Err("git 실행에 실패했습니다(설치 여부 확인)".to_string());
    }
    if run.timed_out {
        return Ok(GitDiffResult {
            diff: String::new(),
            binary: false,
            truncated: false,
            timed_out: true,
        });
    }
    // diff --no-index는 차이가 있으면 exit 1을 내지만 정상 출력이다. 그래서
    // success와 무관하게 stdout을 그대로 파싱한다(exit 2 등 에러도 빈 stdout이면
    // 빈 diff로 귀결되어 UI에 "변경 없음"으로 보인다).
    Ok(finalize_diff(&run.stdout))
}

/// root 기준 `rel_path`의 커밋 히스토리를 `git log --follow`로 가져온다(페이지네이션).
pub fn git_file_history(
    root: &str,
    rel_path: &str,
    limit: usize,
    skip: usize,
) -> Result<GitFileHistoryResult, String> {
    let canon_root = canon_dir(root)?;
    let rel = sanitize_rel_path(rel_path)?;
    let limit = limit.clamp(1, HISTORY_MAX_LIMIT);
    // 필드 구분 US(0x1f), 레코드 구분은 -z의 NUL. subject에 개행이 있어도 안전.
    let n_arg = format!("-n{limit}");
    let skip_arg = format!("--skip={skip}");
    let args = [
        "log",
        "--follow",
        "--date=format:%Y-%m-%d %H:%M",
        "--format=%H%x1f%h%x1f%an%x1f%ad%x1f%s",
        "-z",
        &n_arg,
        &skip_arg,
        "--",
        &rel,
    ];
    let run = run_git(&canon_root, &args, GIT_QUERY_TIMEOUT);
    if run.spawn_failed {
        return Err("git 실행에 실패했습니다(설치 여부 확인)".to_string());
    }
    if run.timed_out {
        return Ok(GitFileHistoryResult {
            commits: Vec::new(),
            has_more: false,
            timed_out: true,
        });
    }
    Ok(parse_history(&run.stdout, limit))
}

/// `git log ... -z --format=%H\x1f%h\x1f%an\x1f%ad\x1f%s` 출력을 파싱한다.
/// 레코드는 NUL로 구분되며 각 레코드 안은 US(0x1f)로 5개 필드가 나뉜다.
fn parse_history(bytes: &[u8], limit: usize) -> GitFileHistoryResult {
    let mut commits = Vec::new();
    for rec in bytes.split(|&b| b == 0) {
        // 레코드 사이 개행이 섞일 수 있어 앞뒤 공백/개행을 다듬는다.
        let s = String::from_utf8_lossy(rec);
        let s = s.trim_matches(['\n', '\r']);
        if s.is_empty() {
            continue;
        }
        let mut f = s.splitn(5, '\u{1f}');
        let hash = f.next().unwrap_or("").to_string();
        let short_hash = f.next().unwrap_or("").to_string();
        let author = f.next().unwrap_or("").to_string();
        let date = f.next().unwrap_or("").to_string();
        let subject = f.next().unwrap_or("").to_string();
        if hash.is_empty() {
            continue;
        }
        commits.push(GitCommitEntry {
            hash,
            short_hash,
            author,
            date,
            subject,
        });
    }
    let has_more = commits.len() >= limit;
    GitFileHistoryResult {
        commits,
        has_more,
        timed_out: false,
    }
}

/// 특정 커밋이 `rel_path`에 만든 변경(diff)을 `git show`로 가져온다. `--format=`로
/// 커밋 메시지 헤더를 지워 diff만 남긴다.
pub fn git_diff_commit(root: &str, commit: &str, rel_path: &str) -> Result<GitDiffResult, String> {
    let canon_root = canon_dir(root)?;
    if !valid_commit(commit) {
        return Err(format!("잘못된 커밋 해시입니다: {commit}"));
    }
    let rel = sanitize_rel_path(rel_path)?;
    let args = ["show", "--format=", commit, "--", &rel];
    let run = run_git(&canon_root, &args, GIT_QUERY_TIMEOUT);
    if run.spawn_failed {
        return Err("git 실행에 실패했습니다(설치 여부 확인)".to_string());
    }
    if run.timed_out {
        return Ok(GitDiffResult {
            diff: String::new(),
            binary: false,
            truncated: false,
            timed_out: true,
        });
    }
    Ok(finalize_diff(&run.stdout))
}

/// 외부 비교 도구(`git difftool`)를 fire-and-forget으로 띄운다. GUI 도구가
/// 설정돼 있어야 의미가 있고, 미설정이면 백그라운드에서 조용히 실패한다(인앱
/// diff가 항상 폴백이므로 여기서는 spawn 성공만 확인). `commit`이 있으면 그
/// 커밋의 변경을, 없으면 `mode`에 따른 현재 변경을 연다.
pub fn launch_difftool(
    root: &str,
    rel_path: &str,
    mode: &str,
    commit: Option<&str>,
) -> Result<(), String> {
    let canon_root = canon_dir(root)?;
    let rel = sanitize_rel_path(rel_path)?;
    let mut args: Vec<String> = vec!["difftool".to_string(), "-y".to_string()];
    match commit {
        Some(c) => {
            if !valid_commit(c) {
                return Err(format!("잘못된 커밋 해시입니다: {c}"));
            }
            // "<hash>^!"는 그 커밋 하나의 변경(부모↔커밋)을 뜻한다.
            args.push(format!("{c}^!"));
        }
        None => match mode {
            "indexVsHead" => args.push("--cached".to_string()),
            "worktreeVsHead" => args.push("HEAD".to_string()),
            // worktreeVsIndex(기본)·untracked 등은 추가 rev 없이 워킹트리 비교.
            _ => {}
        },
    }
    args.push("--".to_string());
    args.push(rel);

    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut cmd = Command::new("git");
    cmd.current_dir(&canon_root)
        .args(&arg_refs)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("외부 비교 도구 실행 실패: {e}"))
}

/// root 아래 파일을 스캔한다. markdown.rs의 스캐너와 동일하되 확장자 필터가
/// 없다: `.gitignore`를 존중하고 hidden을 스킵하며 심링크는 따라가지 않는다.
pub fn list_workdir_files(root: &str) -> Result<WorkdirListResult, String> {
    let canon_root = std::fs::canonicalize(root)
        .map_err(|e| format!("작업 폴더를 찾을 수 없습니다: {root} ({e})"))?;
    if !canon_root.is_dir() {
        return Err(format!("작업 폴더가 디렉터리가 아닙니다: {root}"));
    }

    let mut builder = WalkBuilder::new(&canon_root);
    builder
        .follow_links(false) // 심링크는 따라가지 않는다(root 밖 유출 방지).
        .hidden(true) // 숨김 파일/폴더 스킵.
        .git_ignore(true) // .gitignore 존중.
        .require_git(false); // .git이 없어도 .gitignore를 적용.

    let mut files = Vec::new();
    let mut truncated = false;

    for entry in builder.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue, // 개별 항목 접근 오류는 조용히 건너뛴다.
        };
        // 파일만(디렉터리·심링크 등 제외). file_type은 root 자체엔 없을 수 있다.
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        let Ok(rel) = path.strip_prefix(&canon_root) else {
            continue; // root 하위가 아니면(있을 수 없지만) 스킵.
        };
        let rel_path = normalize_separators(rel);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        files.push(WorkdirFileEntry { rel_path, name });

        if files.len() >= MAX_LIST {
            truncated = true;
            break;
        }
    }

    // relPath 오름차순 정렬(스캔 순서는 비결정적이므로 안정적 출력을 위해).
    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(WorkdirListResult { files, truncated })
}

/// 경로 구분자를 '/'로 정규화한다(Windows의 '\\' → '/').
fn normalize_separators(rel: &Path) -> String {
    rel.components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect::<Vec<_>>()
        .join("/")
}

/// root의 git 상태를 조회한다. 저장소가 아니거나 git이 없으면 is_repo=false,
/// 타임아웃이면 timed_out=true인 정상 응답을 돌려준다(에러 문자열은 root가 아예
/// 없는 등 조회 이전 단계 실패에서만 반환).
pub fn collect_git_status(root: &str) -> Result<GitStatusResult, String> {
    let canon_root = std::fs::canonicalize(root)
        .map_err(|e| format!("작업 폴더를 찾을 수 없습니다: {root} ({e})"))?;
    if !canon_root.is_dir() {
        return Err(format!("작업 폴더가 디렉터리가 아닙니다: {root}"));
    }
    Ok(run_git_status(&canon_root, GIT_STATUS_TIMEOUT))
}

/// `git status --porcelain=v2 --branch -z`를 root에서 실행하고 결과를 파싱한다.
/// 타임아웃 초과 시 자식을 죽이고 timed_out 응답을 돌려준다. 실행/파이프 처리는
/// 공용 `run_git`에 위임한다.
fn run_git_status(root: &Path, timeout: Duration) -> GitStatusResult {
    let run = run_git(
        root,
        &["status", "--porcelain=v2", "--branch", "-z"],
        timeout,
    );
    // git 바이너리 부재 등 -- 저장소 아님으로 취급(뱃지 조용히 생략).
    if run.spawn_failed {
        return GitStatusResult::not_repo();
    }
    // 타임아웃.
    if run.timed_out {
        return GitStatusResult::timed_out();
    }
    if run.success {
        // exit 0: 정상 파싱.
        parse_porcelain_v2(&run.stdout)
    } else {
        // non-zero: 비 git 저장소(혹은 기타 git 에러) -- 뱃지 생략.
        GitStatusResult::not_repo()
    }
}

/// `git status --porcelain=v2 --branch -z` 출력을 파싱한다. 레코드는 NUL로
/// 구분되며, rename(type 2) 레코드만 예외적으로 경로 뒤에 원본경로가 NUL로 한
/// 필드 더 붙는다 -- 그래서 토큰을 순회하며 type 2를 만나면 다음 토큰 하나를
/// 원본경로로 소비한다.
///
/// 참고 포맷:
/// - `# branch.head <name>` / `# branch.ab +N -M`
/// - `1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>`           (일반 변경)
/// - `2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <Xscore> <path>`  (rename/copy; +원본경로)
/// - `u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>` (충돌)
/// - `? <path>`  (untracked)  /  `! <path>` (ignored; 스킵)
pub fn parse_porcelain_v2(bytes: &[u8]) -> GitStatusResult {
    let mut result = GitStatusResult {
        is_repo: true,
        branch: None,
        ahead: 0,
        behind: 0,
        entries: Vec::new(),
        timed_out: false,
    };

    let tokens: Vec<&[u8]> = bytes
        .split(|&b| b == 0)
        .filter(|t| !t.is_empty())
        .collect();

    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        match tok.first() {
            Some(b'#') => {
                let line = String::from_utf8_lossy(tok);
                if let Some(rest) = line.strip_prefix("# branch.head ") {
                    let name = rest.trim();
                    // detached HEAD는 "(detached)" 라고 나온다 -- 브랜치 없음.
                    result.branch = if name == "(detached)" || name.is_empty() {
                        None
                    } else {
                        Some(name.to_string())
                    };
                } else if let Some(rest) = line.strip_prefix("# branch.ab ") {
                    // "+N -M" 형태.
                    let mut parts = rest.split_whitespace();
                    if let Some(a) = parts.next() {
                        result.ahead = a.trim_start_matches('+').parse().unwrap_or(0);
                    }
                    if let Some(b) = parts.next() {
                        result.behind = b.trim_start_matches('-').parse().unwrap_or(0);
                    }
                }
            }
            Some(b'1') | Some(b'u') => {
                if let Some((xy, path)) = parse_changed_entry(tok) {
                    result.entries.push(make_status(xy, path));
                }
            }
            Some(b'2') => {
                if let Some((xy, path)) = parse_changed_entry(tok) {
                    result.entries.push(make_status(xy, path));
                }
                // rename/copy는 다음 토큰이 원본경로 -- 소비만 하고 버린다.
                i += 1;
            }
            // "? <path>": 앞 2바이트("? ") 제거. 경로가 없으면(있을 수 없지만) 스킵.
            Some(b'?') if tok.len() > 2 => {
                let path = String::from_utf8_lossy(&tok[2..]).into_owned();
                result.entries.push(GitFileStatus {
                    path,
                    status: "?".to_string(),
                    xy: "??".to_string(),
                });
            }
            // '!'(ignored) 및 알 수 없는 라인은 스킵.
            _ => {}
        }
        i += 1;
    }

    result
}

/// type 1/2/u 레코드에서 (XY 2글자, 경로)를 뽑는다. 경로는 공백을 포함할 수
/// 있으므로 "마지막 필드"로 취급한다. type 2는 XY 뒤 필드 수가 하나 더(Xscore)
/// 많지만, "경로 = 마지막 공백 이후 전체"라 필드 개수와 무관하게 처리된다.
fn parse_changed_entry(tok: &[u8]) -> Option<(String, String)> {
    let s = String::from_utf8_lossy(tok);
    let mut parts = s.splitn(3, ' ');
    let _kind = parts.next()?; // '1' | '2' | 'u'
    let xy = parts.next()?; // "MD" 등 2글자
    let rest = parts.next()?; // "<sub> ... <path>"
    // 경로는 마지막 공백 이후 전체. rsplit 한 번으로 뒤 필드만 떼면 경로 중간의
    // 공백이 보존된다: rest = "N... <path>" 에서 rsplitn(?, ' ')는 부적절하므로,
    // 필드 개수만큼 앞에서 건너뛴다.
    let path = skip_fixed_fields(rest, xy.as_bytes(), tok.first())?;
    Some((xy.to_string(), path))
}

/// `rest`(= XY 다음부터)에서 고정 메타 필드를 건너뛰고 경로만 돌려준다.
/// 고정 필드 개수: type 1 → 6(sub,mH,mI,mW,hH,hI), type 2 → 7(+Xscore),
/// type u → 8(sub,m1,m2,m3,mW,h1,h2,h3). 경로는 그 뒤 전체(공백 포함).
fn skip_fixed_fields(rest: &str, _xy: &[u8], kind: Option<&u8>) -> Option<String> {
    let fixed = match kind {
        Some(b'1') => 6,
        Some(b'2') => 7,
        Some(b'u') => 8,
        _ => return None,
    };
    // fixed개 필드를 공백으로 건너뛰고 나머지 전부를 경로로.
    let mut it = rest.splitn(fixed + 1, ' ');
    for _ in 0..fixed {
        it.next()?;
    }
    let path = it.next()?;
    if path.is_empty() {
        None
    } else {
        Some(path.to_string())
    }
}

/// XY(스테이지 X + 워킹트리 Y)에서 표시용 단일 뱃지 문자를 고른다: 워킹트리
/// 쪽(Y)이 변경돼 있으면 Y, 아니면 스테이지 쪽(X). 충돌(u 레코드)은 XY가 둘 다
/// 알파벳이라 그대로 첫 글자가 잡히지만, 표시는 'U'로 통일한다.
fn make_status(xy: String, path: String) -> GitFileStatus {
    let x = xy.chars().next().unwrap_or('.');
    let y = xy.chars().nth(1).unwrap_or('.');
    // 충돌 상태(양쪽 다 대문자이고 unmerged 조합)는 'U'로.
    let is_conflict = matches!(
        (x, y),
        ('D', 'D') | ('A', 'A') | ('U', _) | (_, 'U')
    );
    let status = if is_conflict {
        'U'
    } else if y != '.' {
        y
    } else {
        x
    };
    GitFileStatus {
        path,
        status: status.to_string(),
        xy,
    }
}

/// `list_workdir_files`의 Tauri 커맨드 래퍼. 시작 폴더 UI가 `~/dev/foo`류
/// 입력을 허용하므로 세션 생성과 동일한 틸드 확장을 거친다(open_in_vscode 관례).
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_list_files(root: String) -> Result<WorkdirListResult, String> {
    list_workdir_files(&crate::session::manager::expand_tilde(root))
}

/// `collect_git_status`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_git_status(root: String) -> Result<GitStatusResult, String> {
    collect_git_status(&crate::session::manager::expand_tilde(root))
}

/// `git_diff_file`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_diff_file(
    root: String,
    rel_path: String,
    mode: String,
) -> Result<GitDiffResult, String> {
    git_diff_file(
        &crate::session::manager::expand_tilde(root),
        &rel_path,
        &mode,
    )
}

/// `git_file_history`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_file_history(
    root: String,
    rel_path: String,
    limit: usize,
    skip: usize,
) -> Result<GitFileHistoryResult, String> {
    git_file_history(
        &crate::session::manager::expand_tilde(root),
        &rel_path,
        limit,
        skip,
    )
}

/// `git_diff_commit`의 Tauri 커맨드 래퍼.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_diff_commit(
    root: String,
    commit: String,
    rel_path: String,
) -> Result<GitDiffResult, String> {
    git_diff_commit(
        &crate::session::manager::expand_tilde(root),
        &commit,
        &rel_path,
    )
}

/// `launch_difftool`의 Tauri 커맨드 래퍼. `commit`이 빈 문자열/미지정이면 현재
/// 변경을, 아니면 그 커밋의 변경을 외부 도구로 연다.
#[tauri::command(rename_all = "camelCase")]
pub async fn workdir_difftool(
    root: String,
    rel_path: String,
    mode: String,
    commit: Option<String>,
) -> Result<(), String> {
    let commit_ref = commit.as_deref().filter(|c| !c.is_empty());
    launch_difftool(
        &crate::session::manager::expand_tilde(root),
        &rel_path,
        &mode,
        commit_ref,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 토큰들을 NUL로 이어 porcelain -z 출력 바이트를 만든다(끝에도 NUL).
    fn nul_join(tokens: &[&str]) -> Vec<u8> {
        let mut v = Vec::new();
        for t in tokens {
            v.extend_from_slice(t.as_bytes());
            v.push(0);
        }
        v
    }

    #[test]
    fn parses_branch_and_ab() {
        let bytes = nul_join(&[
            "# branch.oid abc123",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -3",
        ]);
        let r = parse_porcelain_v2(&bytes);
        assert!(r.is_repo);
        assert_eq!(r.branch.as_deref(), Some("main"));
        assert_eq!(r.ahead, 2);
        assert_eq!(r.behind, 3);
        assert!(r.entries.is_empty());
    }

    #[test]
    fn detached_head_has_no_branch() {
        let bytes = nul_join(&["# branch.head (detached)", "# branch.ab +0 -0"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.branch, None);
    }

    #[test]
    fn parses_ordinary_modified_entry() {
        // 워킹트리 수정(스테이지 안 됨): XY = ".M".
        let bytes = nul_join(&["1 .M N... 100644 100644 100644 aaa bbb src/lib.rs"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].path, "src/lib.rs");
        assert_eq!(r.entries[0].status, "M");
        assert_eq!(r.entries[0].xy, ".M");
    }

    #[test]
    fn staged_added_uses_x_when_worktree_clean() {
        // 스테이지된 추가(워킹트리 클린): XY = "A.".
        let bytes = nul_join(&["1 A. N... 000000 100644 100644 000 bbb new.txt"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries[0].status, "A");
        assert_eq!(r.entries[0].xy, "A.");
    }

    #[test]
    fn path_with_spaces_is_preserved() {
        let bytes = nul_join(&["1 .M N... 100644 100644 100644 aaa bbb my dir/a b.txt"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries[0].path, "my dir/a b.txt");
        assert_eq!(r.entries[0].status, "M");
    }

    #[test]
    fn rename_entry_consumes_orig_path_token() {
        // type 2 뒤에는 원본경로 토큰이 하나 더 온다. 그 뒤 일반 엔트리가
        // 정상적으로 이어져야 파싱 오프셋이 맞는 것.
        let bytes = nul_join(&[
            "2 R. N... 100644 100644 100644 aaa bbb R100 new/name.rs",
            "old/name.rs",
            "1 .M N... 100644 100644 100644 ccc ddd other.rs",
        ]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries.len(), 2);
        assert_eq!(r.entries[0].path, "new/name.rs");
        assert_eq!(r.entries[0].status, "R");
        assert_eq!(r.entries[1].path, "other.rs");
        assert_eq!(r.entries[1].status, "M");
    }

    #[test]
    fn untracked_and_ignored() {
        let bytes = nul_join(&["? untracked.txt", "! ignored.txt"]);
        let r = parse_porcelain_v2(&bytes);
        // untracked만 잡히고 ignored는 스킵.
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].path, "untracked.txt");
        assert_eq!(r.entries[0].status, "?");
        assert_eq!(r.entries[0].xy, "??");
    }

    #[test]
    fn unmerged_entry_maps_to_u() {
        // 충돌: u 레코드, XY = "UU".
        let bytes =
            nul_join(&["u UU N... 100644 100644 100644 100644 aaa bbb ccc conflict.rs"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries.len(), 1);
        assert_eq!(r.entries[0].path, "conflict.rs");
        assert_eq!(r.entries[0].status, "U");
    }

    #[test]
    fn deleted_entry() {
        // 워킹트리 삭제: XY = " D" 아님 -- v2는 ".D".
        let bytes = nul_join(&["1 .D N... 100644 100644 000000 aaa bbb gone.rs"]);
        let r = parse_porcelain_v2(&bytes);
        assert_eq!(r.entries[0].status, "D");
    }

    #[test]
    fn empty_output_is_clean_repo() {
        let r = parse_porcelain_v2(&[]);
        assert!(r.is_repo);
        assert!(r.entries.is_empty());
        assert_eq!(r.branch, None);
    }

    #[test]
    fn nonexistent_root_is_error() {
        assert!(collect_git_status("/definitely/not/a/dir/xyzzy").is_err());
        assert!(list_workdir_files("/definitely/not/a/dir/xyzzy").is_err());
    }

    /// 실제 이 저장소에서 git status를 호출하는 스모크(호스트 git 검증용).
    #[test]
    fn this_repo_is_detected_as_git() {
        let root = env!("CARGO_MANIFEST_DIR");
        let r = collect_git_status(root).unwrap();
        assert!(r.is_repo, "이 크레이트는 git 저장소여야 함");
        assert!(!r.timed_out);
    }

    #[test]
    fn sanitize_rel_path_rejects_escapes() {
        assert!(sanitize_rel_path("").is_err());
        assert!(sanitize_rel_path("/etc/passwd").is_err());
        assert!(sanitize_rel_path("../secret").is_err());
        assert!(sanitize_rel_path("a/../../b").is_err());
        // 정상: 정규화되어 '/' 구분.
        assert_eq!(sanitize_rel_path("src/lib.rs").unwrap(), "src/lib.rs");
        assert_eq!(sanitize_rel_path("./src/./lib.rs").unwrap(), "src/lib.rs");
        // 선행 '-' 파일명은 통과한다(항상 `--` 뒤 pathspec으로 넘겨 안전).
        assert_eq!(sanitize_rel_path("-weird.txt").unwrap(), "-weird.txt");
    }

    #[test]
    fn valid_commit_accepts_only_hex_7_to_40() {
        assert!(valid_commit("dd7c2d8"));
        assert!(valid_commit("dd7c2d861e6c0619e58bed7340efebe2ae7915db"));
        assert!(!valid_commit("dd7c2d")); // 6자
        assert!(!valid_commit("HEAD"));
        assert!(!valid_commit("dd7c2d8; rm -rf /"));
        assert!(!valid_commit("../../etc"));
        // 41자 초과.
        assert!(!valid_commit("dd7c2d861e6c0619e58bed7340efebe2ae7915db0"));
    }

    #[test]
    fn finalize_diff_flags_binary() {
        let d = finalize_diff(b"Binary files a/x.png and b/x.png differ\n");
        assert!(d.binary);
        assert!(!d.truncated);
    }

    #[test]
    fn finalize_diff_truncates_by_lines() {
        let mut big = String::new();
        for i in 0..(MAX_DIFF_LINES + 100) {
            big.push_str(&format!("+line {i}\n"));
        }
        let d = finalize_diff(big.as_bytes());
        assert!(d.truncated);
        assert_eq!(d.diff.lines().count(), MAX_DIFF_LINES);
    }

    #[test]
    fn parse_history_splits_records_and_fields() {
        // 필드 US(0x1f), 레코드 NUL. 2개 커밋.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(
            "abc1234def5678\u{1f}abc1234\u{1f}Alice\u{1f}2026-01-01 10:00\u{1f}first subject"
                .as_bytes(),
        );
        bytes.push(0);
        bytes.extend_from_slice(
            "9990000aaaa\u{1f}9990000\u{1f}Bob\u{1f}2026-02-02 11:11\u{1f}second".as_bytes(),
        );
        bytes.push(0);
        let r = parse_history(&bytes, 50);
        assert_eq!(r.commits.len(), 2);
        assert_eq!(r.commits[0].hash, "abc1234def5678");
        assert_eq!(r.commits[0].short_hash, "abc1234");
        assert_eq!(r.commits[0].author, "Alice");
        assert_eq!(r.commits[0].date, "2026-01-01 10:00");
        assert_eq!(r.commits[0].subject, "first subject");
        assert_eq!(r.commits[1].author, "Bob");
        assert!(!r.has_more); // 2 < 50
    }

    #[test]
    fn parse_history_has_more_when_full() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice("aaaaaaa\u{1f}aaaaaaa\u{1f}A\u{1f}d\u{1f}s".as_bytes());
        bytes.push(0);
        let r = parse_history(&bytes, 1);
        assert_eq!(r.commits.len(), 1);
        assert!(r.has_more); // 1 >= limit(1)
    }

    #[test]
    fn unknown_diff_mode_is_error() {
        let root = env!("CARGO_MANIFEST_DIR");
        assert!(git_diff_file(root, "Cargo.toml", "bogus").is_err());
    }

    #[test]
    fn diff_out_of_root_is_error() {
        let root = env!("CARGO_MANIFEST_DIR");
        assert!(git_diff_file(root, "../../../etc/passwd", "worktreeVsHead").is_err());
        assert!(git_file_history(root, "../escape", 10, 0).is_err());
        assert!(git_diff_commit(root, "0000000", "../escape").is_err());
    }

    #[test]
    fn diff_commit_rejects_bad_hash() {
        let root = env!("CARGO_MANIFEST_DIR");
        // 유효하지 않은 해시는 sanitize 이전에 거부.
        assert!(git_diff_commit(root, "not-a-hash", "Cargo.toml").is_err());
    }

    /// 실제 저장소에서 파일 히스토리를 조회하는 스모크. workdir.rs는 커밋
    /// 이력이 있으므로 최소 1건 이상 나와야 한다.
    #[test]
    fn this_repo_file_history_smoke() {
        let root = env!("CARGO_MANIFEST_DIR");
        let r = git_file_history(root, "src/workdir.rs", 10, 0).unwrap();
        assert!(!r.timed_out);
        assert!(
            !r.commits.is_empty(),
            "workdir.rs는 커밋 이력이 있어야 함"
        );
        // 해시는 hex 40자여야 한다.
        assert!(valid_commit(&r.commits[0].hash));
    }

    /// 특정 커밋(#11 도입 커밋)의 workdir.rs diff를 뽑는 스모크.
    #[test]
    fn this_repo_diff_commit_smoke() {
        let root = env!("CARGO_MANIFEST_DIR");
        let r =
            git_diff_commit(root, "dd7c2d861e6c0619e58bed7340efebe2ae7915db", "src/workdir.rs")
                .unwrap();
        assert!(!r.timed_out);
        // 그 커밋이 workdir.rs를 새로 추가했으므로 diff에 파일 헤더가 있어야 한다.
        assert!(r.diff.contains("diff --git") || r.diff.contains("new file"));
    }
}
