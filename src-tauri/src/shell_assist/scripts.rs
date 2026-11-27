// src-tauri/src/shell_assist/scripts.rs
//
// 셸 캐릭터 세션에 명령 경계 표식(`marks.rs`)을 찍게 하는 셸 훅.
//
// 관찰 shim(ZDOTDIR·--rcfile)은 관찰을 켰을 때만 생기고 모든 캐릭터가 같은 파일을 쓴다.
// 그래서 셸 캐릭터만 세션이 뜬 직후 stdin으로 한 줄을 넣어 훅 파일을 읽힌다(시작 명령과
// 같은 경로). 여러 번 읽어도 한 번만 걸리게 가드한다.

use std::path::{Path, PathBuf};

/// 표식을 지원하는 셸 종류.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellKind {
    Zsh,
    Bash,
    PowerShell,
}

impl ShellKind {
    /// 실행 파일 경로 → 셸 종류. 모르면 None(fish·WSL 등은 표식 없이 둔다).
    pub fn from_program(program: &str) -> Option<Self> {
        let base = program.rsplit(['/', '\\']).next().unwrap_or(program).to_ascii_lowercase();
        let base = base.strip_suffix(".exe").unwrap_or(&base);
        match base {
            "zsh" => Some(Self::Zsh),
            "bash" | "sh" => Some(Self::Bash),
            "pwsh" | "powershell" => Some(Self::PowerShell),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Zsh => "zsh",
            Self::Bash => "bash",
            Self::PowerShell => "pwsh",
        }
    }

    fn file_name(self) -> &'static str {
        match self {
            Self::Zsh => "shell-marks.zsh",
            Self::Bash => "shell-marks.bash",
            Self::PowerShell => "shell-marks.ps1",
        }
    }

    fn script(self) -> &'static str {
        match self {
            Self::Zsh => ZSH,
            Self::Bash => BASH,
            Self::PowerShell => POWERSHELL,
        }
    }
}

// precmd는 다른 precmd보다 먼저 돌아야 `$?`가 명령의 종료 코드다(앞에 끼운다).
const ZSH: &str = r#"# agent-office 셸 캐릭터: 명령 경계 표식
if [[ -z "$__AO_MARKS" ]]; then
  typeset -g __AO_MARKS=1 __AO_RUNNING=
  __ao_preexec() {
    local c="${1//[$'\a\e\r\n']/ }"
    printf '\e]6973;C;%s\a' "$c"
    __AO_RUNNING=1
  }
  __ao_precmd() {
    local s=$?
    if [[ -n "$__AO_RUNNING" ]]; then
      printf '\e]6973;D;%s\a' "$s"
      __AO_RUNNING=
    fi
    printf '\e]6973;P;%s\a' "${PWD//[$'\a\e']/}"
  }
  typeset -ga preexec_functions precmd_functions
  preexec_functions+=(__ao_preexec)
  precmd_functions=(__ao_precmd $precmd_functions)
fi
"#;

// PS0(bash 4.4+)은 명령을 읽은 직후 실행 전에 펼쳐진다. 그 안의 명령 치환은 서브셸이라
// 변수를 못 바꾸므로, 끝 표식은 늘 찍고 기록기가 실행 중이 아니면 무시한다.
const BASH: &str = r#"# agent-office 셸 캐릭터: 명령 경계 표식
if [ -z "$__AO_MARKS" ]; then
  __AO_MARKS=1
  __ao_cmdline() {
    local c
    c=$(HISTTIMEFORMAT= builtin history 1)
    c="${c#"${c%%[![:space:]]*}"}"
    c="${c#*[[:space:]]}"
    c="${c#"${c%%[![:space:]]*}"}"
    printf '\033]6973;C;%s\007' "${c//[$'\a\e\r\n']/ }"
  }
  __ao_precmd() {
    local s=$?
    printf '\033]6973;D;%s\007\033]6973;P;%s\007' "$s" "$PWD"
  }
  PS0='$(__ao_cmdline)'"${PS0}"
  PROMPT_COMMAND="__ao_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
fi
"#;

// PSReadLine의 Enter 처리에서 명령 줄을 읽어 시작 표식을 찍고, prompt 함수를 감싸 끝·프롬프트
// 표식을 찍는다. ConPTY가 모르는 OSC를 걸러 내는 Windows 빌드에서는 표식이 오지 않을 수 있다.
const POWERSHELL: &str = r#"# agent-office 셸 캐릭터: 명령 경계 표식
if (-not $global:__AOMarks) {
  $global:__AOMarks = $true
  $global:__AORunning = $false
  $global:__AOPrompt = $function:prompt
  function global:prompt {
    $ok = $?
    $code = $global:LASTEXITCODE
    $e = [char]27; $b = [char]7
    $out = ''
    if ($global:__AORunning) {
      $n = if ($ok) { 0 } elseif ($code) { $code } else { 1 }
      $out = "$e]6973;D;$n$b"
      $global:__AORunning = $false
    }
    $out += "$e]6973;P;$($executionContext.SessionState.Path.CurrentLocation.ProviderPath)$b"
    $out + (& $global:__AOPrompt)
  }
  if (Get-Command Set-PSReadLineKeyHandler -ErrorAction SilentlyContinue) {
    Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {
      $line = $null; $cursor = $null
      [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
      [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
      if ($line -and $line.Trim()) {
        $global:__AORunning = $true
        $c = $line -replace '[\x07\x1b\r\n]', ' '
        [Console]::Write("$([char]27)]6973;C;$c$([char]7)")
      }
    }
  }
}
"#;

/// 훅 파일을 `base`에 쓰고 경로를 돌려준다.
pub fn write_script(base: &Path, kind: ShellKind) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(base)?;
    let path = base.join(kind.file_name());
    let script = kind.script();
    // 같은 내용이면 다시 쓰지 않는다(여러 세션이 동시에 뜰 때 읽는 중인 파일을 갈아엎지 않게).
    if std::fs::read_to_string(&path).ok().as_deref() != Some(script) {
        crate::persistence::durable::write_atomic(&path, script.as_bytes())?;
    }
    Ok(path)
}

/// 세션 stdin에 넣을 한 줄. 앞 공백은 HIST_IGNORE_SPACE·ignorespace인 셸에서 기록을 남기지 않게,
/// 화면 지우기는 이 줄의 메아리를 감추려고 둔다. 줄 끝은 CR(PowerShell은 CR에서만 제출).
pub fn inject_line(kind: ShellKind, path: &Path) -> String {
    let p = path.to_string_lossy();
    match kind {
        ShellKind::Zsh => format!(" source {}; clear\r", sh_quote(&p)),
        // Git Bash는 역슬래시 경로를 받지만 따옴표 안에서 슬래시가 더 안전하다.
        ShellKind::Bash => format!(" source {}; clear\r", sh_quote(&p.replace('\\', "/"))),
        // 실행 정책이 스크립트 파일을 막아도 돌도록 파일 내용을 스크립트 블록으로 만들어 점 소싱한다.
        ShellKind::PowerShell => format!(
            " . ([ScriptBlock]::Create([IO.File]::ReadAllText({}))); Clear-Host\r",
            ps_quote(&p)
        ),
    }
}

fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_kind_from_program_paths() {
        assert_eq!(ShellKind::from_program("/bin/zsh"), Some(ShellKind::Zsh));
        assert_eq!(ShellKind::from_program(r"C:\Program Files\Git\bin\bash.exe"), Some(ShellKind::Bash));
        assert_eq!(ShellKind::from_program(r"C:\Program Files\PowerShell\7\pwsh.exe"), Some(ShellKind::PowerShell));
        assert_eq!(ShellKind::from_program("powershell.exe"), Some(ShellKind::PowerShell));
        assert_eq!(ShellKind::from_program("wsl.exe"), None);
        assert_eq!(ShellKind::from_program("/usr/bin/fish"), None);
    }

    #[test]
    fn inject_line_quotes_paths() {
        let line = inject_line(ShellKind::Zsh, Path::new("/tmp/it's/shell-marks.zsh"));
        assert_eq!(line, " source '/tmp/it'\\''s/shell-marks.zsh'; clear\r");
        let line = inject_line(ShellKind::PowerShell, Path::new(r"C:\Users\o'k\a.ps1"));
        assert!(line.contains(r"'C:\Users\o''k\a.ps1'"), "{line}");
        assert!(line.ends_with('\r'));
        let line = inject_line(ShellKind::Bash, Path::new(r"C:\T\a.bash"));
        assert!(line.contains("'C:/T/a.bash'"), "{line}");
    }

    #[test]
    fn write_script_is_idempotent() {
        let base = std::env::temp_dir().join(format!("agent-office-marks-{}", uuid::Uuid::new_v4()));
        let p1 = write_script(&base, ShellKind::Zsh).unwrap();
        let first = std::fs::metadata(&p1).unwrap().modified().unwrap();
        let p2 = write_script(&base, ShellKind::Zsh).unwrap();
        assert_eq!(p1, p2);
        assert_eq!(std::fs::metadata(&p2).unwrap().modified().unwrap(), first);
        assert_eq!(std::fs::read_to_string(&p1).unwrap(), ZSH);
        let _ = std::fs::remove_dir_all(base);
    }

    /// 실제 zsh로 훅을 돌려 표식이 순서대로 나오는지 본다(zsh가 없으면 건너뛴다).
    #[test]
    #[cfg(unix)]
    fn zsh_hooks_emit_marks_in_order() {
        if std::process::Command::new("zsh").arg("-c").arg("true").output().is_err() {
            return;
        }
        let base = std::env::temp_dir().join(format!("agent-office-marks-{}", uuid::Uuid::new_v4()));
        let path = write_script(&base, ShellKind::Zsh).unwrap();
        // 비대화형 zsh에는 프롬프트 훅이 없으니 훅 함수를 직접 부른다.
        let script = format!(
            "source {}; __ao_precmd; __ao_preexec 'echo hi'; echo hi; false; __ao_precmd",
            sh_quote(&path.to_string_lossy())
        );
        let out = std::process::Command::new("zsh").arg("-f").arg("-c").arg(&script).current_dir(&base).output().unwrap();
        let mut scanner = super::super::marks::MarkScanner::default();
        let pieces = scanner.feed(&out.stdout);
        use super::super::marks::{Mark, Piece};
        let marks: Vec<Mark> = pieces.into_iter().filter_map(|p| match p { Piece::Mark(m) => Some(m), _ => None }).collect();
        let cwd = std::fs::canonicalize(&base).unwrap().to_string_lossy().into_owned();
        assert_eq!(
            marks,
            vec![
                Mark::Prompt { cwd: cwd.clone() },
                Mark::Start { command: "echo hi".into() },
                Mark::Done { exit: Some(1) },
                Mark::Prompt { cwd },
            ]
        );
        let _ = std::fs::remove_dir_all(base);
    }
}
