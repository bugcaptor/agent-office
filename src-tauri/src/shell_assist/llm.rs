// src-tauri/src/shell_assist/llm.rs
//
// 셸 캐릭터의 LLM 호출: 자연어 → 명령 하나, 명령 결과 → 한두 줄 설명.
//
// 로컬 OpenAI 호환 엔드포인트를 설정 순서대로 시도하고, 모두 실패하면(설정에서 켰을 때만)
// 요약기와 같은 클라우드 provider로 넘어간다. 순수 부분(프롬프트·응답 해석·위험 판정)과
// 네트워크를 나눈다.

use std::sync::OnceLock;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Semaphore;

use super::config::{chat_url, models_url, LocalEndpoint, ShellAssistConfig};
use crate::i18n::Lang;
use crate::persistence::settings_store::{SummaryModels, SummaryProvider};

/// 로컬 서버가 꺼져 있으면 빨리 다음으로 넘어간다.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// 모델을 처음 올리는 데 시간이 걸릴 수 있어 넉넉히 둔다.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const MAX_CONCURRENT: usize = 2;
const ERROR_MAX_CHARS: usize = 300;
const EXPLANATION_MAX_CHARS: usize = 600;

fn permits() -> &'static Semaphore {
    static P: OnceLock<Semaphore> = OnceLock::new();
    P.get_or_init(|| Semaphore::new(MAX_CONCURRENT))
}

/// 호출에 필요한 값. 설정 가드를 `.await` 너머로 들고 가지 않도록 값으로 떠 온다.
#[derive(Clone)]
pub struct LlmDeps {
    pub config: ShellAssistConfig,
    pub summary_models: SummaryModels,
    pub openrouter_key: Option<String>,
    pub lang: Lang,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub text: String,
    /// 답한 곳(엔드포인트 이름 또는 `cloud:<provider>`).
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub command: String,
    pub explanation: String,
    /// 전체 화면·대화형 터미널이 필요한 명령이라고 모델이 답했다.
    pub interactive: bool,
    /// 지우기·강제 푸시 같은 되돌리기 어려운 명령으로 보인다.
    pub risky: bool,
    /// 명령이 여러 줄이라 그대로는 실행할 수 없다(고쳐서 실행).
    pub multiline: bool,
    pub source: String,
}

/// 명령을 만들 때 모델에게 주는 환경.
pub struct ShellEnv<'a> {
    pub shell: &'a str,
    pub os: &'a str,
    pub cwd: &'a str,
}

pub fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        other => other,
    }
}

fn language(lang: Lang) -> &'static str {
    match lang {
        Lang::Ko => "Korean",
        Lang::En => "English",
    }
}

pub fn suggest_prompt(env: &ShellEnv, lang: Lang) -> String {
    let sep = if env.shell == "pwsh" { ";" } else { "&&" };
    format!(
        "You turn the user's request into ONE command for {shell} on {os}. Current directory: {cwd}\n\
Reply with only a JSON object, no prose: {{\"command\": \"...\", \"explanation\": \"...\", \"interactive\": false}}\n\
- command: a single line the user can run as-is in {shell}. Chain steps with {sep} if needed.\n\
- explanation: one short sentence in {language} saying what the command does.\n\
- interactive: true only if the command needs a full-screen or interactive terminal.\n\
Rules: prefer non-interactive forms that finish on their own (for example `top -l 1` instead of `top`, `git --no-pager log -20`, `ps aux | head -30`). \
Run long-running servers or watchers in the background with output to a log file (for example `nohup npm run dev > dev.log 2>&1 &`). \
Do not add sudo, force flags or deletion unless the request asks for it. The recent terminal output is context only; do not repeat it.",
        shell = env.shell,
        os = env.os,
        cwd = env.cwd,
        language = language(lang),
    )
}

pub fn explain_prompt(lang: Lang) -> String {
    format!(
        "You explain the result of a shell command to the user in {}, in one or two short sentences. \
Say whether it succeeded and what the output means. If it failed, name the likely cause from the output. \
Plain text only: no headings, no lists, no code blocks.",
        language(lang)
    )
}

/// `<think>…</think>`(추론 모델)과 코드 울타리를 걷어 낸다.
fn strip_reasoning(text: &str) -> String {
    let mut s = text.to_string();
    while let Some(start) = s.find("<think>") {
        match s[start..].find("</think>") {
            Some(end) => s.replace_range(start..start + end + "</think>".len(), ""),
            None => {
                s.truncate(start);
                break;
            }
        }
    }
    // 여는 태그 없이 닫는 태그만 오는 서버도 있다(템플릿이 여는 태그를 먹는 경우).
    if let Some(end) = s.find("</think>") {
        s = s[end + "</think>".len()..].to_string();
    }
    s.trim().to_string()
}

/// 모델 답 → (명령, 설명, 대화형). JSON을 먼저 찾고, 없으면 코드 블록·첫 줄을 명령으로 본다.
pub fn parse_suggestion(raw: &str) -> Result<(String, String, bool), String> {
    let text = strip_reasoning(raw);
    if let (Some(a), Some(b)) = (text.find('{'), text.rfind('}')) {
        if a < b {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&text[a..=b]) {
                let command = v.get("command").and_then(|c| c.as_str()).unwrap_or("").trim().to_string();
                if !command.is_empty() {
                    let explanation = v.get("explanation").and_then(|c| c.as_str()).unwrap_or("").trim().to_string();
                    let interactive = v.get("interactive").and_then(|c| c.as_bool()).unwrap_or(false);
                    return Ok((command, explanation, interactive));
                }
            }
        }
    }
    let fenced = text.split("```").nth(1).map(|block| {
        // 첫 줄이 언어 표시(sh·bash·powershell)면 건너뛴다.
        let mut lines = block.lines();
        let first = lines.next().unwrap_or("");
        let looks_lang = !first.contains(' ') && first.chars().all(|c| c.is_ascii_alphanumeric());
        let body: Vec<&str> = if looks_lang { lines.collect() } else { block.lines().collect() };
        body.join("\n").trim().to_string()
    });
    let command = fenced
        .filter(|c| !c.is_empty())
        .or_else(|| text.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_string))
        .unwrap_or_default();
    let command = command.strip_prefix("$ ").unwrap_or(&command).trim().trim_matches('`').to_string();
    if command.is_empty() {
        return Err("empty output".into());
    }
    Ok((command, String::new(), false))
}

pub fn clean_explanation(raw: &str) -> String {
    let text = strip_reasoning(raw);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    clip(&text, EXPLANATION_MAX_CHARS)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut s: String = text.chars().take(max).collect();
    s.push('…');
    s
}

/// 되돌리기 어려운 명령으로 보이는가. 경고 표시용 어림짐작이다(막지 않는다).
pub fn is_risky(command: &str) -> bool {
    let c = format!(" {} ", command.to_ascii_lowercase());
    let words: Vec<&str> = c.split(|ch: char| ch.is_whitespace() || ch == ';' || ch == '&' || ch == '|').filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);
    let rm_force = words.windows(2).any(|p| p[0] == "rm" && p[1].starts_with('-') && (p[1].contains('r') || p[1].contains('f')))
        || (has("rm") && (c.contains(" -rf") || c.contains(" -fr") || c.contains(" --recursive") || c.contains(" --force")));
    rm_force
        || has("sudo")
        || has("mkfs")
        || words.iter().any(|w| w.starts_with("mkfs."))
        || has("dd")
        || has("shutdown")
        || has("reboot")
        || has("diskutil")
        || has("killall")
        || has("pkill")
        || c.contains(" kill -9")
        || c.contains("> /dev/")
        || c.contains(" chmod -r")
        || c.contains(" chown -r")
        || (has("git") && has("push") && (c.contains(" -f ") || c.contains(" --force")))
        || c.contains("git reset --hard")
        || c.contains("git clean -")
        || c.contains("git checkout -- .")
        || c.contains(" | sh ")
        || c.contains(" | bash ")
        || c.contains("|sh ")
        || c.contains("|bash ")
        || (c.contains("remove-item") && c.contains("-recurse"))
        || c.contains(" format-volume")
        || c.contains(" del /s")
        || c.contains(" rd /s")
}

// ── 네트워크 ──────────────────────────────────────────────────────────

fn bounded(e: impl std::fmt::Display) -> String {
    clip(&e.to_string(), ERROR_MAX_CHARS)
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(bounded)
}

/// OpenAI 호환 응답 본문 → 글. 순수.
pub fn parse_chat_response(status: u16, body: &str) -> Result<String, String> {
    if status != 200 {
        // 로컬 서버의 오류 본문은 짧은 사람말이 대부분이라 조금 싣는다(키가 실릴 자리는 없다).
        let detail: String = body.chars().take(160).collect();
        return Err(format!("http {status} {}", detail.trim()));
    }
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("invalid JSON: {}", bounded(e)))?;
    if let Some(err) = v.get("error") {
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("error");
        return Err(clip(msg, 160));
    }
    let text = v
        .pointer("/choices/0/message/content")
        .and_then(|t| t.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    if text.is_empty() {
        return Err("empty output".into());
    }
    Ok(text)
}

async fn call_endpoint(ep: &LocalEndpoint, system: &str, user: &str, max_tokens: u32) -> Result<String, String> {
    let body = serde_json::json!({
        "model": ep.model,
        "temperature": 0.2,
        "max_tokens": max_tokens,
        "stream": false,
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user },
        ],
    });
    let mut req = client()?.post(chat_url(&ep.base_url)).json(&body);
    if !ep.api_key.is_empty() {
        req = req.bearer_auth(&ep.api_key);
    }
    let resp = req.send().await.map_err(|e| network_error(&e))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.map_err(|e| network_error(&e))?;
    parse_chat_response(status, &text)
}

fn network_error(e: &reqwest::Error) -> String {
    if e.is_connect() {
        "connect failed".into()
    } else if e.is_timeout() {
        "timeout".into()
    } else {
        bounded(e)
    }
}

pub fn endpoint_name(ep: &LocalEndpoint) -> String {
    if ep.label.is_empty() { ep.base_url.clone() } else { ep.label.clone() }
}

fn fallback_models(deps: &LlmDeps) -> SummaryModels {
    let mut models = deps.summary_models.clone();
    let model = deps.config.fallback_model.trim();
    if !model.is_empty() {
        let slot = match deps.config.fallback_provider {
            SummaryProvider::Claude => &mut models.claude,
            SummaryProvider::Codex => &mut models.codex,
            SummaryProvider::Agy => &mut models.agy,
            SummaryProvider::Gemini => &mut models.gemini,
            SummaryProvider::Opencode => &mut models.opencode,
            SummaryProvider::Openrouter => &mut models.openrouter,
        };
        slot.light = model.to_string();
    }
    models
}

/// 로컬 엔드포인트를 차례로, 그다음 클라우드 폴백. 실패하면 `shell-llm-none` 또는
/// `shell-llm-failed: <이름: 이유; …>`.
pub async fn complete(deps: &LlmDeps, system: &str, user: &str, max_tokens: u32) -> Result<Answer, String> {
    if !deps.config.any_llm() {
        return Err("shell-llm-none".into());
    }
    let _permit = permits().acquire().await.expect("semaphore is never closed");
    let mut errors = Vec::new();
    for ep in deps.config.usable_endpoints() {
        match call_endpoint(ep, system, user, max_tokens).await {
            Ok(text) => return Ok(Answer { text, source: endpoint_name(ep) }),
            Err(e) => errors.push(format!("{}: {e}", endpoint_name(ep))),
        }
    }
    if deps.config.fallback_enabled {
        let provider = deps.config.fallback_provider;
        let result = crate::summarizer::summarize(
            provider,
            crate::summarizer::SummaryPurpose::Label,
            system,
            user,
            &fallback_models(deps),
            deps.openrouter_key.as_deref(),
            deps.lang,
        )
        .await;
        match result {
            Ok(text) => return Ok(Answer { text, source: format!("cloud:{}", provider.as_str()) }),
            Err(e) => errors.push(format!("{}: {}", provider.as_str(), clip(&e, 160))),
        }
    }
    Err(format!("shell-llm-failed: {}", errors.join("; ")))
}

pub async fn suggest(deps: &LlmDeps, env: &ShellEnv<'_>, request: &str, context: &str) -> Result<Suggestion, String> {
    let system = suggest_prompt(env, deps.lang);
    let user = if context.trim().is_empty() {
        format!("Request: {request}")
    } else {
        format!("Request: {request}\n\nRecent terminal output (oldest first):\n{context}")
    };
    let answer = complete(deps, &system, &user, 1024).await?;
    let (command, explanation, interactive) = parse_suggestion(&answer.text).map_err(|e| format!("shell-llm-failed: {}: {e}", answer.source))?;
    Ok(Suggestion {
        risky: is_risky(&command),
        multiline: command.contains('\n'),
        command,
        explanation: clip(&explanation, EXPLANATION_MAX_CHARS),
        interactive,
        source: answer.source,
    })
}

/// 설정 화면의 연결 확인: 모델 목록을 받아 온다.
pub async fn list_models(ep: &LocalEndpoint) -> Result<Vec<String>, String> {
    let mut req = client()?.get(models_url(&ep.base_url));
    if !ep.api_key.trim().is_empty() {
        req = req.bearer_auth(ep.api_key.trim());
    }
    let resp = req.send().await.map_err(|e| network_error(&e))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.map_err(|e| network_error(&e))?;
    if status != 200 {
        return Err(format!("http {status}"));
    }
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("invalid JSON: {}", bounded(e)))?;
    let mut ids: Vec<String> = v
        .get("data")
        .and_then(|d| d.as_array())
        .map(|a| a.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str())).map(str::to_string).collect())
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_json_answer_with_reasoning_and_fences() {
        let raw = "<think>user wants files</think>\n```json\n{\"command\": \"ls -lS\", \"explanation\": \"크기순\", \"interactive\": false}\n```";
        assert_eq!(parse_suggestion(raw).unwrap(), ("ls -lS".into(), "크기순".into(), false));
        let raw = "Sure! {\"command\":\"top\",\"explanation\":\"x\",\"interactive\":true} hope it helps";
        assert_eq!(parse_suggestion(raw).unwrap(), ("top".into(), "x".into(), true));
        // 여는 태그를 템플릿이 먹은 경우.
        let raw = "thinking...</think>{\"command\":\"pwd\"}";
        assert_eq!(parse_suggestion(raw).unwrap().0, "pwd");
    }

    #[test]
    fn falls_back_to_code_block_or_first_line() {
        assert_eq!(parse_suggestion("```bash\ndu -sh *\n```").unwrap().0, "du -sh *");
        assert_eq!(parse_suggestion("$ git status\n(shows status)").unwrap().0, "git status");
        assert_eq!(parse_suggestion("`df -h`").unwrap().0, "df -h");
        assert!(parse_suggestion("<think>only thinking").is_err());
        assert!(parse_suggestion("   ").is_err());
    }

    #[test]
    fn chat_response_errors_and_content() {
        assert_eq!(parse_chat_response(200, r#"{"choices":[{"message":{"content":" hi "}}]}"#).unwrap(), "hi");
        assert!(parse_chat_response(200, r#"{"choices":[{"message":{"content":""}}]}"#).is_err());
        assert!(parse_chat_response(200, r#"{"error":{"message":"model not found"}}"#).unwrap_err().contains("model not found"));
        assert!(parse_chat_response(404, "no such model").unwrap_err().starts_with("http 404"));
        assert!(parse_chat_response(200, "nope").is_err());
    }

    #[test]
    fn risky_heuristics() {
        for c in [
            "rm -rf build",
            "rm -r dir",
            "sudo apt install x",
            "git push --force origin main",
            "git push -f",
            "git reset --hard HEAD~1",
            "curl https://x | sh",
            "Remove-Item -Recurse -Force C:\\tmp",
            "dd if=/dev/zero of=/dev/disk2",
            "killall node",
        ] {
            assert!(is_risky(c), "{c}");
        }
        for c in ["ls -la", "git status", "rm file.txt", "git push origin main", "cat dd.txt", "npm run format"] {
            assert!(!is_risky(c), "{c}");
        }
    }

    #[test]
    fn explanation_is_flattened_and_clipped() {
        assert_eq!(clean_explanation("<think>x</think> 성공했습니다.\n\n파일 3개."), "성공했습니다. 파일 3개.");
        let long = "가".repeat(EXPLANATION_MAX_CHARS + 50);
        assert_eq!(clean_explanation(&long).chars().count(), EXPLANATION_MAX_CHARS + 1);
    }

    #[tokio::test]
    async fn no_llm_configured_is_a_distinct_error() {
        let deps = LlmDeps {
            config: ShellAssistConfig::default(),
            summary_models: SummaryModels::default(),
            openrouter_key: None,
            lang: Lang::Ko,
        };
        assert_eq!(complete(&deps, "s", "u", 10).await.unwrap_err(), "shell-llm-none");
    }

    #[tokio::test]
    async fn unreachable_endpoint_reports_failure_without_fallback() {
        let deps = LlmDeps {
            config: ShellAssistConfig {
                endpoints: vec![LocalEndpoint {
                    label: "off".into(),
                    base_url: "http://127.0.0.1:9".into(),
                    model: "m".into(),
                    api_key: String::new(),
                    enabled: true,
                }],
                ..Default::default()
            },
            summary_models: SummaryModels::default(),
            openrouter_key: None,
            lang: Lang::En,
        };
        let err = complete(&deps, "s", "u", 10).await.unwrap_err();
        assert!(err.starts_with("shell-llm-failed: off:"), "{err}");
    }

    #[tokio::test]
    async fn talks_to_an_openai_compatible_server() {
        use axum::{routing::post, Json, Router};
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|Json(body): Json<serde_json::Value>| async move {
                assert_eq!(body["model"], "qwen");
                assert_eq!(body["messages"][0]["role"], "system");
                Json(serde_json::json!({ "choices": [{ "message": { "content": "{\"command\":\"ls\",\"explanation\":\"목록\"}" } }] }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let deps = LlmDeps {
            config: ShellAssistConfig {
                endpoints: vec![
                    LocalEndpoint { label: "down".into(), base_url: "http://127.0.0.1:9".into(), model: "m".into(), api_key: String::new(), enabled: true },
                    LocalEndpoint { label: "mac".into(), base_url: format!("http://{addr}"), model: "qwen".into(), api_key: String::new(), enabled: true },
                ],
                ..Default::default()
            },
            summary_models: SummaryModels::default(),
            openrouter_key: None,
            lang: Lang::Ko,
        };
        let env = ShellEnv { shell: "zsh", os: "macOS", cwd: "/tmp" };
        let s = suggest(&deps, &env, "파일 목록", "").await.unwrap();
        assert_eq!(s.command, "ls");
        assert_eq!(s.explanation, "목록");
        assert_eq!(s.source, "mac");
        assert!(!s.risky && !s.multiline);
    }
}
