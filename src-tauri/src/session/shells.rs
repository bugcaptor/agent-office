// src-tauri/src/session/shells.rs
//
// Windows 셸 선택 기능: 사용자가 프로필/세션 단위로 Windows PowerShell,
// PowerShell 7(pwsh), Git Bash, WSL 중 하나를 고를 수 있게 한다. 이 모듈이
// (1) 호스트에 설치된 셸을 탐지하고(`detect_shells*`), (2) 선택된(또는
// 미선택 시 자동) 셸을 실제 spawn 가능한 프로그램/인자로 변환한다
// (`resolve*`).
//
// `ShellProbe` 트레잇은 파일시스템/env/외부 명령 실행이라는 부작용 경계를
// 감싼다 -- `pty_factory.rs`의 `PtyFactory`, `zsh_wrapper.rs`가 테스트를
// 위해 부작용을 트레잇 뒤로 숨기는 것과 같은 패턴. 프로덕션은 `RealProbe`,
// 테스트는 `FakeProbe`(아래 tests 모듈)를 주입한다.
//
// Adapter-provided wrapper specs are rendered for PowerShell, Git Bash, and
// zsh without hard-coding a provider in shell selection.

use crate::observer::CommandWrapperSpec;

/// 탐지된 셸 1개. 렌더러 드롭다운에 그대로 보낸다(list_available_shells).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableShell {
    pub id: String,            // "powershell" | "pwsh" | "git-bash" | "wsl"
    pub label: String,         // 드롭다운 표시용
    pub path: String,          // 실제 실행 프로그램(절대 경로 또는 "wsl.exe")
    pub hooks_supported: bool, // wsl은 MVP에서 false
}

/// 실제로 spawn할 프로그램/인자/추가 env.
pub struct ResolvedShell {
    pub program: String,
    pub args: Vec<String>,
    pub extra_env: Vec<(String, String)>,
}

/// 탐지/해석이 의존하는 부작용을 감싸는 트레잇 -- 테스트는 `FakeProbe`를
/// 주입해 실제 파일시스템/프로세스 없이 로직만 검증한다.
pub trait ShellProbe: Send + Sync {
    fn exists(&self, path: &str) -> bool;
    fn program_files(&self) -> Option<String>;
    fn program_files_x86(&self) -> Option<String>;
    fn system_root(&self) -> Option<String>;
    /// 프로브용 명령 실행 결과의 stdout(예: `wsl -l -q`). 실행/디코드
    /// 실패 시 None.
    fn command_stdout(&self, program: &str, args: &[&str]) -> Option<String>;
}

/// 프로덕션 `ShellProbe`. `std::fs::metadata`로 존재 확인, `std::env::var`로
/// 환경변수 조회, `std::process::Command`로 프로브 명령 실행(Windows는
/// 콘솔 창이 튀지 않도록 CREATE_NO_WINDOW).
pub struct RealProbe;

impl ShellProbe for RealProbe {
    fn exists(&self, path: &str) -> bool {
        std::fs::metadata(path).is_ok()
    }
    fn program_files(&self) -> Option<String> {
        std::env::var("ProgramFiles").ok()
    }
    fn program_files_x86(&self) -> Option<String> {
        std::env::var("ProgramFiles(x86)").ok()
    }
    fn system_root(&self) -> Option<String> {
        std::env::var("SystemRoot").ok()
    }
    fn command_stdout(&self, program: &str, args: &[&str]) -> Option<String> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let output = cmd.output().ok()?;
        if !output.status.success() {
            return None;
        }
        Some(decode_probe_stdout(&output.stdout))
    }
}

pub(crate) trait ObserverShimWriter: Send + Sync {
    fn bashrc(&self, wrappers: &[CommandWrapperSpec]) -> std::io::Result<std::path::PathBuf>;
    fn zdotdir(&self, wrappers: &[CommandWrapperSpec]) -> std::io::Result<std::path::PathBuf>;
}

struct RealObserverShimWriter;

impl ObserverShimWriter for RealObserverShimWriter {
    fn bashrc(&self, wrappers: &[CommandWrapperSpec]) -> std::io::Result<std::path::PathBuf> {
        crate::session::bash_wrapper::write_observer_shim(
            &std::env::temp_dir().join("agent-office"),
            wrappers,
        )
    }

    fn zdotdir(&self, wrappers: &[CommandWrapperSpec]) -> std::io::Result<std::path::PathBuf> {
        crate::session::zsh_wrapper::write_observer_shim(
            &std::env::temp_dir().join("agent-office").join("zdotdir"),
            wrappers,
        )
    }
}

/// 프로브 명령의 raw stdout 바이트를 문자열로 디코드한다. `wsl -l -q`는
/// UTF-16LE로 출력하므로(BOM `FF FE`가 있거나, ASCII 텍스트가 바이트마다
/// NUL과 교차하는 패턴) 이를 감지해 UTF-16LE로 디코드하고, 아니면 평범한
/// UTF-8(lossy)로 취급한다. `where`류 명령은 보통 순수 UTF-8/ANSI라
/// 그대로 통과한다.
pub fn decode_probe_stdout(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
        return utf16le_to_string(&bytes[2..]);
    }
    if looks_like_utf16le(bytes) {
        return utf16le_to_string(bytes);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// NUL이 홀수(리틀엔디안 UTF-16의 상위 바이트) 위치에 몰려 나타나는지로
/// UTF-16LE 여부를 어림잡는다. BOM이 없는 `wsl -l -q` 출력 대비.
fn looks_like_utf16le(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || !bytes.len().is_multiple_of(2) {
        return false;
    }
    let sample_len = bytes.len().min(64);
    let mut odd_zero = 0usize;
    let mut odd_total = 0usize;
    let mut i = 1;
    while i < sample_len {
        odd_total += 1;
        if bytes[i] == 0 {
            odd_zero += 1;
        }
        i += 2;
    }
    odd_total > 0 && odd_zero * 2 >= odd_total
}

fn utf16le_to_string(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
        .chars()
        .filter(|&c| c != '\0')
        .collect()
}

/// PowerShell `-EncodedCommand` payload: Base64 of UTF-16LE bytes.
#[cfg(windows)]
fn encoded_command(script: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = script
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}

#[cfg(windows)]
fn powershell_path(probe: &dyn ShellProbe) -> String {
    if let Some(root) = probe.system_root() {
        let candidate = format!(r"{root}\System32\WindowsPowerShell\v1.0\powershell.exe");
        if probe.exists(&candidate) {
            return candidate;
        }
    }
    "powershell.exe".to_string()
}

#[cfg(windows)]
fn find_pwsh(probe: &dyn ShellProbe) -> Option<String> {
    if let Some(pf) = probe.program_files() {
        let candidate = format!(r"{pf}\PowerShell\7\pwsh.exe");
        if probe.exists(&candidate) {
            return Some(candidate);
        }
        let preview = format!(r"{pf}\PowerShell\7-preview\pwsh.exe");
        if probe.exists(&preview) {
            return Some(preview);
        }
    }
    let out = probe.command_stdout("where", &["pwsh"])?;
    let first = out.lines().map(str::trim).find(|l| !l.is_empty())?;
    if first.to_lowercase().ends_with("pwsh.exe") && probe.exists(first) {
        Some(first.to_string())
    } else {
        None
    }
}

#[cfg(windows)]
fn find_git_bash(probe: &dyn ShellProbe) -> Option<String> {
    if let Some(pf) = probe.program_files() {
        let candidate = format!(r"{pf}\Git\bin\bash.exe");
        if probe.exists(&candidate) {
            return Some(candidate);
        }
    }
    if let Some(pf86) = probe.program_files_x86() {
        let candidate = format!(r"{pf86}\Git\bin\bash.exe");
        if probe.exists(&candidate) {
            return Some(candidate);
        }
    }
    let out = probe.command_stdout("where", &["git"])?;
    let first = out.lines().map(str::trim).find(|l| !l.is_empty())?;
    let prefix = first.strip_suffix(r"\cmd\git.exe")?;
    let candidate = format!(r"{prefix}\bin\bash.exe");
    if probe.exists(&candidate) {
        Some(candidate)
    } else {
        None
    }
}

/// `{system_root}\System32\wsl.exe`의 경로(존재 여부는 호출부가 확인).
#[cfg(windows)]
fn wsl_exe_path(probe: &dyn ShellProbe) -> Option<String> {
    probe
        .system_root()
        .map(|root| format!(r"{root}\System32\wsl.exe"))
}

/// 탐지(detect_shells_with) 전용: `wsl.exe` 존재 + `wsl -l -q`가 배포판
/// 1개 이상을 보고하는지까지 확인한다(설치는 됐지만 배포판이 없는 경우
/// 제외).
#[cfg(windows)]
fn wsl_detected(probe: &dyn ShellProbe) -> bool {
    let Some(path) = wsl_exe_path(probe) else {
        return false;
    };
    if !probe.exists(&path) {
        return false;
    }
    match probe.command_stdout("wsl", &["-l", "-q"]) {
        Some(out) => out.lines().any(|l| !l.trim().is_empty()),
        None => false,
    }
}

/// resolve 전용: `wsl -l -q`는 실행하지 않고(탐지 전용 명령) `wsl.exe`
/// 파일 존재만 확인한다.
#[cfg(windows)]
fn wsl_exe_exists(probe: &dyn ShellProbe) -> bool {
    wsl_exe_path(probe)
        .map(|p| probe.exists(&p))
        .unwrap_or(false)
}

/// 호스트에 설치된 셸 목록(드롭다운용). 프로덕션 경로.
pub fn detect_shells() -> Vec<AvailableShell> {
    detect_shells_with(&RealProbe)
}

#[cfg(windows)]
pub fn detect_shells_with(probe: &dyn ShellProbe) -> Vec<AvailableShell> {
    let mut out = Vec::new();

    out.push(AvailableShell {
        id: "powershell".to_string(),
        label: "Windows PowerShell".to_string(),
        path: powershell_path(probe),
        hooks_supported: true,
    });

    if let Some(path) = find_pwsh(probe) {
        out.push(AvailableShell {
            id: "pwsh".to_string(),
            label: "PowerShell 7 (pwsh)".to_string(),
            path,
            hooks_supported: true,
        });
    }

    if let Some(path) = find_git_bash(probe) {
        out.push(AvailableShell {
            id: "git-bash".to_string(),
            label: "Git Bash".to_string(),
            path,
            hooks_supported: true,
        });
    }

    if wsl_detected(probe) {
        out.push(AvailableShell {
            id: "wsl".to_string(),
            label: "WSL".to_string(),
            path: "wsl.exe".to_string(),
            hooks_supported: false,
        });
    }

    out
}

#[cfg(not(windows))]
pub fn detect_shells_with(_probe: &dyn ShellProbe) -> Vec<AvailableShell> {
    Vec::new()
}

/// Resolves a shell from provider-neutral command wrapper specs. Production
/// session creation uses this resolver directly.
pub fn resolve_observed(selected: Option<&str>, wrappers: &[CommandWrapperSpec]) -> ResolvedShell {
    resolve_observed_with_shims(selected, wrappers, &RealProbe, &RealObserverShimWriter)
}

fn resolve_observed_with(
    selected: Option<&str>,
    wrappers: &[CommandWrapperSpec],
    probe: &dyn ShellProbe,
) -> ResolvedShell {
    resolve_observed_with_shims(selected, wrappers, probe, &RealObserverShimWriter)
}

#[cfg(windows)]
pub(crate) fn resolve_observed_with_shims(
    selected: Option<&str>,
    wrappers: &[CommandWrapperSpec],
    probe: &dyn ShellProbe,
    shims: &dyn ObserverShimWriter,
) -> ResolvedShell {
    let powershell = |program: String| ResolvedShell {
        program,
        args: if wrappers.is_empty() {
            vec!["-NoExit".into()]
        } else {
            vec![
                "-NoExit".into(),
                "-EncodedCommand".into(),
                encoded_command(&crate::session::wrapper_script::render_powershell(wrappers)),
            ]
        },
        extra_env: vec![],
    };
    let auto = || powershell(find_pwsh(probe).unwrap_or_else(|| powershell_path(probe)));

    match selected {
        Some("powershell") => powershell(powershell_path(probe)),
        Some("pwsh") => match find_pwsh(probe) {
            Some(program) => powershell(program),
            None => auto(),
        },
        Some("git-bash") => match find_git_bash(probe) {
            Some(program) => {
                let args = if wrappers.is_empty() {
                    vec!["-i".into()]
                } else {
                    match shims.bashrc(wrappers) {
                        Ok(path) => vec![
                            "--rcfile".into(),
                            path.to_string_lossy().into_owned(),
                            "-i".into(),
                        ],
                        Err(error) => {
                            eprintln!("agent-office: failed to write observer bash shim: {error}");
                            vec!["-i".into()]
                        }
                    }
                };
                ResolvedShell {
                    program,
                    args,
                    extra_env: vec![],
                }
            }
            None => auto(),
        },
        Some("wsl") if wsl_exe_exists(probe) => ResolvedShell {
            program: "wsl.exe".into(),
            args: vec![],
            extra_env: vec![],
        },
        _ => auto(),
    }
}

#[cfg(not(windows))]
pub(crate) fn resolve_observed_with_shims(
    _selected: Option<&str>,
    wrappers: &[CommandWrapperSpec],
    _probe: &dyn ShellProbe,
    shims: &dyn ObserverShimWriter,
) -> ResolvedShell {
    let program = std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/bin/zsh".into()
        } else {
            "/bin/bash".into()
        }
    });
    let mut extra_env = Vec::new();
    if !wrappers.is_empty() && crate::session::zsh_wrapper::is_zsh(&program) {
        match shims.zdotdir(wrappers) {
            Ok(path) => extra_env.push(("ZDOTDIR".into(), path.to_string_lossy().into_owned())),
            Err(error) => {
                eprintln!("agent-office: failed to write observer zsh shim: {error}")
            }
        }
    }
    ResolvedShell {
        program,
        args: vec!["-l".into(), "-i".into()],
        extra_env,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observer::{CommandWrapperSpec, WrapperArg};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    #[derive(Default)]
    struct FakeProbe {
        files: HashSet<String>,
        program_files: Option<String>,
        program_files_x86: Option<String>,
        system_root: Option<String>,
        stdout: HashMap<(String, Vec<String>), Option<String>>,
    }

    impl FakeProbe {
        fn new() -> Self {
            Self::default()
        }
        fn with_file(mut self, path: &str) -> Self {
            self.files.insert(path.to_string());
            self
        }
        fn with_program_files(mut self, v: &str) -> Self {
            self.program_files = Some(v.to_string());
            self
        }
        fn with_program_files_x86(mut self, v: &str) -> Self {
            self.program_files_x86 = Some(v.to_string());
            self
        }
        fn with_system_root(mut self, v: &str) -> Self {
            self.system_root = Some(v.to_string());
            self
        }
        fn with_stdout(mut self, program: &str, args: &[&str], out: Option<&str>) -> Self {
            let key = (
                program.to_string(),
                args.iter().map(|s| s.to_string()).collect(),
            );
            self.stdout.insert(key, out.map(|s| s.to_string()));
            self
        }
    }

    impl ShellProbe for FakeProbe {
        fn exists(&self, path: &str) -> bool {
            self.files.contains(path)
        }
        fn program_files(&self) -> Option<String> {
            self.program_files.clone()
        }
        fn program_files_x86(&self) -> Option<String> {
            self.program_files_x86.clone()
        }
        fn system_root(&self) -> Option<String> {
            self.system_root.clone()
        }
        fn command_stdout(&self, program: &str, args: &[&str]) -> Option<String> {
            let key = (
                program.to_string(),
                args.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
            );
            self.stdout.get(&key).cloned().flatten()
        }
    }

    fn decode_ps_script(encoded: &str) -> String {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .expect("valid base64");
        let utf16: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&utf16).expect("valid UTF-16LE")
    }

    fn observer_wrappers() -> Vec<CommandWrapperSpec> {
        vec![
            CommandWrapperSpec {
                command: "claude".into(),
                prefix_args: vec![
                    WrapperArg::Literal("--settings".into()),
                    WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
                ],
                skip_if_present: vec!["--settings".into()],
            },
            CommandWrapperSpec {
                command: "codex".into(),
                prefix_args: vec![
                    WrapperArg::Literal("--enable".into()),
                    WrapperArg::Literal("hooks".into()),
                    WrapperArg::Literal("-c".into()),
                    WrapperArg::Env("AGENT_OFFICE_CODEX_HOOK_STOP".into()),
                ],
                skip_if_present: vec![],
            },
        ]
    }

    struct FailingShims;

    impl ObserverShimWriter for FailingShims {
        fn bashrc(&self, _wrappers: &[CommandWrapperSpec]) -> std::io::Result<PathBuf> {
            Err(std::io::Error::other("injected bash shim failure"))
        }

        fn zdotdir(&self, _wrappers: &[CommandWrapperSpec]) -> std::io::Result<PathBuf> {
            Err(std::io::Error::other("injected zsh shim failure"))
        }
    }

    struct UnexpectedShims;

    impl ObserverShimWriter for UnexpectedShims {
        fn bashrc(&self, _wrappers: &[CommandWrapperSpec]) -> std::io::Result<PathBuf> {
            panic!("bash shim must not be written")
        }

        fn zdotdir(&self, _wrappers: &[CommandWrapperSpec]) -> std::io::Result<PathBuf> {
            panic!("zsh shim must not be written")
        }
    }

    struct SuccessfulShims;

    impl ObserverShimWriter for SuccessfulShims {
        fn bashrc(&self, _wrappers: &[CommandWrapperSpec]) -> std::io::Result<PathBuf> {
            Ok(PathBuf::from("observer-bashrc"))
        }

        fn zdotdir(&self, _wrappers: &[CommandWrapperSpec]) -> std::io::Result<PathBuf> {
            Ok(PathBuf::from("observer-zdotdir"))
        }
    }

    #[cfg(windows)]
    #[test]
    fn observer_off_powershell_has_no_encoded_observer_function() {
        let probe = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_file(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        let resolved = resolve_observed_with(Some("powershell"), &[], &probe);
        assert_eq!(
            resolved.program,
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
        );
        assert_eq!(resolved.args, vec!["-NoExit"]);
    }

    #[cfg(windows)]
    #[test]
    fn observer_on_powershell_encoded_script_contains_both_functions() {
        let probe = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_file(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        let resolved = resolve_observed_with(Some("powershell"), &observer_wrappers(), &probe);
        assert_eq!(resolved.args[..2], ["-NoExit", "-EncodedCommand"]);
        let script = decode_ps_script(&resolved.args[2]);
        assert!(script.contains("global:claude"), "{script}");
        assert!(script.contains("global:codex"), "{script}");
    }

    #[cfg(windows)]
    #[test]
    fn observer_off_git_bash_is_plain_interactive_and_writes_no_shim() {
        let probe = FakeProbe::new()
            .with_program_files(r"C:\Program Files")
            .with_file(r"C:\Program Files\Git\bin\bash.exe");
        let resolved = resolve_observed_with_shims(Some("git-bash"), &[], &probe, &UnexpectedShims);
        assert_eq!(resolved.args, vec!["-i"]);
        assert!(resolved.extra_env.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn observer_git_bash_shim_failure_falls_back_to_usable_plain_interactive_shell() {
        let probe = FakeProbe::new()
            .with_program_files(r"C:\Program Files")
            .with_file(r"C:\Program Files\Git\bin\bash.exe");
        let resolved = resolve_observed_with_shims(
            Some("git-bash"),
            &observer_wrappers(),
            &probe,
            &FailingShims,
        );
        assert_eq!(resolved.program, r"C:\Program Files\Git\bin\bash.exe");
        assert_eq!(resolved.args, vec!["-i"]);
        assert!(resolved.extra_env.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn observer_wsl_remains_unwrapped_even_when_specs_are_present() {
        let probe = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_file(r"C:\Windows\System32\wsl.exe");
        let resolved = resolve_observed_with_shims(
            Some("wsl"),
            &observer_wrappers(),
            &probe,
            &UnexpectedShims,
        );
        assert_eq!(resolved.program, "wsl.exe");
        assert!(resolved.args.is_empty());
        assert!(resolved.extra_env.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn observer_pwsh_uses_adapter_wrapper_specs() {
        let probe = FakeProbe::new()
            .with_program_files(r"C:\Program Files")
            .with_file(r"C:\Program Files\PowerShell\7\pwsh.exe");
        let resolved = resolve_observed_with(Some("pwsh"), &observer_wrappers(), &probe);
        assert_eq!(resolved.program, r"C:\Program Files\PowerShell\7\pwsh.exe");
        assert_eq!(resolved.args[..2], ["-NoExit", "-EncodedCommand"]);
        let script = decode_ps_script(&resolved.args[2]);
        assert!(script.contains("global:claude"), "{script}");
        assert!(script.contains("global:codex"), "{script}");
    }

    #[cfg(windows)]
    #[test]
    fn observer_auto_prefers_pwsh_when_present() {
        let probe = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_program_files(r"C:\Program Files")
            .with_file(r"C:\Program Files\PowerShell\7\pwsh.exe");
        let resolved = resolve_observed_with(None, &[], &probe);
        assert_eq!(resolved.program, r"C:\Program Files\PowerShell\7\pwsh.exe");
        assert_eq!(resolved.args, vec!["-NoExit"]);
    }

    #[cfg(windows)]
    #[test]
    fn observer_auto_falls_back_to_powershell_when_pwsh_absent() {
        let probe = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_file(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        let resolved = resolve_observed_with(None, &[], &probe);
        assert_eq!(
            resolved.program,
            r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe"
        );
        assert_eq!(resolved.args, vec!["-NoExit"]);
    }

    #[cfg(windows)]
    #[test]
    fn observer_auto_falls_back_to_literal_powershell_exe_when_system_root_missing() {
        let probe = FakeProbe::new();
        let resolved = resolve_observed_with(None, &[], &probe);
        assert_eq!(resolved.program, "powershell.exe");
    }

    #[cfg(windows)]
    #[test]
    fn observer_git_bash_with_wrappers_uses_rcfile_shim() {
        let probe = FakeProbe::new()
            .with_program_files(r"C:\Program Files")
            .with_file(r"C:\Program Files\Git\bin\bash.exe");
        let resolved = resolve_observed_with_shims(
            Some("git-bash"),
            &observer_wrappers(),
            &probe,
            &SuccessfulShims,
        );
        assert_eq!(resolved.program, r"C:\Program Files\Git\bin\bash.exe");
        assert_eq!(resolved.args, vec!["--rcfile", "observer-bashrc", "-i"]);
    }

    #[cfg(windows)]
    #[test]
    fn observer_git_bash_falls_back_to_program_files_x86_when_64bit_path_absent() {
        // 32비트 Git 설치는 %ProgramFiles(x86)%\Git\bin\bash.exe에만 있다 --
        // %ProgramFiles%\Git\bin\bash.exe가 없어도 이 경로로 탐지돼야 한다.
        let probe = FakeProbe::new()
            .with_program_files(r"C:\Program Files")
            .with_program_files_x86(r"C:\Program Files (x86)")
            .with_file(r"C:\Program Files (x86)\Git\bin\bash.exe");
        let resolved = resolve_observed_with(Some("git-bash"), &[], &probe);
        assert_eq!(resolved.program, r"C:\Program Files (x86)\Git\bin\bash.exe");
    }

    #[cfg(windows)]
    #[test]
    fn observer_git_bash_not_detected_falls_back_to_auto() {
        let probe = FakeProbe::new().with_system_root(r"C:\Windows");
        let resolved = resolve_observed_with(Some("git-bash"), &observer_wrappers(), &probe);
        assert_eq!(resolved.program, "powershell.exe");
        assert_eq!(resolved.args[..2], ["-NoExit", "-EncodedCommand"]);
    }

    #[cfg(windows)]
    #[test]
    fn observer_unknown_selected_id_falls_back_to_auto() {
        let probe = FakeProbe::new().with_system_root(r"C:\Windows");
        let resolved = resolve_observed_with(Some("bogus"), &[], &probe);
        assert_eq!(resolved.program, "powershell.exe");
        assert_eq!(resolved.args, vec!["-NoExit"]);
    }

    // ---- detect_shells_with ----

    #[cfg(windows)]
    #[test]
    fn detect_shells_always_includes_powershell() {
        let probe = FakeProbe::new();
        let shells = detect_shells_with(&probe);
        assert!(shells.iter().any(|s| s.id == "powershell"));
    }

    #[cfg(windows)]
    #[test]
    fn detect_shells_includes_pwsh_and_git_bash_only_when_files_present() {
        let probe_absent = FakeProbe::new();
        let shells_absent = detect_shells_with(&probe_absent);
        assert!(!shells_absent.iter().any(|s| s.id == "pwsh"));
        assert!(!shells_absent.iter().any(|s| s.id == "git-bash"));

        let probe_present = FakeProbe::new()
            .with_program_files(r"C:\Program Files")
            .with_file(r"C:\Program Files\PowerShell\7\pwsh.exe")
            .with_file(r"C:\Program Files\Git\bin\bash.exe");
        let shells_present = detect_shells_with(&probe_present);
        assert!(shells_present.iter().any(|s| s.id == "pwsh"));
        assert!(shells_present.iter().any(|s| s.id == "git-bash"));
    }

    #[cfg(windows)]
    #[test]
    fn detect_shells_includes_wsl_only_when_exe_present_and_has_distros() {
        let probe_no_exe = FakeProbe::new().with_system_root(r"C:\Windows");
        assert!(!detect_shells_with(&probe_no_exe)
            .iter()
            .any(|s| s.id == "wsl"));

        let probe_no_distro = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_file(r"C:\Windows\System32\wsl.exe")
            .with_stdout("wsl", &["-l", "-q"], Some(""));
        assert!(!detect_shells_with(&probe_no_distro)
            .iter()
            .any(|s| s.id == "wsl"));

        let probe_ok = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_file(r"C:\Windows\System32\wsl.exe")
            .with_stdout("wsl", &["-l", "-q"], Some("Ubuntu\r\n"));
        let shells = detect_shells_with(&probe_ok);
        let wsl = shells
            .iter()
            .find(|s| s.id == "wsl")
            .expect("wsl must be detected");
        assert_eq!(wsl.path, "wsl.exe");
        assert!(!wsl.hooks_supported);
    }

    // ---- decode_probe_stdout ----

    #[test]
    fn decode_probe_stdout_handles_utf16le_with_bom() {
        // "Ubuntu\r\n" encoded as UTF-16LE with a leading BOM (FF FE), as
        // `wsl -l -q` emits on real Windows hosts.
        let text = "Ubuntu\r\n";
        let mut bytes = vec![0xFFu8, 0xFE];
        for u in text.encode_utf16() {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let decoded = decode_probe_stdout(&bytes);
        assert_eq!(decoded, text);
    }

    #[test]
    fn decode_probe_stdout_passes_through_plain_utf8() {
        let decoded = decode_probe_stdout(b"C:\\Program Files\\Git\\cmd\\git.exe\r\n");
        assert_eq!(decoded, "C:\\Program Files\\Git\\cmd\\git.exe\r\n");
    }

    // ---- non-windows: detect_shells_with returns empty ----

    #[cfg(not(windows))]
    #[test]
    fn detect_shells_with_returns_empty_on_non_windows() {
        let probe = FakeProbe::new();
        assert!(detect_shells_with(&probe).is_empty());
    }
}
