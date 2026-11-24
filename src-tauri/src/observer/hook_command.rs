use std::path::Path;

#[cfg(windows)]
use base64::Engine as _;

use super::ObserverAdapterError;

#[cfg(windows)]
fn powershell_encoded_command(script: &str) -> String {
    let bytes = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect::<Vec<_>>();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

pub(super) fn forwarder_path(executable: &Path) -> Result<&str, ObserverAdapterError> {
    if executable.as_os_str().is_empty() || !executable.is_absolute() {
        return Err(ObserverAdapterError::new(
            "observer forwarder path must be absolute",
        ));
    }
    let path = executable
        .to_str()
        .ok_or_else(|| ObserverAdapterError::new("observer forwarder path must be Unicode"))?;
    if cfg!(windows) && path.contains('"') {
        return Err(ObserverAdapterError::new(
            "observer forwarder path contains a quote",
        ));
    }
    Ok(path)
}

/// Codex는 선택한 훅 셸을 어댑터에 알리지 않는다. 드라이브 절대 경로 중 세 셸에서
/// 확장되지 않는 것만 한 토큰으로 쓰고, Git Bash도 읽도록 경로 구분자를 통일한다.
#[cfg(any(windows, test))]
fn windows_command_token(path: &str) -> Option<String> {
    let bytes = path.as_bytes();
    if bytes.len() < 3
        || !bytes[0].is_ascii_alphabetic()
        || bytes[1] != b':'
        || !matches!(bytes[2], b'/' | b'\\')
    {
        return None;
    }
    let tail = &path[2..];
    if !tail
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '/' | '\\' | '-' | '_' | '.' | '~' | ' '))
    {
        return None;
    }
    let tail = tail.replace('\\', "/");
    let drive = &path[..2];
    // PowerShell에서 첫 토큰이 따옴표로 시작하면 경로가 문자열 식이 된다.
    // 드라이브는 밖에 두고 공백을 포함한 나머지를 한 인용 구간으로 묶는다.
    Some(if tail.contains(' ') {
        format!(r#"{drive}"{tail}""#)
    } else {
        format!("{drive}{tail}")
    })
}

#[cfg(windows)]
fn short_path(path: &str) -> Option<String> {
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;

    let input: Vec<_> = path.encode_utf16().chain([0]).collect();
    // 버퍼가 부족하면 끝의 NUL까지 포함한 필요 길이가 돌아온다.
    let mut buffer = vec![0u16; 260];
    loop {
        let length =
            unsafe { GetShortPathNameW(input.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) }
                as usize;
        if length == 0 {
            return None;
        }
        if length < buffer.len() {
            return String::from_utf16(&buffer[..length]).ok();
        }
        buffer.resize(length, 0);
    }
}

/// `<executable> --observer-forward <args...>`를 훅 셸에서 실행하는 명령 문자열을 만든다.
/// unix는 codex/claude 어댑터가 함께 쓰고, Windows는 codex만 쓴다(claude는 훅의
/// `args` 배열로 forwarder를 직접 실행한다).
///
/// 훅 URL을 명령에 박지 않는 것이 핵심이다: forwarder가 실행 시점에 세션 env의
/// `AGENT_OFFICE_HOOK_URL`을 읽고, 연결이 거부되면 `AGENT_OFFICE_APP_DATA/observer-port`
/// 파일로 1회 재시도한다(이슈 #30). URL을 박으면 재시작 후 입양된 세션이 죽은 포트를 때린다.
pub fn forwarder_shell_command(
    executable: &Path,
    args: &[&str],
) -> Result<String, ObserverAdapterError> {
    let path = forwarder_path(executable)?;
    let forwarded = format!("--observer-forward {}", args.join(" "));
    #[cfg(windows)]
    {
        if let Some(token) = windows_command_token(path)
            .or_else(|| short_path(path).as_deref().and_then(windows_command_token))
        {
            return Ok(format!("{token} {forwarded}"));
        }
        // UNC나 셸 확장 문자가 있고 안전한 별칭도 없으면 기존 호환 호출을 사용한다.
        // PowerShell이 스마트 따옴표까지 구문으로 해석하므로 경로는 데이터로 전달한다.
        let encoded_path = base64::engine::general_purpose::STANDARD.encode(path.as_bytes());
        let script = format!(
            "$ErrorActionPreference='Stop'\n\
             & ([Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{encoded_path}'))) {forwarded}\n\
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
    }
    #[cfg(not(windows))]
    {
        Ok(format!("'{}' {forwarded}", path.replace('\'', "'\"'\"'")))
    }
}

#[cfg(test)]
mod tests {
    use super::windows_command_token;

    #[test]
    fn only_unambiguous_drive_paths_can_run_without_a_shell_wrapper() {
        assert_eq!(
            windows_command_token(r"C:\한글\PROGRA~1\agent-office.exe").as_deref(),
            Some("C:/한글/PROGRA~1/agent-office.exe")
        );
        assert_eq!(
            windows_command_token(r"C:\Program Files\ao.exe").as_deref(),
            Some(r#"C:"/Program Files/ao.exe""#)
        );
        for path in [
            r"C:\%TEMP%\ao.exe",
            r"C:\$var\ao.exe",
            r"C:\a&b\ao.exe",
            r"\\server\share\ao.exe",
            "ao.exe",
        ] {
            assert_eq!(windows_command_token(path), None, "{path}");
        }
    }
}
