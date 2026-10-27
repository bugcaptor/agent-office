use std::path::Path;

use base64::Engine as _;

use super::ObserverAdapterError;

fn powershell_encoded_command(script: &str) -> String {
    let bytes = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect::<Vec<_>>();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// `<executable> --observer-forward <args...>`를 현재 OS 셸에서 실행하는 훅 명령
/// 문자열을 만든다. codex/claude 어댑터가 공유한다.
///
/// 훅 URL을 명령에 박아 넣지 않는 것이 핵심이다: 앱 바이너리 forwarder를 경유하면
/// forwarder가 실행 시점에 세션 env의 `AGENT_OFFICE_HOOK_URL`을 읽고, 연결이
/// 거부되면 `AGENT_OFFICE_APP_DATA/observer-port` 파일로 1회 재시도한다
/// (이슈 #30). URL을 명령에 박으면
/// 재시작 후 입양된 세션이 죽은(스폰 시점) 포트를 계속 때린다.
///
/// codex는 `["codex"]`, claude는 `["claude", "<EventName>"]`로 위임한다.
/// `["codex"]` 인자로 부르면 이전 구현과 **바이트 단위로 동일한** 문자열을 낸다.
pub fn forwarder_shell_command(
    executable: &Path,
    args: &[&str],
) -> Result<String, ObserverAdapterError> {
    if executable.as_os_str().is_empty() || !executable.is_absolute() {
        return Err(ObserverAdapterError::new(
            "observer forwarder path must be absolute",
        ));
    }
    let path = executable
        .to_str()
        .ok_or_else(|| ObserverAdapterError::new("observer forwarder path must be Unicode"))?;
    if cfg!(windows) {
        if path.contains('"') {
            return Err(ObserverAdapterError::new(
                "observer forwarder path contains a quote",
            ));
        }
        let path = path.replace('\'', "''");
        let forwarded = std::iter::once("--observer-forward")
            .chain(args.iter().copied())
            .map(|arg| format!("'{}'", arg.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "$ErrorActionPreference='Stop'\n\
             & '{path}' {forwarded}\n\
             $forwarderSucceeded=$?\n\
             $forwarderExit=$LASTEXITCODE\n\
             if ($null -ne $forwarderExit) {{ exit $forwarderExit }}\n\
             if ($forwarderSucceeded) {{ exit 0 }}\n\
             exit 1"
        );
        let encoded = powershell_encoded_command(&script);
        Ok(format!(
            "powershell.exe -NoProfile -NonInteractive -EncodedCommand {encoded}"
        ))
    } else {
        let forwarded = std::iter::once("--observer-forward")
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        Ok(format!("'{}' {forwarded}", path.replace('\'', "'\"'\"'")))
    }
}
