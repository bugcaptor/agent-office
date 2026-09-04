//! 레시피 실행은 캐릭터 PTY에 쓰지 않는다. agentId당 하나의 별도 프로세스만
//! 소유하고, 종료 대기 스레드가 정상 종료도 반드시 회수한다.

use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::types::{RunRecipeProcess, RunRecipeStartInput};

#[derive(Clone)]
struct Entry {
    pid: u32,
    token: u64,
    process: RunRecipeProcess,
}

pub struct RunRecipeRuntime {
    entries: Mutex<HashMap<String, Entry>>,
    operations: Mutex<()>,
    next_token: AtomicU64,
    shutting_down: AtomicBool,
}

impl Default for RunRecipeRuntime {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            operations: Mutex::new(()),
            next_token: AtomicU64::new(1),
            shutting_down: AtomicBool::new(false),
        }
    }
}

fn non_empty(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}

fn relative_cwd(value: &str) -> bool {
    !value.trim().is_empty()
        && Path::new(value).is_relative()
        && !Path::new(value).components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

fn validate(input: &RunRecipeStartInput) -> Result<PathBuf, String> {
    if !non_empty(&input.agent_id, 256)
        || !non_empty(&input.recipe_id, 256)
        || !non_empty(&input.label, 1024)
        || !non_empty(&input.command, 32 * 1024)
    {
        return Err("invalid-run-recipe-input".into());
    }
    if input
        .shell
        .as_deref()
        .is_some_and(|shell| !matches!(shell, "powershell" | "pwsh" | "git-bash" | "wsl"))
    {
        return Err("invalid-run-recipe-shell".into());
    }
    #[cfg(windows)]
    if input.shell.as_deref() == Some("wsl") {
        // taskkill은 WSL VM 안의 자식 트리까지 끝냈다는 보장을 못 한다. 앱이
        // 수명을 소유할 수 없는 실행은 시작하지 않는다.
        return Err("run-recipe-wsl-unsupported".into());
    }

    let root = PathBuf::from(super::paths::normalize_root(input.root.clone())?);
    if !root.is_dir() {
        return Err("invalid-run-recipe-root".into());
    }
    let cwd = match input.cwd.as_deref() {
        None => root,
        Some(cwd) if relative_cwd(cwd) => root.join(cwd),
        Some(_) => return Err("invalid-run-recipe-cwd".into()),
    };
    if !cwd.is_dir() {
        return Err("invalid-run-recipe-cwd".into());
    }
    Ok(cwd)
}

fn command_for_shell(shell: Option<&str>, command: &str) -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        // 프로필과 같은 셸을 고르되 PTY용 interactive 인자는 쓰지 않는다.
        let resolved = crate::session::shells::resolve_observed(shell, &[]);
        let program = resolved.program.to_ascii_lowercase().replace('\\', "/");
        let args = if program.ends_with("/bash.exe") || program == "bash.exe" {
            vec!["-lc".into(), command.into()]
        } else if program.ends_with("/wsl.exe") || program == "wsl.exe" {
            vec!["--".into(), "sh".into(), "-lc".into(), command.into()]
        } else {
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                command.into(),
            ]
        };
        (resolved.program, args)
    }
    #[cfg(not(windows))]
    {
        let resolved = crate::session::shells::resolve_observed(shell, &[]);
        (resolved.program, vec!["-lc".into(), command.into()])
    }
}

impl RunRecipeRuntime {
    pub fn start(self: &Arc<Self>, input: RunRecipeStartInput) -> Result<RunRecipeProcess, String> {
        let _operation = self.operations.lock().unwrap();
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("run-recipe-runtime-stopped".into());
        }
        let cwd = validate(&input)?;
        if self.entries.lock().unwrap().contains_key(&input.agent_id) {
            return Err("run-recipe-already-running".into());
        }

        let (program, args) = command_for_shell(input.shell.as_deref(), &input.command);
        let process = RunRecipeProcess {
            agent_id: input.agent_id.clone(),
            recipe_id: input.recipe_id,
            label: input.label,
            command: input.command,
            started_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: setsid는 fork 뒤 async-signal-safe하며 할당하지 않는다.
            unsafe {
                command.pre_exec(|| {
                    if libc::setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command
            .spawn()
            .map_err(|error| format!("run-recipe-spawn-failed: {error}"))?;
        let pid = child.id();
        let token = self.next_token.fetch_add(1, Ordering::Relaxed);
        {
            // 동시에 들어온 두 요청도 맵에는 하나만 남긴다. 진 쪽은 방금 띄운
            // 프로세스 트리를 바로 정리한다.
            let mut entries = self.entries.lock().unwrap();
            if entries.contains_key(&process.agent_id) {
                drop(entries);
                if terminate_pid(pid).is_err() {
                    let _ = child.kill();
                }
                let _ = child.wait();
                return Err("run-recipe-already-running".into());
            }
            entries.insert(
                process.agent_id.clone(),
                Entry {
                    pid,
                    token,
                    process: process.clone(),
                },
            );
        }

        let runtime = Arc::clone(self);
        let agent_id = process.agent_id.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            wait_for_process_tree(pid);
            let mut entries = runtime.entries.lock().unwrap();
            if entries
                .get(&agent_id)
                .is_some_and(|entry| entry.token == token)
            {
                entries.remove(&agent_id);
            }
        });
        Ok(process)
    }

    pub fn status(&self, agent_id: &str) -> Result<Option<RunRecipeProcess>, String> {
        if !non_empty(agent_id, 256) {
            return Err("invalid-run-recipe-agent-id".into());
        }
        Ok(self
            .entries
            .lock()
            .unwrap()
            .get(agent_id)
            .map(|entry| entry.process.clone()))
    }

    pub fn stop(&self, agent_id: &str) -> Result<(), String> {
        let _operation = self.operations.lock().unwrap();
        if !non_empty(agent_id, 256) {
            return Err("invalid-run-recipe-agent-id".into());
        }
        let entry = self.entries.lock().unwrap().get(agent_id).cloned();
        if let Some(entry) = entry {
            terminate_pid(entry.pid)?;
            let mut entries = self.entries.lock().unwrap();
            if entries
                .get(agent_id)
                .is_some_and(|current| current.token == entry.token)
            {
                entries.remove(agent_id);
            }
        }
        Ok(())
    }

    pub fn stop_all(&self) {
        self.shutting_down.store(true, Ordering::Release);
        let _operation = self.operations.lock().unwrap();
        let entries = std::mem::take(&mut *self.entries.lock().unwrap());
        terminate_all(entries.into_values().map(|entry| entry.pid));
    }
}

#[cfg(unix)]
fn signal_group(pid: u32, signal: i32) -> Result<(), String> {
    if unsafe { libc::kill(-(pid as i32), signal) } == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(format!("run-recipe-stop-failed: {error}"));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn group_exists(pid: u32) -> bool {
    if unsafe { libc::kill(-(pid as i32), 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(unix)]
fn wait_for_process_tree(pid: u32) {
    while group_exists(pid) {
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(windows)]
fn wait_for_process_tree(_pid: u32) {}

#[cfg(unix)]
fn terminate_pid(pid: u32) -> Result<(), String> {
    signal_group(pid, libc::SIGTERM)?;
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(25));
        if !group_exists(pid) {
            return Ok(());
        }
    }
    signal_group(pid, libc::SIGKILL)
}

#[cfg(unix)]
fn terminate_all(pids: impl Iterator<Item = u32>) {
    let pids: Vec<u32> = pids.collect();
    for &pid in &pids {
        let _ = signal_group(pid, libc::SIGTERM);
    }
    for _ in 0..10 {
        if pids.iter().all(|&pid| !group_exists(pid)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    for pid in pids {
        if group_exists(pid) {
            let _ = signal_group(pid, libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
fn terminate_pid(pid: u32) -> Result<(), String> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("run-recipe-stop-failed: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("run-recipe-stop-failed".into())
    }
}

#[cfg(windows)]
fn terminate_all(pids: impl Iterator<Item = u32>) {
    for pid in pids {
        let _ = terminate_pid(pid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_escaping_cwd() {
        assert!(!relative_cwd("../outside"));
        assert!(relative_cwd("web/app"));
    }

    #[test]
    fn rejects_control_characters() {
        assert!(!non_empty("a\0b", 8));
    }

    #[test]
    fn command_keeps_string_as_single_shell_argument() {
        let (_, args) = command_for_shell(None, "echo 'a b' && true");
        assert_eq!(args.last().unwrap(), "echo 'a b' && true");
    }

    #[test]
    #[cfg(unix)]
    fn starts_reports_and_reaps_a_process() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Arc::new(RunRecipeRuntime::default());
        let input = RunRecipeStartInput {
            agent_id: "a1".into(),
            recipe_id: "short".into(),
            label: "Short".into(),
            command: "sleep 0.15".into(),
            root: dir.path().to_string_lossy().into_owned(),
            cwd: None,
            shell: None,
        };

        let started = runtime.start(input).unwrap();
        assert_eq!(runtime.status("a1").unwrap(), Some(started));
        for _ in 0..100 {
            if runtime.status("a1").unwrap().is_none() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("completed process was not reaped");
    }

    #[test]
    #[cfg(unix)]
    fn duplicate_start_is_rejected_and_stop_clears_status() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Arc::new(RunRecipeRuntime::default());
        let input = RunRecipeStartInput {
            agent_id: "a1".into(),
            recipe_id: "long".into(),
            label: "Long".into(),
            command: "sleep 30".into(),
            root: dir.path().to_string_lossy().into_owned(),
            cwd: None,
            shell: None,
        };

        runtime.start(input.clone()).unwrap();
        assert_eq!(
            runtime.start(input).unwrap_err(),
            "run-recipe-already-running"
        );
        runtime.stop("a1").unwrap();
        assert_eq!(runtime.status("a1").unwrap(), None);
    }

    #[test]
    #[cfg(unix)]
    fn stop_kills_a_descendant_that_ignores_term() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let runtime = Arc::new(RunRecipeRuntime::default());
        runtime
            .start(RunRecipeStartInput {
                agent_id: "a1".into(),
                recipe_id: "tree".into(),
                label: "Tree".into(),
                command: format!(
                    "sh -c 'trap \"\" TERM; echo $$ > \"{}\"; while :; do sleep 1; done' &",
                    pid_file.display()
                ),
                root: dir.path().to_string_lossy().into_owned(),
                cwd: None,
                shell: None,
            })
            .unwrap();

        for _ in 0..50 {
            if pid_file.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let child_pid: i32 = std::fs::read_to_string(&pid_file)
            .expect("descendant pid file")
            .trim()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(child_pid, 0) }, 0);
        // 명령 셸은 이미 끝났어도 같은 그룹의 백그라운드 자식이 살아 있으면
        // 런타임은 소유권을 유지해야 한다.
        std::thread::sleep(Duration::from_millis(100));
        assert!(runtime.status("a1").unwrap().is_some());

        runtime.stop("a1").unwrap();
        for _ in 0..50 {
            if unsafe { libc::kill(child_pid, 0) } == -1
                && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("descendant process survived stop");
    }
}
