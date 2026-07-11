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
    let parsed_url = match url.parse::<reqwest::Url>() {
        Ok(parsed_url)
            if parsed_url.scheme() == "http"
                && parsed_url.host_str() == Some("127.0.0.1")
                && parsed_url.port().is_some() =>
        {
            parsed_url
        }
        _ => return 0,
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
    let _: Result<reqwest::Response, reqwest::Error> = runtime.block_on(async {
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()?
            .post(parsed_url)
            .query(&[("session", session.as_str()), ("provider", "codex")])
            .body(body)
            .send()
            .await
    });
    0
}
