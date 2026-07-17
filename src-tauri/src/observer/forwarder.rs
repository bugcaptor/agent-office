use std::io::Read;
use std::time::Duration;

pub fn run_codex_forwarder() -> i32 {
    let session = match std::env::var("AGENT_OFFICE_SESSION") {
        Ok(value) if !value.is_empty() => value,
        _ => return 0,
    };
    let url = match std::env::var("AGENT_OFFICE_HOOK_URL") {
        Ok(value) => value,
        Err(_) => return 0,
    };
    let parsed_url = match parse_local_hook_url(&url) {
        Some(parsed_url) => parsed_url,
        None => return 0,
    };

    let mut body = Vec::new();
    let mut stdin = std::io::stdin();
    if stdin.read_to_end(&mut body).is_err() {
        return 0;
    }
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(_) => return 0,
    };
    let client = match reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(client) => client,
        Err(_) => return 0,
    };

    let primary: Result<reqwest::Response, reqwest::Error> =
        runtime.block_on(post(&client, parsed_url.clone(), &session, &body));

    // §핵심 5(docs/session-handoff-design.md): 세션 env의 AGENT_OFFICE_HOOK_URL은
    // 스폰 시점 포트를 담는다 -- 재시작 후 입양된 세션은 죽은 포트를 친다.
    // 연결 자체가 안 됐을 때만(호스트가 살아있는데 4xx/5xx인 경우는 재시도
    // 대상이 아니다) 최신 포트 파일을 읽어 1회 재시도한다. 이 재시도가
    // 실패해도 베스트에포트 -- observer 알림은 부가 기능이라 종료 코드에
    // 반영하지 않는다(기존 계약 유지).
    if primary.as_ref().err().is_some_and(reqwest::Error::is_connect) {
        if let Some(retry_url) = retry_url_from_port_file(&parsed_url) {
            let _ = runtime.block_on(post(&client, retry_url, &session, &body));
        }
    }
    0
}

async fn post(
    client: &reqwest::Client,
    url: reqwest::Url,
    session: &str,
    body: &[u8],
) -> Result<reqwest::Response, reqwest::Error> {
    client
        .post(url)
        .query(&[("session", session), ("provider", "codex")])
        .body(body.to_vec())
        .send()
        .await
}

/// `http://127.0.0.1:<port>/hook` 형태만 허용한다(루프백·평문·명시적 포트).
/// 프록시/리다이렉트/비루프백 호스트로의 유출을 막는 기존 계약과 동일.
fn parse_local_hook_url(url: &str) -> Option<reqwest::Url> {
    let parsed_url = url.parse::<reqwest::Url>().ok()?;
    if parsed_url.scheme() == "http"
        && parsed_url.host_str() == Some("127.0.0.1")
        && parsed_url.port().is_some()
    {
        Some(parsed_url)
    } else {
        None
    }
}

/// `AGENT_OFFICE_APP_DATA/observer-port`를 읽어, `original`과 같은 스킴/호스트/
/// 경로/쿼리를 유지한 채 포트만 그 파일의 값으로 바꾼 URL을 만든다. env가
/// 없거나 파일이 없거나 파싱에 실패하면 None(재시도 자체를 건너뛴다).
fn retry_url_from_port_file(original: &reqwest::Url) -> Option<reqwest::Url> {
    let app_data_dir = std::env::var("AGENT_OFFICE_APP_DATA").ok()?;
    let port_text =
        std::fs::read_to_string(std::path::Path::new(&app_data_dir).join("observer-port")).ok()?;
    let port: u16 = port_text.trim().parse().ok()?;
    let mut retry_url = original.clone();
    retry_url.set_port(Some(port)).ok()?;
    Some(retry_url)
}

#[cfg(test)]
mod tests {
    use super::*;

    // AGENT_OFFICE_APP_DATA는 프로세스 전역 -- 이를 건드리는 두 테스트가
    // `cargo test`의 스레드 병렬 실행에서 서로 경합하지 않도록 직렬화한다
    // (pty_factory.rs의 PROCESS_ENV_LOCK과 동일 관례).
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn parse_local_hook_url_accepts_loopback_http_with_explicit_port() {
        let parsed = parse_local_hook_url("http://127.0.0.1:4567/hook").unwrap();
        assert_eq!(parsed.host_str(), Some("127.0.0.1"));
        assert_eq!(parsed.port(), Some(4567));
    }

    #[test]
    fn parse_local_hook_url_rejects_non_loopback_and_non_http() {
        assert!(parse_local_hook_url("https://127.0.0.1:4567/hook").is_none());
        assert!(parse_local_hook_url("http://localhost:4567/hook").is_none());
        assert!(parse_local_hook_url("http://127.0.0.1/hook").is_none()); // no explicit port
        assert!(parse_local_hook_url("not a url").is_none());
    }

    #[test]
    fn retry_url_from_port_file_swaps_only_the_port() {
        let _env_guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "agent-office-forwarder-retry-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("observer-port"), "54321\n").unwrap();

        let previous = std::env::var("AGENT_OFFICE_APP_DATA").ok();
        std::env::set_var("AGENT_OFFICE_APP_DATA", &dir);

        let original = "http://127.0.0.1:1/hook?session=s1&provider=codex"
            .parse::<reqwest::Url>()
            .unwrap();
        let retried = retry_url_from_port_file(&original).unwrap();
        assert_eq!(retried.port(), Some(54321));
        assert_eq!(retried.path(), "/hook");
        assert_eq!(retried.query(), Some("session=s1&provider=codex"));

        match previous {
            Some(v) => std::env::set_var("AGENT_OFFICE_APP_DATA", v),
            None => std::env::remove_var("AGENT_OFFICE_APP_DATA"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn retry_url_from_port_file_is_none_without_the_env_or_file() {
        let _env_guard = ENV_LOCK.lock().unwrap();
        let previous = std::env::var("AGENT_OFFICE_APP_DATA").ok();
        std::env::remove_var("AGENT_OFFICE_APP_DATA");

        let original = "http://127.0.0.1:1/hook".parse::<reqwest::Url>().unwrap();
        assert!(retry_url_from_port_file(&original).is_none());

        match previous {
            Some(v) => std::env::set_var("AGENT_OFFICE_APP_DATA", v),
            None => std::env::remove_var("AGENT_OFFICE_APP_DATA"),
        }
    }
}
