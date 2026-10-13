//! CLI 전환의 launch-bound 반환 receipt 어댑터.
//!
//! receipt는 "이 동기 CLI 호출이 이 셸로 돌아왔다"는 좁은 사실만 보인다.
//! PTY 생존, 출력 idle, foreground process 같은 간접 신호로 다음 CLI를
//! 기동하지 않도록 이 모듈에서 명령 생성과 파일 검증을 함께 소유한다.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const RECEIPT_DIR: &str = "automation-cli-returns";
const RECEIPT_FILE: &str = "return.json";
const STARTED_FILE: &str = "started.json";
const SCRIPT_FILE: &str = "launch.sh";
const PS_SCRIPT_FILE: &str = "launch.ps1";
const MAX_RECEIPT_BYTES: u64 = 4 * 1024;
const MAX_SUBMISSION_BYTES: usize = 512;

/// 앱이 직접 띄운 세션 셸 중 기동 스크립트를 렌더링할 수 있는 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchShell {
    /// Unix bash/zsh.
    Posix,
    /// Windows Git Bash. POSIX 스크립트에 `C:/…` 경로를 쓴다.
    GitBash,
    /// Windows PowerShell 5.1 / PowerShell 7.
    PowerShell,
}

/// CLI 명령 문자열의 인용 문법.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandSyntax {
    Posix,
    PowerShell,
}

impl LaunchShell {
    pub fn syntax(self) -> CommandSyntax {
        match self {
            LaunchShell::Posix | LaunchShell::GitBash => CommandSyntax::Posix,
            LaunchShell::PowerShell => CommandSyntax::PowerShell,
        }
    }

    /// 실제 세션 셸 경로로 판정한다. WSL·cmd·미식별 셸은 None이다.
    pub fn for_shell_path(shell_path: &str) -> Option<Self> {
        let path = Path::new(shell_path);
        let basename = path.file_name()?.to_str()?;
        if cfg!(unix) {
            return (path.is_absolute() && matches!(basename, "bash" | "zsh"))
                .then_some(LaunchShell::Posix);
        }
        if cfg!(windows) {
            // PowerShell은 SystemRoot가 없을 때 `powershell.exe` 이름만으로 뜬다.
            return match basename.to_ascii_lowercase().as_str() {
                "bash.exe" if path.is_absolute() => Some(LaunchShell::GitBash),
                "powershell.exe" | "pwsh.exe" => Some(LaunchShell::PowerShell),
                _ => None,
            };
        }
        None
    }
}

/// UI 미리보기와 실행 전 검증이 공유하는, 실제 세션 셸 기반 capability다.
/// Windows/attach/tmux/미식별 셸은 추측으로 auto를 켜지 않는다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CliTransitionCapability {
    pub shell_path: Option<String>,
    pub auto_return_supported: bool,
    pub unavailable_reason: Option<String>,
}

/// 한 번의 LaunchCli 제출에만 유효한 receipt 정보.
#[derive(Debug, Clone)]
pub struct PreparedLaunch {
    pub launch_id: String,
    pub receipt_path: PathBuf,
    pub started_path: PathBuf,
    canonical_root: PathBuf,
    session_id: String,
    run_id: String,
    launch_step_id: String,
    step_execution_id: String,
    nonce: String,
    pub(crate) rendered_command: String,
}

impl PreparedLaunch {
    /// strict input gate에 넘길, CLI 호출과 반환 기록을 묶은 셸 문장.
    pub fn command(&self) -> &str {
        &self.rendered_command
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptRead {
    Pending,
    Returned { exit_code: i32 },
    Invalid(String),
}

/// `started.json`은 source 된 스크립트가 실제로 실행되었다는 ACK다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartedRead {
    Pending,
    Started,
    Invalid(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReturnReceipt {
    version: u32,
    session_id: String,
    run_id: String,
    launch_id: String,
    launch_step_id: String,
    step_execution_id: String,
    nonce: String,
    exit_code: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartedReceipt {
    version: u32,
    session_id: String,
    run_id: String,
    launch_id: String,
    launch_step_id: String,
    step_execution_id: String,
    nonce: String,
}

/// app-data 아래의 반환 기록 디렉터리를 소유하는 전환 어댑터.
#[derive(Debug, Clone)]
pub struct CliTransition {
    app_data_dir: PathBuf,
}

impl CliTransition {
    pub fn new(app_data_dir: PathBuf) -> Self {
        Self { app_data_dir }
    }

    pub fn capability_for_shell_path(shell_path: Option<&str>) -> CliTransitionCapability {
        let Some(shell_path) = shell_path else {
            return CliTransitionCapability {
                shell_path: None,
                auto_return_supported: false,
                unavailable_reason: Some("automation-cli-return-shell-unverified".into()),
            };
        };
        // 자동화 v2는 Unix bash/zsh만 검증했다. Windows 셸은 업무 보내기만 쓴다.
        let posix_supported = LaunchShell::for_shell_path(shell_path) == Some(LaunchShell::Posix);
        CliTransitionCapability {
            shell_path: Some(shell_path.to_string()),
            auto_return_supported: posix_supported,
            unavailable_reason: (!posix_supported)
                .then_some("automation-cli-return-shell-unsupported".into()),
        }
    }

    /// 앱 소유 디렉터리를 준비하고 POSIX 동기 호출 + atomic receipt writer를 만든다.
    /// 호출자는 먼저 `capability_for_shell_path`로 현재 세션의 지원 여부를 확인한다.
    pub fn prepare_posix_launch(
        &self,
        session_id: &str,
        run_id: &str,
        launch_step_id: &str,
        step_execution_id: &str,
        cli_command: &str,
    ) -> Result<PreparedLaunch, String> {
        self.prepare_launch(
            LaunchShell::Posix,
            session_id,
            run_id,
            launch_step_id,
            step_execution_id,
            cli_command,
        )
    }

    /// `shell` 문법의 동기 호출 + atomic receipt writer를 만든다.
    /// `cli_command`는 같은 셸의 `CommandSyntax`로 인용돼 있어야 한다.
    pub fn prepare_launch(
        &self,
        shell: LaunchShell,
        session_id: &str,
        run_id: &str,
        launch_step_id: &str,
        step_execution_id: &str,
        cli_command: &str,
    ) -> Result<PreparedLaunch, String> {
        for value in [session_id, run_id, step_execution_id] {
            if !valid_identifier(value) {
                return Err("automation-cli-return-identifier-invalid".into());
            }
        }
        if launch_step_id.trim().is_empty() {
            return Err("automation-step-id-invalid".into());
        }
        if cli_command.trim().is_empty() || cli_command.contains('\0') {
            return Err("automation-cli-launch-command-invalid".into());
        }

        let root = canonical_owned_directory(&self.app_data_dir, None)?;
        let returns_root = canonical_owned_directory(&root.join(RECEIPT_DIR), Some(&root))?;
        let run_dir = canonical_owned_directory(&returns_root.join(run_id), Some(&returns_root))?;
        let launch_id = Uuid::new_v4().to_string();
        let launch_dir = canonical_owned_directory(&run_dir.join(&launch_id), Some(&run_dir))?;
        let receipt_path = launch_dir.join(RECEIPT_FILE);
        let started_path = launch_dir.join(STARTED_FILE);
        let script_path = launch_dir.join(match shell {
            LaunchShell::PowerShell => PS_SCRIPT_FILE,
            LaunchShell::Posix | LaunchShell::GitBash => SCRIPT_FILE,
        });
        let nonce = Uuid::new_v4().to_string();

        let receipt_prefix = serde_json::json!({
            "version": 1,
            "sessionId": session_id,
            "runId": run_id,
            "launchId": launch_id,
            "launchStepId": launch_step_id,
            "stepExecutionId": step_execution_id,
            "nonce": nonce,
        })
        .to_string();
        // JSON object의 마지막 }를 exitCode 동적 숫자로 바꾼다. 모든 문자열은
        // serde_json과 POSIX single-quote helper를 거치므로 셸 코드가 되지 않는다.
        let receipt_prefix = receipt_prefix
            .strip_suffix('}')
            .expect("json object ends with brace");
        let receipt_prefix = format!("{receipt_prefix},\"exitCode\":");
        let started_json = serde_json::json!({
            "version": 1, "sessionId": session_id, "runId": run_id,
            "launchId": launch_id, "launchStepId": launch_step_id,
            "stepExecutionId": step_execution_id, "nonce": nonce,
        })
        .to_string();
        let paths = ScriptPaths {
            started_tmp: launch_dir.join(format!(".started-{nonce}.tmp")),
            started: started_path.clone(),
            receipt_tmp: launch_dir.join(format!(".return-{nonce}.tmp")),
            receipt: receipt_path.clone(),
        };
        let (script, rendered_command) = match shell {
            LaunchShell::Posix | LaunchShell::GitBash => {
                let path = |p: &Path| shell_quote(&script_path_text(p, shell));
                (
                    posix_script(&paths, path, &started_json, &receipt_prefix, cli_command),
                    format!(". {}", path(&script_path)),
                )
            }
            LaunchShell::PowerShell => {
                let path = |p: &Path| ps_quote(&script_path_text(p, shell));
                // 실행 정책은 스크립트 파일에만 적용된다. 문자열로 만든 scriptblock을
                // `&`로 부르면 정책과 무관하고, 자식 스코프라 변수·cd가 새지 않는다.
                (
                    powershell_script(&paths, path, &started_json, &receipt_prefix, cli_command),
                    format!(
                        "& ([scriptblock]::Create([System.IO.File]::ReadAllText({})))",
                        path(&script_path)
                    ),
                )
            }
        };
        write_atomic_script(&script_path, &script)?;
        if rendered_command.as_bytes().len() + 1 > MAX_SUBMISSION_BYTES {
            return Err("automation-cli-launch-source-too-long".into());
        }

        Ok(PreparedLaunch {
            launch_id,
            receipt_path,
            started_path,
            canonical_root: root,
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            launch_step_id: launch_step_id.to_string(),
            step_execution_id: step_execution_id.to_string(),
            nonce,
            rendered_command,
        })
    }

    /// receipt를 소비하지 않고 읽는다. 실행기는 `Returned`를 한 번만 전이로
    /// 소비해야 하며, 늦은/중복 poll 자체는 다음 기동의 근거가 될 수 없다.
    pub fn poll_receipt(&self, launch: &PreparedLaunch) -> ReceiptRead {
        self.poll_file(launch, &launch.receipt_path, RECEIPT_FILE, |bytes| {
            let receipt: ReturnReceipt = serde_json::from_slice(bytes).map_err(|_| ())?;
            if receipt.version != 1
                || receipt.session_id != launch.session_id
                || receipt.run_id != launch.run_id
                || receipt.launch_id != launch.launch_id
                || receipt.launch_step_id != launch.launch_step_id
                || receipt.step_execution_id != launch.step_execution_id
                || receipt.nonce != launch.nonce
            {
                return Err(());
            }
            Ok(receipt.exit_code)
        })
        .map_or_else(
            |e| ReceiptRead::Invalid(e),
            |v| match v {
                None => ReceiptRead::Pending,
                Some(code) => ReceiptRead::Returned { exit_code: code },
            },
        )
    }

    pub fn poll_started(&self, launch: &PreparedLaunch) -> StartedRead {
        self.poll_file(launch, &launch.started_path, STARTED_FILE, |bytes| {
            let receipt: StartedReceipt = serde_json::from_slice(bytes).map_err(|_| ())?;
            if receipt.version != 1
                || receipt.session_id != launch.session_id
                || receipt.run_id != launch.run_id
                || receipt.launch_id != launch.launch_id
                || receipt.launch_step_id != launch.launch_step_id
                || receipt.step_execution_id != launch.step_execution_id
                || receipt.nonce != launch.nonce
            {
                return Err(());
            }
            Ok(())
        })
        .map_or_else(
            |e| StartedRead::Invalid(e),
            |v| {
                if v.is_some() {
                    StartedRead::Started
                } else {
                    StartedRead::Pending
                }
            },
        )
    }

    fn poll_file<T>(
        &self,
        launch: &PreparedLaunch,
        file: &Path,
        expected_name: &str,
        decode: impl FnOnce(&[u8]) -> Result<T, ()>,
    ) -> Result<Option<T>, String> {
        // Preparation is not a lasting path guarantee: recheck every owned ancestor.
        let expected_parent = launch
            .canonical_root
            .join(RECEIPT_DIR)
            .join(&launch.run_id)
            .join(&launch.launch_id);
        if file.parent() != Some(expected_parent.as_path())
            || file.file_name().and_then(|n| n.to_str()) != Some(expected_name)
        {
            return Err("automation-cli-return-path-escape".into());
        }
        let mut path = launch.canonical_root.clone();
        for child in [
            None,
            Some(RECEIPT_DIR),
            Some(launch.run_id.as_str()),
            Some(launch.launch_id.as_str()),
        ] {
            if let Some(child) = child {
                path.push(child);
            }
            let Ok(meta) = fs::symlink_metadata(&path) else {
                return Err("automation-cli-return-root-invalid".into());
            };
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err("automation-cli-return-symlink".into());
            }
            if fs::canonicalize(&path).ok().as_ref() != Some(&path) {
                return Err("automation-cli-return-path-escape".into());
            }
        }
        let metadata = match fs::symlink_metadata(file) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err("automation-cli-return-read-failed".into()),
        };
        if metadata.file_type().is_symlink() {
            return Err("automation-cli-return-symlink".into());
        }
        if !metadata.is_file() || metadata.len() > MAX_RECEIPT_BYTES {
            return Err("automation-cli-return-file-invalid".into());
        }
        let mut bytes = Vec::new();
        let read = fs::File::open(file)
            .and_then(|file| file.take(MAX_RECEIPT_BYTES + 1).read_to_end(&mut bytes));
        if read.is_err() {
            return Err("automation-cli-return-read-failed".into());
        }
        if bytes.len() as u64 > MAX_RECEIPT_BYTES {
            return Err("automation-cli-return-file-invalid".into());
        }
        match decode(&bytes) {
            Ok(value) => Ok(Some(value)),
            Err(_) => Err("automation-cli-return-mismatch".into()),
        }
    }
}

fn write_atomic_script(path: &Path, contents: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    fs::write(&tmp, contents).map_err(|_| "automation-cli-launch-script-write-failed")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o700))
            .map_err(|_| "automation-cli-launch-script-write-failed")?;
    }
    fs::rename(&tmp, path).map_err(|_| "automation-cli-launch-script-write-failed")?;
    Ok(())
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// PowerShell 작은따옴표 문자열. 안쪽 `'`만 두 번 쓴다.
pub(crate) fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

struct ScriptPaths {
    started_tmp: PathBuf,
    started: PathBuf,
    receipt_tmp: PathBuf,
    receipt: PathBuf,
}

/// 셸에 넘길 경로 문자열. Windows의 canonical `\\?\` 접두사는 셸 도구가
/// 다루지 못하므로 떼고, Git Bash에는 `C:/…` 형식으로 넘긴다.
fn script_path_text(path: &Path, shell: LaunchShell) -> String {
    let text = path.to_string_lossy();
    let text = match shell {
        LaunchShell::Posix => return text.into_owned(),
        LaunchShell::GitBash | LaunchShell::PowerShell => strip_verbatim_prefix(&text),
    };
    match shell {
        LaunchShell::GitBash => text.replace('\\', "/"),
        _ => text,
    }
}

fn strip_verbatim_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
}

fn posix_script(
    paths: &ScriptPaths,
    path: impl Fn(&Path) -> String,
    started_json: &str,
    receipt_prefix: &str,
    cli_command: &str,
) -> String {
    let started_tmp_q = path(&paths.started_tmp);
    let started_q = path(&paths.started);
    let temp_q = path(&paths.receipt_tmp);
    let receipt_q = path(&paths.receipt);
    let started_json_q = shell_quote(started_json);
    let prefix_q = shell_quote(receipt_prefix);
    // Sourcing must be short enough for canonical-mode PTYs. The subshell
    // inherits shell functions (claude/agy wrappers) but isolates cd/env/temp vars.
    format!(
        "( __ao_started_tmp={started_tmp_q}; printf '%s\\n' {started_json_q} > \"$__ao_started_tmp\" && mv -f -- \"$__ao_started_tmp\" {started_q}; __ao_started_rc=$?; if [ \"$__ao_started_rc\" -ne 0 ]; then printf '%s\\n' 'agent-office: automation launch ACK write failed' >&2; exit 1; fi; {cli_command}; __ao_cli_rc=$?; __ao_receipt_tmp={temp_q}; printf '%s%s%s\\n' {prefix_q} \"$__ao_cli_rc\" '}}' > \"$__ao_receipt_tmp\" && mv -f -- \"$__ao_receipt_tmp\" {receipt_q}; __ao_receipt_rc=$?; if [ \"$__ao_receipt_rc\" -ne 0 ]; then printf '%s\\n' 'agent-office: automation return receipt write failed' >&2; fi )\n"
    )
}

/// PowerShell 5.1의 `Set-Content -Encoding UTF8`은 BOM을 붙여 serde가 거절하므로
/// BOM 없는 UTF8Encoding으로 쓴다. 래퍼 함수(claude 등)는 전역이라 그대로 보인다.
/// scriptblock 스코프는 현재 위치를 격리하지 않으므로 Push/Pop-Location으로 감싼다.
fn powershell_script(
    paths: &ScriptPaths,
    path: impl Fn(&Path) -> String,
    started_json: &str,
    receipt_prefix: &str,
    cli_command: &str,
) -> String {
    let started_tmp_q = path(&paths.started_tmp);
    let started_q = path(&paths.started);
    let temp_q = path(&paths.receipt_tmp);
    let receipt_q = path(&paths.receipt);
    let started_json_q = ps_quote(started_json);
    let prefix_q = ps_quote(receipt_prefix);
    format!(
        "$__ao_enc = New-Object System.Text.UTF8Encoding $false\n\
try {{ [System.IO.File]::WriteAllText({started_tmp_q}, {started_json_q} + \"`n\", $__ao_enc); Move-Item -LiteralPath {started_tmp_q} -Destination {started_q} -Force -ErrorAction Stop }} catch {{ [Console]::Error.WriteLine('agent-office: automation launch ACK write failed'); return }}\n\
$global:LASTEXITCODE = 0\n\
Push-Location\n\
try {{\n\
{cli_command}\n\
$__ao_ok = $?\n\
$__ao_rc = $global:LASTEXITCODE\n\
}} finally {{ Pop-Location }}\n\
if ($null -eq $__ao_rc) {{ $__ao_rc = 0 }}\n\
if (-not $__ao_ok -and $__ao_rc -eq 0) {{ $__ao_rc = 1 }}\n\
try {{ [System.IO.File]::WriteAllText({temp_q}, {prefix_q} + $__ao_rc + '}}' + \"`n\", $__ao_enc); Move-Item -LiteralPath {temp_q} -Destination {receipt_q} -Force -ErrorAction Stop }} catch {{ [Console]::Error.WriteLine('agent-office: automation return receipt write failed') }}\n"
    )
}

/// 새 디렉터리도 기존 디렉터리도 symbolic link가 아니고 기준 루트 밖으로 나가지
/// 않는지 확인한다. receipt 자체도 poll마다 별도로 검증한다.
fn canonical_owned_directory(path: &Path, parent: Option<&Path>) -> Result<PathBuf, String> {
    if path.exists()
        && fs::symlink_metadata(path)
            .map_err(|_| "automation-cli-return-root-invalid")?
            .file_type()
            .is_symlink()
    {
        return Err("automation-cli-return-symlink".into());
    }
    fs::create_dir_all(path).map_err(|_| "automation-cli-return-root-create-failed")?;
    let metadata = fs::symlink_metadata(path).map_err(|_| "automation-cli-return-root-invalid")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("automation-cli-return-root-invalid".into());
    }
    let canonical = fs::canonicalize(path).map_err(|_| "automation-cli-return-root-invalid")?;
    if let Some(parent) = parent {
        if canonical.parent() != Some(parent) {
            return Err("automation-cli-return-path-escape".into());
        }
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn adapter() -> (tempfile::TempDir, CliTransition) {
        let dir = tempfile::tempdir().unwrap();
        let app_data = dir.path().join("앱 데이터 with spaces");
        fs::create_dir_all(&app_data).unwrap();
        (dir, CliTransition::new(app_data))
    }

    fn prepared(adapter: &CliTransition) -> PreparedLaunch {
        adapter
            .prepare_posix_launch(
                "session_1",
                "run-1",
                "launch-1",
                "step-1",
                "printf 'cli argument: %s\\n' '한글 path with spaces'",
            )
            .unwrap()
    }

    #[test]
    fn capability_only_enables_direct_posix_bash_or_zsh() {
        assert_eq!(
            CliTransition::capability_for_shell_path(Some("/bin/bash")).auto_return_supported,
            cfg!(unix)
        );
        assert_eq!(
            CliTransition::capability_for_shell_path(Some("/bin/zsh")).auto_return_supported,
            cfg!(unix)
        );
        assert!(!CliTransition::capability_for_shell_path(Some("/bin/sh")).auto_return_supported);
        assert!(!CliTransition::capability_for_shell_path(None).auto_return_supported);
    }

    #[test]
    fn rejects_invalid_identifiers_and_receipt_symlink() {
        let (_dir, adapter) = adapter();
        assert!(adapter
            .prepare_posix_launch("bad/id", "run-1", "launch-1", "step-1", "true")
            .is_err());

        let launch = prepared(&adapter);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp", &launch.receipt_path).unwrap();
            assert_eq!(
                adapter.poll_receipt(&launch),
                ReceiptRead::Invalid("automation-cli-return-symlink".into())
            );
        }
    }

    #[test]
    fn rejects_oversized_and_mismatched_receipts() {
        let (_dir, adapter) = adapter();
        let launch = prepared(&adapter);
        fs::write(
            &launch.receipt_path,
            vec![b'x'; MAX_RECEIPT_BYTES as usize + 1],
        )
        .unwrap();
        assert_eq!(
            adapter.poll_receipt(&launch),
            ReceiptRead::Invalid("automation-cli-return-file-invalid".into())
        );
        fs::write(&launch.receipt_path, b"{}").unwrap();
        assert_eq!(
            adapter.poll_receipt(&launch),
            ReceiptRead::Invalid("automation-cli-return-mismatch".into())
        );
    }

    #[test]
    fn bash_and_zsh_write_atomic_receipts_with_the_cli_exit_code() {
        for shell in ["/bin/bash", "/bin/zsh"] {
            if !Path::new(shell).is_file() {
                continue;
            }
            let (_dir, adapter) = adapter();
            let launch = adapter
                .prepare_posix_launch("session_1", "run-1", "launch-1", "step-1", "false")
                .unwrap();
            let status = Command::new(shell)
                .arg("-c")
                .arg(launch.command())
                .status()
                .unwrap();
            assert!(status.success(), "{shell} receipt wrapper itself failed");
            assert_eq!(
                adapter.poll_receipt(&launch),
                ReceiptRead::Returned { exit_code: 1 }
            );
        }
    }

    #[test]
    fn short_source_command_runs_in_a_subshell_with_shell_functions_and_ack() {
        for shell in ["/bin/bash", "/bin/zsh"] {
            if !Path::new(shell).is_file() {
                continue;
            }
            let (dir, adapter) = adapter();
            let output = dir.path().join("function-observation");
            let command = "ao_fake_cli";
            let launch = adapter
                .prepare_posix_launch("s", "r", "l", "e", command)
                .unwrap();
            assert!(launch.command().as_bytes().len() + 1 <= MAX_SUBMISSION_BYTES);
            let fake = format!(
                "__ao_before=$PWD; ao_fake_cli() {{ [ -f {} ] && printf '%s' ok > {}; cd /; export AO_LEAK=bad; }}; {} ; [ \"$PWD\" = \"$__ao_before\" ] && [ \"${{AO_LEAK-unset}}\" = unset ]",
                shell_quote(&launch.started_path.to_string_lossy()), shell_quote(&output.to_string_lossy()), launch.command()
            );
            assert!(
                Command::new(shell)
                    .arg("-c")
                    .arg(fake)
                    .status()
                    .unwrap()
                    .success(),
                "{shell}"
            );
            assert_eq!(fs::read_to_string(output).unwrap(), "ok");
            assert_eq!(adapter.poll_started(&launch), StartedRead::Started);
            assert_eq!(
                adapter.poll_receipt(&launch),
                ReceiptRead::Returned { exit_code: 0 }
            );
        }
    }

    #[test]
    fn started_ack_rejects_a_stale_launch_identity() {
        let (_dir, adapter) = adapter();
        let launch = prepared(&adapter);
        fs::write(
            &launch.started_path,
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "sessionId": "stale", "runId": launch.run_id,
                "launchId": launch.launch_id, "launchStepId": launch.launch_step_id,
                "stepExecutionId": launch.step_execution_id, "nonce": launch.nonce,
            }))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            adapter.poll_started(&launch),
            StartedRead::Invalid("automation-cli-return-mismatch".into())
        );
    }
    #[cfg(unix)]
    #[test]
    fn receipt_identity_and_parent_replacement_are_rejected() {
        let (_dir, adapter) = adapter();
        let launch = adapter
            .prepare_posix_launch("s", "r", "l", "e", "true")
            .unwrap();
        assert!(Command::new("/bin/bash")
            .arg("-c")
            .arg(launch.command())
            .status()
            .unwrap()
            .success());
        let original = fs::read(&launch.receipt_path).unwrap();
        for field in [
            "sessionId",
            "runId",
            "launchId",
            "launchStepId",
            "stepExecutionId",
            "nonce",
        ] {
            let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
            value[field] = "stale".into();
            fs::write(&launch.receipt_path, serde_json::to_vec(&value).unwrap()).unwrap();
            assert_eq!(
                adapter.poll_receipt(&launch),
                ReceiptRead::Invalid("automation-cli-return-mismatch".into()),
                "{field}"
            );
        }
        fs::write(&launch.receipt_path, &original).unwrap();
        let parent = launch.receipt_path.parent().unwrap();
        let moved = parent.with_extension("moved");
        fs::rename(parent, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, parent).unwrap();
        assert_eq!(
            adapter.poll_receipt(&launch),
            ReceiptRead::Invalid("automation-cli-return-symlink".into())
        );
    }

    #[cfg(unix)]
    #[test]
    fn shell_writer_preserves_quoted_arguments_and_launch_identity() {
        for shell in ["/bin/bash", "/bin/zsh"] {
            if !Path::new(shell).is_file() {
                continue;
            }
            let (dir, adapter) = adapter();
            let output = dir.path().join("인자 ' output.txt");
            let value = "한글 path ' with $ticks `and` spaces";
            let command = format!(
                "printf '%s' {} > {}",
                shell_quote(value),
                shell_quote(output.to_str().unwrap())
            );
            let first = adapter
                .prepare_posix_launch("s", "r", "Claude 기동.1", "e1", &command)
                .unwrap();
            let second = adapter
                .prepare_posix_launch("s", "r", "l", "e2", &command)
                .unwrap();
            assert_ne!(first.launch_id, second.launch_id);
            assert!(Command::new(shell)
                .arg("-c")
                .arg(first.command())
                .status()
                .unwrap()
                .success());
            assert_eq!(fs::read_to_string(&output).unwrap(), value);
            assert_eq!(
                adapter.poll_receipt(&first),
                ReceiptRead::Returned { exit_code: 0 }
            );
            assert_eq!(adapter.poll_receipt(&second), ReceiptRead::Pending);
        }
    }

    #[test]
    fn launch_shell_kind_follows_platform_and_executable_name() {
        let expect = |path: &str, unix: Option<LaunchShell>, windows: Option<LaunchShell>| {
            let expected = if cfg!(windows) { windows } else { unix };
            assert_eq!(LaunchShell::for_shell_path(path), expected, "{path}");
        };
        expect("/bin/zsh", Some(LaunchShell::Posix), None);
        expect("bash", None, None);
        expect(r"C:\Program Files\Git\bin\bash.exe", None, Some(LaunchShell::GitBash));
        expect(r"C:\Program Files\PowerShell\7\PWSH.EXE", None, Some(LaunchShell::PowerShell));
        expect("powershell.exe", None, Some(LaunchShell::PowerShell));
        expect(r"C:\Windows\System32\wsl.exe", None, None);
        // 자동화 v2 capability는 Windows 셸을 켜지 않는다.
        assert!(
            !CliTransition::capability_for_shell_path(Some(r"C:\Program Files\Git\bin\bash.exe"))
                .auto_return_supported
        );
        assert_eq!(LaunchShell::GitBash.syntax(), CommandSyntax::Posix);
        assert_eq!(LaunchShell::PowerShell.syntax(), CommandSyntax::PowerShell);
    }

    #[test]
    fn windows_script_paths_drop_the_verbatim_prefix() {
        let path = Path::new(r"\\?\C:\Users\앱 데이터\launch.sh");
        assert_eq!(script_path_text(path, LaunchShell::GitBash), "C:/Users/앱 데이터/launch.sh");
        assert_eq!(
            script_path_text(path, LaunchShell::PowerShell),
            r"C:\Users\앱 데이터\launch.sh"
        );
        assert_eq!(
            script_path_text(Path::new(r"\\?\UNC\host\share\x"), LaunchShell::PowerShell),
            r"\\host\share\x"
        );
        assert_eq!(ps_quote("it's"), "'it''s'");
    }

    #[cfg(windows)]
    fn windows_adapter() -> (tempfile::TempDir, CliTransition) {
        let dir = tempfile::tempdir().unwrap();
        let app_data = dir.path().join("앱 데이터 it's");
        fs::create_dir_all(&app_data).unwrap();
        (dir, CliTransition::new(app_data))
    }

    #[cfg(windows)]
    fn powershell_programs() -> Vec<String> {
        let mut programs = vec![];
        if let Ok(root) = std::env::var("SystemRoot") {
            programs.push(format!(r"{root}\System32\WindowsPowerShell\v1.0\powershell.exe"));
        }
        if let Ok(pf) = std::env::var("ProgramFiles") {
            programs.push(format!(r"{pf}\PowerShell\7\pwsh.exe"));
        }
        programs.into_iter().filter(|p| Path::new(p).is_file()).collect()
    }

    #[cfg(windows)]
    #[test]
    fn powershell_launch_runs_wrapper_functions_in_a_child_scope_and_writes_receipts() {
        for program in powershell_programs() {
            let (dir, adapter) = windows_adapter();
            let output = dir.path().join("인자 ' output.txt");
            let command = format!(
                "ao_fake_cli --model {} -c {}",
                ps_quote("한글 ' $model"),
                ps_quote("model_reasoning_effort=high")
            );
            let launch = adapter
                .prepare_launch(LaunchShell::PowerShell, "s", "r", "l", "e", &command)
                .unwrap();
            assert!(launch.command().as_bytes().len() + 1 <= MAX_SUBMISSION_BYTES);
            assert!(launch.command().starts_with("& ([scriptblock]::Create("));
            let script = format!(
                "$before = (Get-Location).Path; function ao_fake_cli {{ [System.IO.File]::WriteAllText({}, ($args -join '|')); Set-Location \\; cmd /c exit 3 }}; {}; if ((Get-Location).Path -ne $before -or $null -ne $__ao_rc) {{ exit 9 }}",
                ps_quote(&output.to_string_lossy()),
                launch.command()
            );
            let status = Command::new(&program)
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .status()
                .unwrap();
            assert!(status.success(), "{program}: {status}");
            assert_eq!(
                fs::read_to_string(&output).unwrap(),
                "--model|한글 ' $model|-c|model_reasoning_effort=high"
            );
            assert_eq!(adapter.poll_started(&launch), StartedRead::Started, "{program}");
            assert_eq!(
                adapter.poll_receipt(&launch),
                ReceiptRead::Returned { exit_code: 3 },
                "{program}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn powershell_missing_cli_reports_a_failed_return() {
        for program in powershell_programs() {
            let (_dir, adapter) = windows_adapter();
            let launch = adapter
                .prepare_launch(LaunchShell::PowerShell, "s", "r", "l", "e", "ao_missing_cli_xyz")
                .unwrap();
            Command::new(&program)
                .args(["-NoProfile", "-NonInteractive", "-Command", launch.command()])
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert_eq!(
                adapter.poll_receipt(&launch),
                ReceiptRead::Returned { exit_code: 1 },
                "{program}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn git_bash_launch_uses_forward_slash_windows_paths() {
        let Some(bash) = std::env::var("ProgramFiles")
            .ok()
            .map(|pf| format!(r"{pf}\Git\bin\bash.exe"))
            .filter(|p| Path::new(p).is_file())
        else {
            return;
        };
        let (_dir, adapter) = windows_adapter();
        let launch = adapter
            .prepare_launch(LaunchShell::GitBash, "s", "r", "l", "e", "ao_fake_cli 'a b'")
            .unwrap();
        assert!(
            launch.command().starts_with(". '")
                && launch.command().contains(":/")
                && !launch.command().contains(":\\")
                && !launch.command().contains("?"),
            "{}",
            launch.command()
        );
        let status = Command::new(&bash)
            .args(["--norc", "--noprofile", "-c"])
            .arg(format!("ao_fake_cli() {{ [ \"$1\" = 'a b' ] && return 4; }}; {}", launch.command()))
            .status()
            .unwrap();
        assert!(status.success(), "{status}");
        assert_eq!(adapter.poll_started(&launch), StartedRead::Started);
        assert_eq!(adapter.poll_receipt(&launch), ReceiptRead::Returned { exit_code: 4 });
    }
}
