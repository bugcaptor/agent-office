//! Windows 빈 셸 판정. ConPTY에는 foreground 프로세스 그룹이 없으므로
//! 앱이 띄운 셸 프로세스에 살아 있는 자손이 없으면 프롬프트 대기로 본다.
//! 조회 실패는 None, 판정이 애매하면 false(재시작 확인)로 둔다.

use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent: u32,
    pub exe: String,
}

/// `shell_pid`가 앱이 띄운 `shell_path` 셸일 때, 자식 없이 입력을 기다리는지.
pub fn shell_is_idle(shell_pid: u32, shell_path: &str) -> Option<bool> {
    idle_from_entries(shell_pid, shell_path, &snapshot()?)
}

pub(crate) fn idle_from_entries(
    shell_pid: u32,
    shell_path: &str,
    entries: &[ProcessEntry],
) -> Option<bool> {
    if shell_pid == 0 || !entries.iter().any(|e| e.pid == shell_pid) {
        return None;
    }
    let mut seen = HashSet::from([shell_pid]);
    let mut queue = vec![shell_pid];
    let mut descendants = Vec::new();
    while let Some(parent) = queue.pop() {
        for entry in entries.iter().filter(|e| e.parent == parent) {
            if seen.insert(entry.pid) {
                queue.push(entry.pid);
                descendants.push(entry);
            }
        }
    }
    let shell_exe = Path::new(shell_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    // Git for Windows의 bin\bash.exe는 usr\bin\bash.exe를 자식으로 띄우는 런처다.
    let launcher = shell_exe.eq_ignore_ascii_case("bash.exe");
    Some(match descendants.as_slice() {
        [] => true,
        [only] => launcher && only.parent == shell_pid && only.exe.eq_ignore_ascii_case("bash.exe"),
        _ => false,
    })
}

fn snapshot() -> Option<Vec<ProcessEntry>> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    // SAFETY: 스냅숏 핸들은 이 함수 안에서만 쓰고 닫는다. 엔트리는 dwSize를 채운
    // 지역 버퍼이며 API가 NUL로 끝나는 szExeFile을 쓴다.
    unsafe {
        let handle = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut entries = Vec::new();
        let mut ok = Process32FirstW(handle, &mut entry) != 0;
        while ok {
            let len = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            entries.push(ProcessEntry {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                exe: String::from_utf16_lossy(&entry.szExeFile[..len]),
            });
            ok = Process32NextW(handle, &mut entry) != 0;
        }
        CloseHandle(handle);
        Some(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent: u32, exe: &str) -> ProcessEntry {
        ProcessEntry { pid, parent, exe: exe.into() }
    }

    const PS: &str = r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe";
    const GIT_BASH: &str = r"C:\Program Files\Git\bin\bash.exe";

    #[test]
    fn powershell_without_children_is_idle() {
        let entries = [p(4, 0, "System"), p(10, 1, "agent-office.exe"), p(20, 10, "powershell.exe")];
        assert_eq!(idle_from_entries(20, PS, &entries), Some(true));
    }

    #[test]
    fn any_descendant_means_busy() {
        let entries = [p(20, 10, "powershell.exe"), p(30, 20, "node.exe"), p(40, 30, "conhost.exe")];
        assert_eq!(idle_from_entries(20, PS, &entries), Some(false));
        let entries = [p(20, 10, "pwsh.exe"), p(30, 20, "bash.exe")];
        assert_eq!(idle_from_entries(20, PS, &entries), Some(false));
    }

    #[test]
    fn git_bash_launcher_allows_only_its_inner_bash() {
        let idle = [p(20, 10, "bash.exe"), p(21, 20, "bash.exe")];
        assert_eq!(idle_from_entries(20, GIT_BASH, &idle), Some(true));
        let busy = [p(20, 10, "bash.exe"), p(21, 20, "bash.exe"), p(22, 21, "claude.exe")];
        assert_eq!(idle_from_entries(20, GIT_BASH, &busy), Some(false));
        let other = [p(20, 10, "bash.exe"), p(21, 20, "node.exe")];
        assert_eq!(idle_from_entries(20, GIT_BASH, &other), Some(false));
    }

    #[test]
    fn missing_shell_is_unknown_and_cycles_terminate() {
        assert_eq!(idle_from_entries(20, PS, &[p(30, 20, "node.exe")]), None);
        assert_eq!(idle_from_entries(0, PS, &[p(0, 0, "Idle")]), None);
        let cycle = [p(20, 30, "powershell.exe"), p(30, 20, "node.exe")];
        assert_eq!(idle_from_entries(20, PS, &cycle), Some(false));
    }

    #[test]
    fn real_snapshot_contains_the_current_process() {
        let me = std::process::id();
        assert!(snapshot().unwrap().iter().any(|e| e.pid == me));
    }
}
