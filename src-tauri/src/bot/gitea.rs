// src-tauri/src/bot/gitea.rs
//
// 봇 모드의 Gitea 접근 계층. 앱은 Gitea 토큰을 직접 보관하지 않고 `tea` CLI에
// 위임한다(`tea api`가 tea 로그인 자격으로 임의 Gitea REST를 호출) — "로컬에서
// 접근 가능한 계정 토큰"이 그대로 명령·게시 주체가 되는 이슈 #57의 보안 모델.
// docs/bot-mode-design.md 참고.
//
// 앱은 **읽기 전용 폴링**만 한다: 이슈/댓글 조회, 계정 확인, PR head 조회(완료
// 판정). 접수/진행/완료 댓글과 PR 생성 등 모든 쓰기는 에이전트가 프롬프트
// 지시에 따라 `tea`로 직접 수행한다.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// tea/git 호출 기본 타임아웃. 네트워크 왕복이 있어 diff 조회(workdir)보다 넉넉히.
pub(crate) const TEA_TIMEOUT: Duration = Duration::from_secs(20);

/// 서브프로세스 1회 실행 결과. `run_git`(workdir.rs)과 같은 구조지만 stderr도
/// 보존한다 — tea는 미로그인/401 진단을 stderr로 내보내기 때문.
struct Run {
    spawn_failed: bool,
    timed_out: bool,
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// `program`을 `args`로 한 번 실행한다. stdout/stderr는 각각 별도 스레드로 끝까지
/// 읽어 파이프 교착을 막고, 타임아웃을 넘기면 자식을 죽인다. workdir.rs의
/// `run_git` 리더-스레드 패턴을 미러링한다. `cwd`가 있으면 그 디렉터리에서 실행.
fn run(program: &str, args: &[&str], cwd: Option<&Path>, timeout: Duration) -> Run {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // C 로케일 강제. GUI(Finder/launchd)로 띄운 앱은 셸 프로파일을 거치지 않아
    // LANG/LC_* 가 비어, git이 시스템 로케일(예: 한국어)로 오류를 낸다. tea는
    // git의 영어 "not a git repository" 문자열을 매치해 "여기는 저장소 아님 →
    // 기본 로그인으로 폴백"을 판정하므로, 로케일이 비면 폴백이 깨져 `tea api`가
    // 통째로 실패한다(사용자에겐 "tea 로그인 실패"로 보임). LC_ALL=C로 git/tea
    // 메시지를 영어로 고정해 파싱을 결정화한다. tea가 뱉는 JSON 본문은 HTTP
    // 원문 바이트라 로케일 영향을 받지 않는다.
    cmd.env("LC_ALL", "C");
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => {
            return Run {
                spawn_failed: true,
                timed_out: false,
                success: false,
                stdout: Vec::new(),
                stderr: Vec::new(),
            }
        }
    };

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (otx, orx) = mpsc::channel();
    let (etx, erx) = mpsc::channel();
    if let Some(mut s) = stdout {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            let _ = otx.send(buf);
        });
    } else {
        let _ = otx.send(Vec::new());
    }
    if let Some(mut s) = stderr {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            let _ = etx.send(buf);
        });
    } else {
        let _ = etx.send(Vec::new());
    }

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

    let stdout = orx.recv().unwrap_or_default();
    let stderr = erx.recv().unwrap_or_default();
    match status {
        Some(s) => Run {
            spawn_failed: false,
            timed_out: false,
            success: s.success(),
            stdout,
            stderr,
        },
        None => Run {
            spawn_failed: false,
            timed_out: true,
            success: false,
            stdout,
            stderr,
        },
    }
}

/// `tea api <path>`를 실행해 stdout(순수 JSON)을 반환한다. tea는 진단 메시지를
/// stderr로 내보내므로 stdout만 파싱하면 된다.
fn tea_api(path: &str) -> Result<Vec<u8>, String> {
    let r = run("tea", &["api", path], None, TEA_TIMEOUT);
    if r.spawn_failed {
        return Err("tea 실행에 실패했습니다(설치 여부를 확인하세요)".to_string());
    }
    if r.timed_out {
        return Err("tea 요청이 시간 초과되었습니다".to_string());
    }
    if !r.success {
        let msg = String::from_utf8_lossy(&r.stderr);
        let msg = msg.trim();
        let msg = if msg.is_empty() {
            "tea 요청이 실패했습니다".to_string()
        } else {
            // stderr 첫 줄만(토큰 등 민감정보가 실릴 여지를 줄인다).
            msg.lines().next().unwrap_or("tea 요청 실패").to_string()
        };
        return Err(msg);
    }
    Ok(r.stdout)
}

/// Gitea 사용자(댓글 작성자·이슈 작성자)의 최소 표현.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct User {
    pub login: String,
}

/// 폴링으로 읽는 이슈. PR도 issues 엔드포인트에 섞여 나오므로 `pull_request`가
/// 채워진 항목은 호출부에서 건너뛴다(`is_pull` 참고).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Issue {
    pub number: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub user: User,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub pull_request: Option<serde_json::Value>,
}

impl Issue {
    /// PR은 Gitea issues 엔드포인트에도 나타난다. `pull_request`가 있으면 PR.
    pub fn is_pull(&self) -> bool {
        self.pull_request.is_some()
    }
}

/// 폴링으로 읽는 이슈 댓글. `issue_url`이 비어 있고 `pull_request_url`이 채워지면
/// PR 댓글이므로 건너뛴다.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Comment {
    pub id: u64,
    #[serde(default)]
    pub body: String,
    pub user: User,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub issue_url: String,
    #[serde(default)]
    pub pull_request_url: String,
}

impl Comment {
    /// PR에 달린 댓글이면 true(이슈 봇 대상 아님).
    pub fn is_pull_comment(&self) -> bool {
        self.issue_url.is_empty() && !self.pull_request_url.is_empty()
    }

    /// `issue_url`(예: `http://host/owner/repo/issues/57`) 끝에서 이슈 번호를
    /// 파싱한다. 파싱 불가면 None.
    pub fn issue_number(&self) -> Option<u64> {
        issue_number_from_url(&self.issue_url)
    }
}

fn issue_number_from_url(url: &str) -> Option<u64> {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
}

/// `origin` remote URL에서 `<owner>/<repo>` slug를 파싱한다. ssh(포트 포함)·
/// scp-like(`git@host:owner/repo`)·http(s) 형식을 모두 처리한다. `.git` 접미와
/// 후행 슬래시를 제거한 뒤 경로 세그먼트의 마지막 둘을 취한다.
pub fn parse_slug(remote_url: &str) -> Option<String> {
    let s = remote_url.trim();
    let s = s.strip_suffix(".git").unwrap_or(s);
    let s = s.trim_end_matches('/');
    let parts: Vec<&str> = s
        .split(|c| c == '/' || c == ':')
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 2 {
        return None;
    }
    let repo = parts[parts.len() - 1];
    let owner = parts[parts.len() - 2];
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// 저장소 루트에서 `git remote get-url origin`으로 slug를 감지한다. 봇 탭은
/// 이 slug의 이슈를 폴링한다.
pub fn detect_slug(root: &Path) -> Result<String, String> {
    let r = run("git", &["remote", "get-url", "origin"], Some(root), TEA_TIMEOUT);
    if r.spawn_failed {
        return Err("git 실행에 실패했습니다".to_string());
    }
    if !r.success {
        return Err("origin 원격을 찾을 수 없습니다".to_string());
    }
    let url = String::from_utf8_lossy(&r.stdout);
    parse_slug(url.trim()).ok_or_else(|| format!("원격 URL에서 저장소를 파싱할 수 없습니다: {}", url.trim()))
}

/// 현재 tea 로그인 계정(login)을 반환한다. 화이트리스트 기본값이자 게시 주체.
pub fn current_user() -> Result<String, String> {
    let bytes = tea_api("user")?;
    let user: User = serde_json::from_slice(&bytes)
        .map_err(|e| format!("tea 사용자 응답 파싱 실패: {e}"))?;
    Ok(user.login)
}

/// slug의 열린 이슈를 조회한다. `since`(ISO8601)가 있으면 그 이후 갱신분만.
/// PR은 제외한다.
pub fn list_open_issues(slug: &str, since: Option<&str>) -> Result<Vec<Issue>, String> {
    let mut path = format!("repos/{slug}/issues?type=issues&state=open");
    if let Some(s) = since {
        path.push_str("&since=");
        path.push_str(s);
    }
    let bytes = tea_api(&path)?;
    let issues: Vec<Issue> = serde_json::from_slice(&bytes)
        .map_err(|e| format!("이슈 목록 파싱 실패: {e}"))?;
    Ok(issues.into_iter().filter(|i| !i.is_pull()).collect())
}

/// slug의 이슈 댓글을 저장소 전체 단위로 증분 조회한다. `since`가 있으면 그 이후
/// 갱신분만. PR 댓글은 제외한다.
pub fn list_issue_comments(slug: &str, since: Option<&str>) -> Result<Vec<Comment>, String> {
    let mut path = format!("repos/{slug}/issues/comments");
    if let Some(s) = since {
        path.push_str("?since=");
        path.push_str(s);
    }
    let bytes = tea_api(&path)?;
    let comments: Vec<Comment> = serde_json::from_slice(&bytes)
        .map_err(|e| format!("댓글 목록 파싱 실패: {e}"))?;
    Ok(comments
        .into_iter()
        .filter(|c| !c.is_pull_comment())
        .collect())
}

/// 주어진 이슈 번호를 title/body에서 `#<번호>`로 참조하는 열린 PR의 번호를
/// 찾는다(완료 판정). 봇 잡의 결정적 완료 시그널 — 에이전트가 PR을 만들면서
/// 이슈를 언급하면 여기서 감지된다. 없으면 None.
pub fn find_pr_for_issue(slug: &str, issue: u64) -> Result<Option<u64>, String> {
    let path = format!("repos/{slug}/pulls?state=open");
    let bytes = tea_api(&path)?;
    let pulls: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("PR 목록 파싱 실패: {e}"))?;
    let needle = format!("#{issue}");
    if let Some(arr) = pulls.as_array() {
        for p in arr {
            let title = p.get("title").and_then(|v| v.as_str()).unwrap_or("");
            let body = p.get("body").and_then(|v| v.as_str()).unwrap_or("");
            if references_issue(title, &needle) || references_issue(body, &needle) {
                if let Some(n) = p.get("number").and_then(|v| v.as_u64()) {
                    return Ok(Some(n));
                }
            }
        }
    }
    Ok(None)
}

/// `text`가 `needle`(예: `#57`)을 토큰 경계에서 참조하는지. `#570` 같은 더 긴
/// 번호에 오탐하지 않도록 needle 뒤가 숫자면 매치로 보지 않는다.
fn references_issue(text: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = text[from..].find(needle) {
        let idx = from + pos;
        let after = idx + needle.len();
        let next_is_digit = text[after..].chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false);
        if !next_is_digit {
            return true;
        }
        from = after;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_slug_handles_ssh_with_port() {
        assert_eq!(
            parse_slug("ssh://git@100.107.46.116:2222/bugcaptor/agent-office.git"),
            Some("bugcaptor/agent-office".to_string())
        );
    }

    #[test]
    fn parse_slug_handles_scp_like() {
        assert_eq!(
            parse_slug("git@github.com:bugcaptor/agent-office.git"),
            Some("bugcaptor/agent-office".to_string())
        );
    }

    #[test]
    fn parse_slug_handles_http_and_trailing_slash() {
        assert_eq!(
            parse_slug("http://host:5088/owner/repo/"),
            Some("owner/repo".to_string())
        );
        assert_eq!(
            parse_slug("https://host/owner/repo"),
            Some("owner/repo".to_string())
        );
    }

    #[test]
    fn parse_slug_rejects_too_short() {
        assert_eq!(parse_slug("justonesegment"), None);
        assert_eq!(parse_slug(""), None);
    }

    #[test]
    fn issue_number_parsed_from_url() {
        assert_eq!(
            issue_number_from_url("http://100.107.46.116:5088/bugcaptor/agent-office/issues/57"),
            Some(57)
        );
        assert_eq!(
            issue_number_from_url("http://host/o/r/issues/12/"),
            Some(12)
        );
        assert_eq!(issue_number_from_url(""), None);
        assert_eq!(issue_number_from_url("http://host/o/r/issues/abc"), None);
    }

    #[test]
    fn references_issue_token_boundary() {
        assert!(references_issue("closes #57", "#57"));
        assert!(references_issue("이슈 #57 처리", "#57"));
        assert!(references_issue("#57", "#57"));
        assert!(!references_issue("#570 다른 이슈", "#57")); // 더 긴 번호에 오탐 금지
        assert!(!references_issue("아무 언급 없음", "#57"));
    }

    #[test]
    fn is_pull_comment_filters_pr_comments() {
        let pr_comment = Comment {
            id: 1,
            body: String::new(),
            user: User { login: "x".into() },
            updated_at: String::new(),
            issue_url: String::new(),
            pull_request_url: "http://host/o/r/pulls/1".into(),
        };
        assert!(pr_comment.is_pull_comment());
        let issue_comment = Comment {
            id: 2,
            body: String::new(),
            user: User { login: "x".into() },
            updated_at: String::new(),
            issue_url: "http://host/o/r/issues/5".into(),
            pull_request_url: String::new(),
        };
        assert!(!issue_comment.is_pull_comment());
        assert_eq!(issue_comment.issue_number(), Some(5));
    }
}
