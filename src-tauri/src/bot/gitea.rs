// src-tauri/src/bot/gitea.rs
//
// 봇 모드의 Gitea 접근 계층(#57, #58). 앱은 Gitea REST를 **직접**(reqwest) 호출해
// 읽기 전용 폴링을 한다: 이슈/댓글 조회, 계정 확인, PR head 조회(완료 판정).
// 접수/진행/완료 댓글과 PR 생성 등 모든 쓰기는 여전히 에이전트가 프롬프트 지시에
// 따라 로그인 셸 PTY에서 `curl`로 직접 수행한다. docs/bot-mode-design.md 참고.
//
// 이력(#58): 폴링은 원래 `tea api` 서브프로세스에 위임했으나, GUI(Finder/launchd)로
// 띄운 번들 앱은 최소 PATH라 Homebrew의 `tea`를 못 찾아 실패했다. 이제 폴링을
// reqwest REST로 옮겨 `tea` spawn 의존을 없앴고, 토큰/베이스 URL은 봇 시작 시
// 로그인 셸에서 캡처한 env(session::env_capture)에서 읽는다. `git`은 origin slug
// 감지에만 남는데, `git`은 보통 `/usr/bin/git`(Xcode CLT)이라 최소 PATH에도 있다.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// git 호출 기본 타임아웃(origin slug 감지용).
const GIT_TIMEOUT: Duration = Duration::from_secs(20);
/// REST 요청 타임아웃. 네트워크 왕복을 감안.
const REST_TIMEOUT: Duration = Duration::from_secs(20);

/// git 서브프로세스 1회 실행 결과.
struct Run {
    spawn_failed: bool,
    success: bool,
    stdout: Vec<u8>,
}

/// `git`을 `args`로 한 번 실행한다. stdout을 별도 스레드로 끝까지 읽어 파이프
/// 교착을 막고, 타임아웃을 넘기면 자식을 죽인다. slug 감지 전용이라 stderr는
/// 버린다.
fn run_git(args: &[&str], cwd: &Path, timeout: Duration) -> Run {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // C 로케일 강제. GUI 기동 앱은 LANG/LC_*가 비어 git이 시스템 로케일로 오류를
    // 낼 수 있다. slug 감지는 성공 경로만 파싱하지만, 일관성을 위해 고정한다.
    cmd.env("LC_ALL", "C");
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
                success: false,
                stdout: Vec::new(),
            }
        }
    };

    let stdout = child.stdout.take();
    let (otx, orx) = mpsc::channel();
    if let Some(mut s) = stdout {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            let _ = otx.send(buf);
        });
    } else {
        let _ = otx.send(Vec::new());
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
    Run {
        spawn_failed: false,
        success: status.map(|s| s.success()).unwrap_or(false),
        stdout,
    }
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

/// http(s) origin remote URL에서 Gitea 웹 베이스(`scheme://host[:port]`)를
/// 파싱한다. ssh/scp-like 형식은 웹 포트를 알 수 없어 None(그 경우 호출부가
/// `GITEA_BASE_URL`을 요구한다).
pub fn parse_base_url(remote_url: &str) -> Option<String> {
    let s = remote_url.trim();
    let rest = s.strip_prefix("http://").map(|r| ("http", r))
        .or_else(|| s.strip_prefix("https://").map(|r| ("https", r)))?;
    let (scheme, after) = rest;
    // after = `host[:port]/owner/repo…` 또는 `user@host…`. 첫 '/' 전까지가 authority.
    let authority = after.split('/').next().unwrap_or("");
    // 자격정보(user[:pass]@)가 있으면 제거.
    let host_port = authority.rsplit('@').next().unwrap_or(authority);
    if host_port.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host_port}"))
}

/// 저장소 루트에서 `git remote get-url origin`으로 slug를 감지한다. 봇 탭은
/// 이 slug의 이슈를 폴링한다.
pub fn detect_slug(root: &Path) -> Result<String, String> {
    let r = run_git(&["remote", "get-url", "origin"], root, GIT_TIMEOUT);
    if r.spawn_failed {
        return Err("git 실행에 실패했습니다".to_string());
    }
    if !r.success {
        return Err("origin 원격을 찾을 수 없습니다".to_string());
    }
    let url = String::from_utf8_lossy(&r.stdout);
    parse_slug(url.trim())
        .ok_or_else(|| format!("원격 URL에서 저장소를 파싱할 수 없습니다: {}", url.trim()))
}

/// Gitea 웹 베이스 URL을 도출한다. 우선순위: `GITEA_BASE_URL` env(오버라이드) →
/// http(s) origin에서 파싱. ssh origin이고 env도 없으면, 웹 포트를 알 수 없어
/// 명시적 설정을 요구하는 오류를 낸다.
pub fn resolve_base_url(root: &Path) -> Result<String, String> {
    if let Some(v) = crate::api_keys::env_api_key(crate::api_keys::GITEA_BASE_URL) {
        return Ok(v.trim_end_matches('/').to_string());
    }
    let r = run_git(&["remote", "get-url", "origin"], root, GIT_TIMEOUT);
    if !r.spawn_failed && r.success {
        let url = String::from_utf8_lossy(&r.stdout);
        if let Some(base) = parse_base_url(url.trim()) {
            return Ok(base.trim_end_matches('/').to_string());
        }
    }
    Err("Gitea 베이스 URL을 알 수 없습니다 — 셸 프로파일에 GITEA_BASE_URL(예: http://host:5088)을 설정하세요".to_string())
}

/// 봇 폴링용 Gitea REST 클라이언트. 베이스 URL과 토큰을 보관하고 인증 헤더로
/// 임의 API GET을 수행한다. 토큰은 로그로 절대 내보내지 않는다.
pub struct Gitea {
    base_url: String,
    token: String,
    client: reqwest::blocking::Client,
}

impl Gitea {
    /// env에서 토큰을, `root`/env에서 베이스 URL을 읽어 클라이언트를 만든다.
    /// (봇 시작 전 session::env_capture가 로그인 셸 값을 프로세스 env에 심어둔다.)
    pub fn from_env(root: &Path) -> Result<Self, String> {
        let base_url = resolve_base_url(root)?;
        let token = crate::api_keys::env_api_key(crate::api_keys::GITEA_TOKEN).ok_or_else(|| {
            "GITEA_TOKEN 환경변수가 비어 있습니다 — 셸 프로파일에 export 하세요".to_string()
        })?;
        let client = reqwest::blocking::Client::builder()
            .timeout(REST_TIMEOUT)
            .build()
            .map_err(|e| format!("HTTP 클라이언트 생성 실패: {e}"))?;
        Ok(Self {
            base_url,
            token,
            client,
        })
    }

    /// `<base>/api/v1/<path>`를 GET해 응답 본문 바이트를 반환한다. 인증 실패·
    /// 네트워크·HTTP 오류를 각각 구분된 한국어 메시지로 매핑한다(토큰 미노출).
    fn get(&self, path: &str) -> Result<Vec<u8>, String> {
        let url = format!("{}/api/v1/{}", self.base_url, path);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("token {}", self.token))
            .header("Accept", "application/json")
            .send()
            .map_err(|e| {
                if e.is_timeout() {
                    "Gitea 요청이 시간 초과되었습니다".to_string()
                } else {
                    "Gitea 연결에 실패했습니다(네트워크/베이스 URL 확인)".to_string()
                }
            })?;
        let status = resp.status();
        if status.as_u16() == 401 || status.as_u16() == 403 {
            return Err("Gitea 인증 실패 — GITEA_TOKEN이 유효한지 확인하세요".to_string());
        }
        if !status.is_success() {
            return Err(format!("Gitea 요청 실패(HTTP {})", status.as_u16()));
        }
        resp.bytes()
            .map(|b| b.to_vec())
            .map_err(|_| "Gitea 응답 본문을 읽지 못했습니다".to_string())
    }

    /// 현재 토큰 소유 계정(login)을 반환한다. 화이트리스트 기본값이자 게시 주체.
    pub fn current_user(&self) -> Result<String, String> {
        let bytes = self.get("user")?;
        let user: User =
            serde_json::from_slice(&bytes).map_err(|e| format!("사용자 응답 파싱 실패: {e}"))?;
        Ok(user.login)
    }

    /// slug의 열린 이슈를 조회한다. `since`(ISO8601)가 있으면 그 이후 갱신분만.
    /// PR은 제외한다.
    pub fn list_open_issues(
        &self,
        slug: &str,
        since: Option<&str>,
    ) -> Result<Vec<Issue>, String> {
        let mut path = format!("repos/{slug}/issues?type=issues&state=open");
        if let Some(s) = since {
            path.push_str("&since=");
            path.push_str(s);
        }
        let bytes = self.get(&path)?;
        let issues: Vec<Issue> =
            serde_json::from_slice(&bytes).map_err(|e| format!("이슈 목록 파싱 실패: {e}"))?;
        Ok(issues.into_iter().filter(|i| !i.is_pull()).collect())
    }

    /// slug의 이슈 댓글을 저장소 전체 단위로 증분 조회한다. `since`가 있으면 그
    /// 이후 갱신분만. PR 댓글은 제외한다.
    pub fn list_issue_comments(
        &self,
        slug: &str,
        since: Option<&str>,
    ) -> Result<Vec<Comment>, String> {
        let mut path = format!("repos/{slug}/issues/comments");
        if let Some(s) = since {
            path.push_str("?since=");
            path.push_str(s);
        }
        let bytes = self.get(&path)?;
        let comments: Vec<Comment> =
            serde_json::from_slice(&bytes).map_err(|e| format!("댓글 목록 파싱 실패: {e}"))?;
        Ok(comments
            .into_iter()
            .filter(|c| !c.is_pull_comment())
            .collect())
    }

    /// 주어진 이슈 번호를 title/body에서 `#<번호>`로 참조하는 열린 PR의 번호를
    /// 찾는다(완료 판정). 없으면 None.
    pub fn find_pr_for_issue(&self, slug: &str, issue: u64) -> Result<Option<u64>, String> {
        let path = format!("repos/{slug}/pulls?state=open");
        let bytes = self.get(&path)?;
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
}

/// `text`가 `needle`(예: `#57`)을 토큰 경계에서 참조하는지. `#570` 같은 더 긴
/// 번호에 오탐하지 않도록 needle 뒤가 숫자면 매치로 보지 않는다.
fn references_issue(text: &str, needle: &str) -> bool {
    let mut from = 0;
    while let Some(pos) = text[from..].find(needle) {
        let idx = from + pos;
        let after = idx + needle.len();
        let next_is_digit = text[after..]
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);
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
    fn parse_base_url_from_http_with_port() {
        assert_eq!(
            parse_base_url("http://100.107.46.116:5088/bugcaptor/agent-office.git"),
            Some("http://100.107.46.116:5088".to_string())
        );
    }

    #[test]
    fn parse_base_url_from_https_no_port() {
        assert_eq!(
            parse_base_url("https://gitea.example.com/owner/repo"),
            Some("https://gitea.example.com".to_string())
        );
    }

    #[test]
    fn parse_base_url_strips_credentials() {
        assert_eq!(
            parse_base_url("http://user:pass@host:5088/owner/repo"),
            Some("http://host:5088".to_string())
        );
    }

    #[test]
    fn parse_base_url_rejects_ssh() {
        assert_eq!(
            parse_base_url("ssh://git@100.107.46.116:2222/bugcaptor/agent-office.git"),
            None
        );
        assert_eq!(parse_base_url("git@github.com:owner/repo.git"), None);
    }

    #[test]
    fn issue_number_parsed_from_url() {
        assert_eq!(
            issue_number_from_url("http://100.107.46.116:5088/bugcaptor/agent-office/issues/57"),
            Some(57)
        );
        assert_eq!(issue_number_from_url("http://host/o/r/issues/12/"), Some(12));
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
