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
// `AGENT_WRAPPER_PS`/`encoded_command`는 `session::manager`에 있던 것을
// 그대로(verbatim) 옮겨왔다 -- Windows PowerShell 계열(powershell.exe,
// pwsh.exe) 모두 동일한 래퍼 스크립트를 `-EncodedCommand`로 주입해
// `claude` 호출 시 `--settings $env:AGENT_OFFICE_SETTINGS`를 투명하게
// 붙인다(시간 집계 훅 발화 보장). Git Bash는 PowerShell 함수 대신
// `session::bash_wrapper`의 `--rcfile` 심으로 같은 일을 한다.

/// 탐지된 셸 1개. 렌더러 드롭다운에 그대로 보낸다(list_available_shells).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableShell {
    pub id: String,           // "powershell" | "pwsh" | "git-bash" | "wsl"
    pub label: String,        // 드롭다운 표시용
    pub path: String,         // 실제 실행 프로그램(절대 경로 또는 "wsl.exe")
    pub hooks_supported: bool, // wsl은 MVP에서 false
}

/// `resolve`/`resolve_with`에 넘기는 요청. `selected`는 프로필의 셸 id
/// (None이면 자동 선택), `hooks_on`은 이번 세션에 AGENT_OFFICE_SETTINGS가
/// 실제로 주입되는지 여부(훅 opt-in이 켜져 있고 --settings 파일이 쓰인
/// 경우) -- git-bash 분기의 --rcfile 심 설치 여부를 결정한다.
pub struct ShellRequest<'a> {
    pub selected: Option<&'a str>,
    pub hooks_on: bool,
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
    String::from_utf16_lossy(&units).chars().filter(|&c| c != '\0').collect()
}

/// Static PowerShell snippet defining a `claude` wrapper. Reads
/// $env:AGENT_OFFICE_SETTINGS lazily at call time, so one encoded command
/// works for every session. `-CommandType Application,ExternalScript`
/// resolves the real claude (.cmd/.exe/.ps1) and never matches our Function,
/// so no recursion. Skips injection when the user passes --settings.
/// (session::manager에서 verbatim 이전 -- powershell.exe/pwsh.exe 공용.)
#[cfg(windows)]
const AGENT_WRAPPER_PS: &str = r#"
function claude {
    $cmd = Get-Command claude -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $cmd) { Write-Error 'claude executable not found on PATH'; return }
    if ($env:AGENT_OFFICE_SETTINGS -and ($args -notcontains '--settings')) {
        & $cmd.Source --settings $env:AGENT_OFFICE_SETTINGS @args
    } else {
        & $cmd.Source @args
    }
}
function pi {
    $cmd = Get-Command pi -CommandType Application,ExternalScript -ErrorAction SilentlyContinue | Select-Object -First 1
    if (-not $cmd) { Write-Error 'pi executable not found on PATH'; return }
    if ($env:AGENT_OFFICE_PI_EXT) {
        & $cmd.Source -e $env:AGENT_OFFICE_PI_EXT @args
    } else {
        & $cmd.Source @args
    }
}
"#;

/// PowerShell `-EncodedCommand` payload: Base64 of UTF-16LE bytes.
/// (session::manager에서 verbatim 이전.)
#[cfg(windows)]
fn encoded_command(script: &str) -> String {
    use base64::Engine;
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    base64::engine::general_purpose::STANDARD.encode(utf16)
}

#[cfg(windows)]
fn ps_wrapper_args() -> Vec<String> {
    vec!["-NoExit".to_string(), "-EncodedCommand".to_string(), encoded_command(AGENT_WRAPPER_PS)]
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
    probe.system_root().map(|root| format!(r"{root}\System32\wsl.exe"))
}

/// 탐지(detect_shells_with) 전용: `wsl.exe` 존재 + `wsl -l -q`가 배포판
/// 1개 이상을 보고하는지까지 확인한다(설치는 됐지만 배포판이 없는 경우
/// 제외).
#[cfg(windows)]
fn wsl_detected(probe: &dyn ShellProbe) -> bool {
    let Some(path) = wsl_exe_path(probe) else { return false };
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
    wsl_exe_path(probe).map(|p| probe.exists(&p)).unwrap_or(false)
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

/// 선택된(또는 자동) 셸을 실제 spawn 가능한 프로그램/인자로 해석한다.
/// 프로덕션 경로(`RealProbe` 사용).
pub fn resolve(req: ShellRequest) -> ResolvedShell {
    resolve_with(req, &RealProbe)
}

#[cfg(windows)]
pub fn resolve_with(req: ShellRequest, probe: &dyn ShellProbe) -> ResolvedShell {
    match req.selected {
        Some("powershell") => ResolvedShell {
            program: powershell_path(probe),
            args: ps_wrapper_args(),
            extra_env: vec![],
        },
        Some("pwsh") => match find_pwsh(probe) {
            Some(program) => ResolvedShell { program, args: ps_wrapper_args(), extra_env: vec![] },
            // pwsh를 선택했지만 이 호스트엔 없다 -- 자동 선택으로 폴백.
            None => resolve_auto(probe),
        },
        Some("git-bash") => match find_git_bash(probe) {
            Some(program) => {
                let args = if req.hooks_on {
                    match crate::session::bash_wrapper::ensure_bashrc() {
                        Ok(shim) => {
                            vec!["--rcfile".to_string(), shim.to_string_lossy().into_owned(), "-i".to_string()]
                        }
                        Err(e) => {
                            eprintln!("agent-office: failed to write bash rcfile shim: {e}");
                            vec!["-i".to_string()]
                        }
                    }
                } else {
                    vec!["-i".to_string()]
                };
                ResolvedShell { program, args, extra_env: vec![] }
            }
            None => resolve_auto(probe),
        },
        Some("wsl") => {
            if wsl_exe_exists(probe) {
                ResolvedShell { program: "wsl.exe".to_string(), args: vec![], extra_env: vec![] }
            } else {
                resolve_auto(probe)
            }
        }
        // None(자동) 또는 알 수 없는/미탐지 id -- 전부 자동 선택으로 수렴.
        _ => resolve_auto(probe),
    }
}

#[cfg(windows)]
fn resolve_auto(probe: &dyn ShellProbe) -> ResolvedShell {
    // pwsh가 있으면 우선, 없으면 항상 존재하는 powershell.exe로 폴백.
    let program = find_pwsh(probe).unwrap_or_else(|| powershell_path(probe));
    ResolvedShell { program, args: ps_wrapper_args(), extra_env: vec![] }
}

#[cfg(not(windows))]
pub fn resolve_with(_req: ShellRequest, _probe: &dyn ShellProbe) -> ResolvedShell {
    // macOS/Linux: 오늘의 default_shell 동작을 그대로 보존한다(선택 UI는
    // Windows 전용 기능이므로 selected는 무시). zsh ZDOTDIR 심은 여기서
    // 다루지 않고 manager.rs의 create()가 계속 담당한다.
    let shell = std::env::var("SHELL")
        .unwrap_or_else(|_| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/bash".into() });
    ResolvedShell { program: shell, args: vec!["-l".into(), "-i".into()], extra_env: vec![] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

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
            let key = (program.to_string(), args.iter().map(|s| s.to_string()).collect());
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
            let key = (program.to_string(), args.iter().map(|s| s.to_string()).collect::<Vec<_>>());
            self.stdout.get(&key).cloned().flatten()
        }
    }

    fn decode_ps_script(encoded: &str) -> String {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).expect("valid base64");
        let utf16: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16(&utf16).expect("valid UTF-16LE")
    }

    fn assert_ps_wrapper_args(args: &[String]) {
        assert_eq!(args.len(), 3);
        assert_eq!(args[0], "-NoExit");
        assert_eq!(args[1], "-EncodedCommand");
        let script = decode_ps_script(&args[2]);
        assert!(script.contains("function claude"), "{script}");
        assert!(script.contains("--settings $env:AGENT_OFFICE_SETTINGS"), "{script}");
        assert!(script.contains("-CommandType Application,ExternalScript"), "{script}");
        assert!(script.contains("function pi"), "{script}");
        assert!(script.contains("-e $env:AGENT_OFFICE_PI_EXT"), "{script}");
    }

    // ---- resolve_with: powershell/pwsh ----

    #[cfg(windows)]
    #[test]
    fn resolve_powershell_uses_encoded_command_wrapper() {
        let probe = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_file(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        let resolved = resolve_with(ShellRequest { selected: Some("powershell"), hooks_on: true }, &probe);
        assert_eq!(resolved.program, r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        assert_ps_wrapper_args(&resolved.args);
        assert!(resolved.extra_env.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn resolve_pwsh_uses_encoded_command_wrapper() {
        let probe = FakeProbe::new()
            .with_program_files(r"C:\Program Files")
            .with_file(r"C:\Program Files\PowerShell\7\pwsh.exe");
        let resolved = resolve_with(ShellRequest { selected: Some("pwsh"), hooks_on: true }, &probe);
        assert_eq!(resolved.program, r"C:\Program Files\PowerShell\7\pwsh.exe");
        assert_ps_wrapper_args(&resolved.args);
    }

    // ---- resolve_with: auto ----

    #[cfg(windows)]
    #[test]
    fn resolve_auto_prefers_pwsh_when_present() {
        let probe = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_program_files(r"C:\Program Files")
            .with_file(r"C:\Program Files\PowerShell\7\pwsh.exe");
        let resolved = resolve_with(ShellRequest { selected: None, hooks_on: false }, &probe);
        assert_eq!(resolved.program, r"C:\Program Files\PowerShell\7\pwsh.exe");
        assert_ps_wrapper_args(&resolved.args);
    }

    #[cfg(windows)]
    #[test]
    fn resolve_auto_falls_back_to_powershell_when_pwsh_absent() {
        let probe = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_file(r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        let resolved = resolve_with(ShellRequest { selected: None, hooks_on: false }, &probe);
        assert_eq!(resolved.program, r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe");
        assert_ps_wrapper_args(&resolved.args);
    }

    #[cfg(windows)]
    #[test]
    fn resolve_auto_falls_back_to_literal_powershell_exe_when_system_root_missing() {
        let probe = FakeProbe::new();
        let resolved = resolve_with(ShellRequest { selected: None, hooks_on: false }, &probe);
        assert_eq!(resolved.program, "powershell.exe");
    }

    // ---- resolve_with: git-bash ----

    #[cfg(windows)]
    #[test]
    fn resolve_git_bash_hooks_on_uses_rcfile_shim() {
        let probe = FakeProbe::new()
            .with_program_files(r"C:\Program Files")
            .with_file(r"C:\Program Files\Git\bin\bash.exe");
        let resolved = resolve_with(ShellRequest { selected: Some("git-bash"), hooks_on: true }, &probe);
        assert_eq!(resolved.program, r"C:\Program Files\Git\bin\bash.exe");
        assert!(resolved.args.contains(&"--rcfile".to_string()), "{:?}", resolved.args);
        assert!(resolved.args.contains(&"-i".to_string()), "{:?}", resolved.args);
    }

    #[cfg(windows)]
    #[test]
    fn resolve_git_bash_hooks_off_is_plain_interactive() {
        let probe = FakeProbe::new()
            .with_program_files(r"C:\Program Files")
            .with_file(r"C:\Program Files\Git\bin\bash.exe");
        let resolved = resolve_with(ShellRequest { selected: Some("git-bash"), hooks_on: false }, &probe);
        assert_eq!(resolved.args, vec!["-i".to_string()]);
    }

    #[cfg(windows)]
    #[test]
    fn resolve_git_bash_falls_back_to_program_files_x86_when_64bit_path_absent() {
        // 32비트 Git 설치는 %ProgramFiles(x86)%\Git\bin\bash.exe에만 있다 --
        // %ProgramFiles%\Git\bin\bash.exe가 없어도 이 경로로 탐지돼야 한다.
        let probe = FakeProbe::new()
            .with_program_files(r"C:\Program Files")
            .with_program_files_x86(r"C:\Program Files (x86)")
            .with_file(r"C:\Program Files (x86)\Git\bin\bash.exe");
        let resolved = resolve_with(ShellRequest { selected: Some("git-bash"), hooks_on: false }, &probe);
        assert_eq!(resolved.program, r"C:\Program Files (x86)\Git\bin\bash.exe");
    }

    #[cfg(windows)]
    #[test]
    fn resolve_git_bash_not_detected_falls_back_to_auto() {
        let probe = FakeProbe::new().with_system_root(r"C:\Windows");
        let resolved = resolve_with(ShellRequest { selected: Some("git-bash"), hooks_on: true }, &probe);
        // 폴백된 자동 선택은 항상 powershell.exe 계열(-NoExit/-EncodedCommand).
        assert_ps_wrapper_args(&resolved.args);
    }

    // ---- resolve_with: wsl ----

    #[cfg(windows)]
    #[test]
    fn resolve_wsl_uses_bare_wsl_exe_with_no_args() {
        let probe = FakeProbe::new().with_system_root(r"C:\Windows").with_file(r"C:\Windows\System32\wsl.exe");
        let resolved = resolve_with(ShellRequest { selected: Some("wsl"), hooks_on: false }, &probe);
        assert_eq!(resolved.program, "wsl.exe");
        assert!(resolved.args.is_empty());
        assert!(resolved.extra_env.is_empty());
    }

    // ---- resolve_with: unknown id ----

    #[cfg(windows)]
    #[test]
    fn resolve_unknown_selected_id_falls_back_to_auto() {
        let probe = FakeProbe::new().with_system_root(r"C:\Windows");
        let resolved = resolve_with(ShellRequest { selected: Some("bogus"), hooks_on: false }, &probe);
        assert_ps_wrapper_args(&resolved.args);
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
        assert!(!detect_shells_with(&probe_no_exe).iter().any(|s| s.id == "wsl"));

        let probe_no_distro = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_file(r"C:\Windows\System32\wsl.exe")
            .with_stdout("wsl", &["-l", "-q"], Some(""));
        assert!(!detect_shells_with(&probe_no_distro).iter().any(|s| s.id == "wsl"));

        let probe_ok = FakeProbe::new()
            .with_system_root(r"C:\Windows")
            .with_file(r"C:\Windows\System32\wsl.exe")
            .with_stdout("wsl", &["-l", "-q"], Some("Ubuntu\r\n"));
        let shells = detect_shells_with(&probe_ok);
        let wsl = shells.iter().find(|s| s.id == "wsl").expect("wsl must be detected");
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
