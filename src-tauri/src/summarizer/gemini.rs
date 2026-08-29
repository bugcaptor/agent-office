use super::ProviderCommand;
use crate::persistence::settings_store::SummaryProvider;

// gemini CLI는 headless(-p) 모드에서 stdin 본문을 지원한다 — 최종 프롬프트는
// stdin 뒤에 -p 지시문이 덧붙는 형태(공식: "Appended to input on stdin").
// run_with_timeout이 cwd를 임시 폴더로 잡는데 gemini는 비신뢰 폴더에서
// headless 실행을 거부하므로 --skip-trust가 필수다. 순수 텍스트 변환이라
// 도구 승인은 발생하지 않는다.
#[cfg(windows)]
const WINDOWS_SCRIPT: &str = r#"$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Console]::OutputEncoding=[System.Text.Encoding]::UTF8
$OutputEncoding=New-Object System.Text.UTF8Encoding($false)
$c = Get-Command $env:AO_PROGRAM -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $c) { exit 3 }
$in = [Console]::In.ReadToEnd()
$in | & $c.Source --prompt $env:AO_INSTRUCTION --model $env:AO_MODEL --output-format text --skip-trust
exit $LASTEXITCODE"#;

#[cfg(windows)]
pub(super) fn build(program: &str, instruction: &str, model: &str) -> ProviderCommand {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = std::process::Command::new("powershell.exe");
    command.args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]);
    command.creation_flags(CREATE_NO_WINDOW);
    command.env("AO_PROGRAM", program);
    command.env("AO_INSTRUCTION", instruction);
    command.env("AO_MODEL", model);
    ProviderCommand {
        command,
        provider: SummaryProvider::Gemini,
    }
}

#[cfg(not(windows))]
pub(super) fn build(program: &str, instruction: &str, model: &str) -> ProviderCommand {
    let mut command = std::process::Command::new(program);
    command.args([
        "--prompt",
        instruction,
        "--model",
        model,
        "--output-format",
        "text",
        "--skip-trust",
    ]);
    ProviderCommand {
        command,
        provider: SummaryProvider::Gemini,
    }
}

// ── 설정 화면의 모델 카탈로그(`list_provider_models`) ───────────────────────
//
// gemini CLI에는 codex/agy/opencode와 달리 모델 목록 서브커맨드가 없다
// (`gemini --help` 기준: mcp/extensions/skills/hooks/gemma뿐). 그래서 CLI가
// 아니라 Google Generative Language API의 공개 목록 엔드포인트를 쓴다.
//
// 이 엔드포인트는 키가 필수인데, gemini CLI는 보통 OAuth로 로그인하므로
// 앱에는 저장된 Gemini 키가 없다. 새 키 입력 UI를 만드는 대신 **환경변수만**
// 본다(api_keys::env_api_key와 같은 규약) — 키가 없으면 네트워크를 타지 않고
// 조용히 빈 목록이고, 호출측은 정적 프리셋으로 강등한다. 즉 이 라이브 조회는
// `GEMINI_API_KEY`(또는 `GOOGLE_API_KEY`)를 이미 쓰는 사용자에게만 켜진다.
//
// 키 값은 로그·에러 문자열에 절대 싣지 않는다(tts/keys.rs와 같은 규칙) —
// 그래서 URL 쿼리가 아니라 `x-goog-api-key` 헤더로 보낸다.

/// 카탈로그 조회에 쓰는 환경변수 이름(우선순위 순). Google 도구들이 둘 다
/// 관용적으로 받아들인다.
pub const API_KEY_ENVS: [&str; 2] = ["GEMINI_API_KEY", "GOOGLE_API_KEY"];

/// `pageSize` 상한은 1000이다 — 카탈로그가 그보다 길어질 일은 없어 페이지를
/// 넘기지 않는다(넘겼다면 잘린 목록이지만, 프리셋이 앞을 지키므로 무해하다).
pub const MODELS_URL: &str =
    "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1000";

/// 모델 카탈로그 응답 → 모델 id 목록. `name`은 `models/<id>` 형태라 접두를
/// 벗긴다. `generateContent`를 지원하지 않는 항목(임베딩·TTS 전용 등)은
/// 요약에 쓸 수 없으므로 버린다 — 단, 필드 자체가 없으면 남긴다(스키마가
/// 바뀌었을 때 목록이 통째로 비는 것이 더 나쁘다). 순수.
pub fn parse_models_response(status: u16, body: &str) -> Result<Vec<String>, String> {
    if status != 200 {
        return Err(format!("gemini http {status}"));
    }
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| {
        format!(
            "gemini invalid JSON: {}",
            super::bounded_detail(&e.to_string())
        )
    })?;
    let mut ids: Vec<String> = v
        .get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter(|m| {
                    m.get("supportedGenerationMethods")
                        .and_then(|s| s.as_array())
                        .map(|methods| {
                            methods
                                .iter()
                                .any(|x| x.as_str() == Some("generateContent"))
                        })
                        .unwrap_or(true)
                })
                .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
                .map(|n| n.trim().trim_start_matches("models/").trim())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// 모델 목록 조회. 키가 없으면(환경변수 미설정) 네트워크를 타지 않고 빈
/// 목록이다. 실패도 전부 빈 목록 — 다른 provider의 `list_models`와 같은 계약.
pub async fn list_models(timeout: std::time::Duration) -> Vec<String> {
    let Some(api_key) = API_KEY_ENVS
        .iter()
        .find_map(|name| crate::api_keys::env_api_key(name))
    else {
        return Vec::new();
    };
    let Ok(client) = reqwest::Client::builder().timeout(timeout).build() else {
        return Vec::new();
    };
    let Ok(resp) = client
        .get(MODELS_URL)
        .header("x-goog-api-key", api_key)
        .send()
        .await
    else {
        return Vec::new();
    };
    let status = resp.status().as_u16();
    let Ok(text) = resp.text().await else {
        return Vec::new();
    };
    parse_models_response(status, &text).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_models_response_strips_the_models_prefix_and_sorts() {
        let body = r#"{"models":[
            {"name":"models/gemini-3.1-pro-preview","supportedGenerationMethods":["generateContent"]},
            {"name":"models/gemini-3-flash-preview","supportedGenerationMethods":["generateContent","countTokens"]}
        ]}"#;
        assert_eq!(
            parse_models_response(200, body).unwrap(),
            vec![
                "gemini-3-flash-preview".to_string(),
                "gemini-3.1-pro-preview".to_string(),
            ]
        );
    }

    /// 임베딩·TTS 전용처럼 generateContent를 못 하는 모델은 요약에 쓸 수
    /// 없다. 반대로 필드가 아예 없으면(스키마 변화) 남긴다.
    #[test]
    fn parse_models_response_filters_models_that_cannot_generate_content() {
        let body = r#"{"models":[
            {"name":"models/text-embedding-004","supportedGenerationMethods":["embedContent"]},
            {"name":"models/unknown-shape"}
        ]}"#;
        assert_eq!(
            parse_models_response(200, body).unwrap(),
            vec!["unknown-shape".to_string()]
        );
    }

    #[test]
    fn parse_models_response_reports_http_and_json_failures() {
        assert!(parse_models_response(403, "{}").is_err());
        assert!(parse_models_response(200, "not json").is_err());
        // 정상 응답인데 목록이 비는 것은 오류가 아니다.
        assert!(parse_models_response(200, r#"{"models":[]}"#)
            .unwrap()
            .is_empty());
    }

    /// 키는 URL 쿼리가 아니라 헤더로 보낸다 — 쿼리에 실으면 로그·에러
    /// 문자열에 키가 새어 나갈 수 있다.
    #[test]
    fn models_url_carries_no_api_key() {
        assert!(!MODELS_URL.contains("key="), "{MODELS_URL}");
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_command_passes_instruction_model_and_trust_flags() {
        let spec = build("gemini", "요약 지시", "gemini-2.5-flash");
        assert_eq!(spec.provider, SummaryProvider::Gemini);
        let cmd = spec.command;
        assert_eq!(cmd.get_program(), "gemini");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec![
                "--prompt",
                "요약 지시",
                "--model",
                "gemini-2.5-flash",
                "--output-format",
                "text",
                "--skip-trust",
            ]
        );
    }

    /// 모델은 이제 호출측(`summarizer::resolve_model`)이 정해 넘긴다 — 여기서는
    /// 그 값이 왜곡 없이 커맨드로 실리는지만 고정한다(목적별 기본값·설정
    /// 오버라이드 규칙은 mod.rs의 `resolve_model` 테스트가 지킨다).
    #[cfg(not(windows))]
    #[test]
    fn explicit_model_is_passed_through() {
        let spec = build("gemini", "학습자료 지시", "gemini-2.5-pro");
        let args: Vec<_> = spec
            .command
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert!(args.contains(&"gemini-2.5-pro".to_string()), "{args:?}");
        assert!(!args.contains(&"gemini-2.5-flash".to_string()), "{args:?}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_script_pins_bomless_utf8_output_encoding() {
        assert!(
            WINDOWS_SCRIPT.contains("$OutputEncoding=New-Object System.Text.UTF8Encoding($false)")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_script_reads_stdin_to_eof_before_invoking_provider() {
        let gate = WINDOWS_SCRIPT.find("[Console]::In.ReadToEnd()").unwrap();
        let invocation = WINDOWS_SCRIPT.find("$in | & $c.Source").unwrap();
        assert!(gate < invocation, "{WINDOWS_SCRIPT}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_command_uses_powershell_with_no_window_flag_and_env_instruction() {
        let spec = build("gemini", "요약 지시", "gemini-2.5-flash");
        let cmd = spec.command;
        assert_eq!(cmd.get_program(), "powershell.exe");
        let args: Vec<_> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            vec!["-NoProfile", "-NonInteractive", "-Command", WINDOWS_SCRIPT]
        );
        let env_val = cmd
            .get_envs()
            .find(|(k, _)| *k == "AO_INSTRUCTION")
            .and_then(|(_, v)| v);
        assert_eq!(env_val, Some(std::ffi::OsStr::new("요약 지시")));
        assert!(WINDOWS_SCRIPT.contains("--skip-trust"), "{WINDOWS_SCRIPT}");
    }

    /// 실 API 스모크 — `GEMINI_API_KEY`(또는 `GOOGLE_API_KEY`)가 있을 때만
    /// 의미가 있다. 키가 없으면 빈 목록이 정상 동작이므로 건너뛴다.
    #[tokio::test]
    #[ignore = "실 API 키·네트워크 필요(수동 스모크)"]
    async fn live_catalog_smoke() {
        if API_KEY_ENVS
            .iter()
            .all(|n| crate::api_keys::env_api_key(n).is_none())
        {
            eprintln!("키 없음 -- 건너뜀");
            return;
        }
        let models = list_models(std::time::Duration::from_secs(30)).await;
        assert!(
            !models.is_empty(),
            "빈 목록 -- 응답 형식이 바뀌었을 수 있다"
        );
        println!("{models:?}");
    }
}
