mod claude;
mod codex;
#[cfg(windows)]
mod windows_job;

use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{oneshot, OwnedSemaphorePermit, Semaphore};

use crate::persistence::settings_store::SummaryProvider;

const TEXT_MAX_CHARS: usize = 2_000;
const ERROR_MAX_CHARS: usize = 512;
const MAX_CONCURRENT: usize = 2;
const TIMEOUT: Duration = Duration::from_secs(20);
const CLEANUP_CONFIRM_TIMEOUT: Duration = Duration::from_secs(5);
const CLEANUP_RETRY_DELAY: Duration = Duration::from_secs(1);

pub(super) struct ProviderCommand {
    pub command: std::process::Command,
    pub provider: SummaryProvider,
}

fn permits() -> &'static Arc<Semaphore> {
    static PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    PERMITS.get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT)))
}

fn cap_text(text: &str) -> Result<String, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("validation: text is empty".to_string());
    }
    Ok(trimmed.chars().take(TEXT_MAX_CHARS).collect())
}

fn bounded_detail(detail: &str) -> String {
    detail.trim().chars().take(ERROR_MAX_CHARS).collect()
}

fn missing_error(provider: SummaryProvider) -> String {
    format!("{}-not-found", provider.as_str())
}

pub async fn summarize(
    provider: SummaryProvider,
    instruction: &str,
    text: &str,
) -> Result<String, String> {
    let capped = cap_text(text)?;
    let spec = match provider {
        SummaryProvider::Claude => claude::build(instruction),
        SummaryProvider::Codex => codex::build(instruction),
    };
    run_with_timeout(spec, &capped, TIMEOUT).await
}

async fn run_with_timeout(
    spec: ProviderCommand,
    text: &str,
    timeout: Duration,
) -> Result<String, String> {
    let permit = permits()
        .clone()
        .acquire_owned()
        .await
        .expect("semaphore is never closed");
    let (result_tx, result_rx) = oneshot::channel();
    let text = text.to_string();
    let supervisor = tokio::spawn(async move {
        supervise(spec, text, timeout, permit, result_tx).await;
    });
    // Dropping a JoinHandle detaches the task. The result receiver is the
    // cancellation boundary: its closure is selected inside the supervisor.
    drop(supervisor);
    result_rx
        .await
        .map_err(|_| "wait failed: summarizer supervisor stopped".to_string())?
}

async fn supervise(
    spec: ProviderCommand,
    text: String,
    timeout: Duration,
    permit: OwnedSemaphorePermit,
    mut result_tx: oneshot::Sender<Result<String, String>>,
) {
    match supervise_process(spec, &text, timeout, &mut result_tx).await {
        SuperviseOutcome::Finished(result) => {
            // Every spawned process reached confirmed cleanup before capacity
            // is returned, including when the caller dropped result_rx.
            drop(permit);
            let _ = result_tx.send(result);
        }
        SuperviseOutcome::Quarantine { error, owner } => {
            // Report the cleanup failure, but keep process/gate ownership and
            // capacity in this detached task until a retry confirms cleanup.
            let _ = result_tx.send(Err(error));
            quarantine_until_confirmed(owner, permit, CLEANUP_RETRY_DELAY).await;
        }
    }
}

async fn supervise_process(
    mut spec: ProviderCommand,
    text: &str,
    timeout: Duration,
    result_tx: &mut oneshot::Sender<Result<String, String>>,
) -> SuperviseOutcome {
    if result_tx.is_closed() {
        return SuperviseOutcome::Finished(Err("cancelled".to_string()));
    }
    spec.command.current_dir(std::env::temp_dir());
    spec.command.stdin(Stdio::piped());
    spec.command.stdout(Stdio::piped());
    spec.command.stderr(Stdio::piped());

    let containment = match ProcessContainment::new() {
        Ok(containment) => containment,
        Err(error) => {
            return SuperviseOutcome::Finished(Err(format!(
                "spawn failed: process containment: {}",
                bounded_detail(&error.to_string())
            )));
        }
    };
    let mut command: tokio::process::Command = spec.command.into();
    command.kill_on_drop(true);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            let error = if error.kind() == std::io::ErrorKind::NotFound {
                missing_error(spec.provider)
            } else {
                format!("spawn failed: {}", bounded_detail(&error.to_string()))
            };
            return SuperviseOutcome::Finished(Err(error));
        }
    };
    let mut owner = ProcessOwner {
        child,
        containment,
        assigned: false,
    };

    // Windows wrappers read stdin to EOF before invoking the provider. Assign
    // the still-blocked wrapper to its Job before taking or writing stdin; on
    // assignment failure, fail closed and reap it without releasing that gate.
    if let Err(error) = owner.containment.assign(&owner.child) {
        let message = format!(
            "spawn failed: process containment: {}",
            bounded_detail(&error.to_string())
        );
        return finish_after_cleanup(owner, Err(message)).await;
    }
    owner.assigned = true;

    let stdin = owner.child.stdin.take().expect("stdin was piped");
    let stdout = owner.child.stdout.take().expect("stdout was piped");
    let stderr = owner.child.stderr.take().expect("stderr was piped");
    let event = {
        let execution = execute_process(&mut owner.child, stdin, stdout, stderr, text);
        tokio::pin!(execution);
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        tokio::select! {
            result = &mut execution => SupervisorEvent::Completed(result),
            _ = &mut deadline => SupervisorEvent::Timeout,
            _ = result_tx.closed() => SupervisorEvent::Cancelled,
        }
    };

    let result = match event {
        SupervisorEvent::Completed(Ok(output)) => output,
        SupervisorEvent::Completed(Err(error)) => {
            return finish_after_cleanup(owner, Err(error)).await;
        }
        SupervisorEvent::Timeout => {
            return finish_after_cleanup(owner, Err("timeout".to_string())).await;
        }
        SupervisorEvent::Cancelled => {
            return finish_after_cleanup(owner, Err("cancelled".to_string())).await;
        }
    };

    let result = if !result.status.success() {
        if result.status.code() == Some(3) {
            Err(missing_error(spec.provider))
        } else {
            let code = result
                .status
                .code()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let detail = bounded_detail(&String::from_utf8_lossy(&result.stderr));
            Err(format!(
                "{} exited {code}: {detail}",
                spec.provider.as_str()
            ))
        }
    } else {
        let stdout = String::from_utf8_lossy(&result.stdout).trim().to_string();
        if stdout.is_empty() {
            Err("empty output".to_string())
        } else {
            Ok(stdout)
        }
    };
    finish_after_cleanup(owner, result).await
}

async fn finish_after_cleanup(
    mut owner: ProcessOwner,
    result: Result<String, String>,
) -> SuperviseOutcome {
    match owner.confirm_cleanup().await {
        Ok(()) => SuperviseOutcome::Finished(result),
        Err(cleanup) => {
            let context = match &result {
                Ok(_) => "provider completed".to_string(),
                Err(error) => error.clone(),
            };
            SuperviseOutcome::Quarantine {
                error: format!(
                    "{context}; cleanup unconfirmed: {}",
                    bounded_detail(&cleanup)
                ),
                owner,
            }
        }
    }
}

struct CapturedProcess {
    status: std::process::ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

enum SupervisorEvent {
    Completed(Result<CapturedProcess, String>),
    Timeout,
    Cancelled,
}

enum SuperviseOutcome {
    Finished(Result<String, String>),
    Quarantine { error: String, owner: ProcessOwner },
}

struct ProcessOwner {
    child: tokio::process::Child,
    containment: ProcessContainment,
    assigned: bool,
}

type CleanupFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + 'a>>;
type ProcessWaitFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = std::io::Result<()>> + Send + 'a>>;

trait CleanupDriver: Send {
    fn confirm_cleanup(&mut self) -> CleanupFuture<'_>;
}

trait ProcessControl: Send {
    fn exit_confirmed(&mut self) -> std::io::Result<bool>;
    fn request_kill(&mut self) -> std::io::Result<()>;
    fn wait_for_exit(&mut self) -> ProcessWaitFuture<'_>;
}

impl ProcessControl for tokio::process::Child {
    fn exit_confirmed(&mut self) -> std::io::Result<bool> {
        self.try_wait().map(|status| status.is_some())
    }

    fn request_kill(&mut self) -> std::io::Result<()> {
        self.start_kill()
    }

    fn wait_for_exit(&mut self) -> ProcessWaitFuture<'_> {
        Box::pin(async move { self.wait().await.map(|_| ()) })
    }
}

#[cfg(windows)]
trait JobControl: Send {
    fn request_termination(&self) -> std::io::Result<()>;
    fn active_processes(&self) -> std::io::Result<u32>;
    fn close_confirmed(&mut self);
}

impl CleanupDriver for ProcessOwner {
    fn confirm_cleanup(&mut self) -> CleanupFuture<'_> {
        Box::pin(async move {
            #[cfg(windows)]
            {
                if self.assigned {
                    return confirm_contained_cleanup(&mut self.child, &mut self.containment).await;
                }
            }

            confirm_uncontained_cleanup(&mut self.child).await
        })
    }
}

async fn quarantine_until_confirmed<D: CleanupDriver>(
    mut driver: D,
    permit: OwnedSemaphorePermit,
    retry_delay: Duration,
) {
    let _permit = permit;
    loop {
        if driver.confirm_cleanup().await.is_ok() {
            return;
        }
        tokio::time::sleep(retry_delay).await;
    }
}

async fn execute_process(
    child: &mut tokio::process::Child,
    mut stdin: tokio::process::ChildStdin,
    mut stdout: tokio::process::ChildStdout,
    mut stderr: tokio::process::ChildStderr,
    text: &str,
) -> Result<CapturedProcess, String> {
    let write_stdin = async {
        stdin.write_all(text.as_bytes()).await.map_err(|error| {
            format!("stdin write failed: {}", bounded_detail(&error.to_string()))
        })?;
        drop(stdin);
        Ok(())
    };
    let wait = async {
        child
            .wait()
            .await
            .map_err(|error| format!("wait failed: {}", bounded_detail(&error.to_string())))
    };
    let read_stdout = async {
        let mut bytes = Vec::new();
        stdout
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| format!("wait failed: {}", bounded_detail(&error.to_string())))?;
        Ok(bytes)
    };
    let read_stderr = async {
        let mut bytes = Vec::new();
        stderr
            .read_to_end(&mut bytes)
            .await
            .map_err(|error| format!("wait failed: {}", bounded_detail(&error.to_string())))?;
        Ok(bytes)
    };
    let (_, status, stdout, stderr) =
        tokio::try_join!(write_stdin, wait, read_stdout, read_stderr)?;
    Ok(CapturedProcess {
        status,
        stdout,
        stderr,
    })
}

#[cfg(windows)]
async fn confirm_contained_cleanup(
    child: &mut impl ProcessControl,
    containment: &mut impl JobControl,
) -> Result<(), String> {
    containment.request_termination().map_err(|error| {
        format!(
            "TerminateJobObject failed: {}",
            bounded_detail(&error.to_string())
        )
    })?;

    let wait_for_zero = async {
        loop {
            let active = containment.active_processes().map_err(|error| {
                format!(
                    "QueryInformationJobObject failed: {}",
                    bounded_detail(&error.to_string())
                )
            })?;
            if active == 0 {
                return Ok::<(), String>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    tokio::time::timeout(CLEANUP_CONFIRM_TIMEOUT, wait_for_zero)
        .await
        .map_err(|_| "Job Object active-process count did not reach zero".to_string())??;

    tokio::time::timeout(CLEANUP_CONFIRM_TIMEOUT, child.wait_for_exit())
        .await
        .map_err(|_| "root process wait timed out after Job cleanup".to_string())?
        .map_err(|error| {
            format!(
                "root process wait failed after Job cleanup: {}",
                bounded_detail(&error.to_string())
            )
        })?;
    containment.close_confirmed();
    Ok(())
}

async fn confirm_uncontained_cleanup(child: &mut impl ProcessControl) -> Result<(), String> {
    // Assignment failure occurs before stdin is taken. Request termination
    // before Child::wait can close stdin and release the provider-invocation
    // gate. On start_kill failure, ProcessOwner retains Child and its stdin.
    match child.exit_confirmed() {
        Ok(true) => return Ok(()),
        Ok(false) => child.request_kill().map_err(|error| {
            format!("start_kill failed: {}", bounded_detail(&error.to_string()))
        })?,
        Err(error) => {
            return Err(format!(
                "process exit query failed: {}",
                bounded_detail(&error.to_string())
            ));
        }
    }

    tokio::time::timeout(CLEANUP_CONFIRM_TIMEOUT, child.wait_for_exit())
        .await
        .map_err(|_| "process wait timed out after start_kill".to_string())?
        .map(|_| ())
        .map_err(|error| {
            format!(
                "process wait failed after start_kill: {}",
                bounded_detail(&error.to_string())
            )
        })
}

#[cfg(windows)]
struct ProcessContainment(windows_job::WindowsJob);

#[cfg(windows)]
impl ProcessContainment {
    fn new() -> std::io::Result<Self> {
        windows_job::WindowsJob::new().map(Self)
    }

    fn assign(&self, child: &tokio::process::Child) -> std::io::Result<()> {
        self.0.assign(child)
    }
}

#[cfg(windows)]
impl JobControl for ProcessContainment {
    fn request_termination(&self) -> std::io::Result<()> {
        self.0.request_termination()
    }

    fn active_processes(&self) -> std::io::Result<u32> {
        self.0.active_processes()
    }

    fn close_confirmed(&mut self) {
        self.0.close_confirmed();
    }
}

#[cfg(not(windows))]
struct ProcessContainment;

#[cfg(not(windows))]
impl ProcessContainment {
    fn new() -> std::io::Result<Self> {
        Ok(Self)
    }

    fn assign(&self, _child: &tokio::process::Child) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::settings_store::SummaryProvider;
    use std::sync::atomic::{AtomicBool, Ordering};

    static PROCESS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn path_test_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    struct EnvGuard {
        saved: Vec<(std::ffi::OsString, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn set(values: &[(&str, std::ffi::OsString)]) -> Self {
            let mut saved = Vec::with_capacity(values.len());
            for (key, value) in values {
                saved.push(((*key).into(), std::env::var_os(key)));
                std::env::set_var(key, value);
            }
            Self { saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..).rev() {
                match value {
                    Some(value) => std::env::set_var(&key, value),
                    None => std::env::remove_var(&key),
                }
            }
        }
    }

    struct FakeCliDir {
        root: std::path::PathBuf,
        args: std::path::PathBuf,
        stdin: std::path::PathBuf,
        pid: std::path::PathBuf,
        calls: std::path::PathBuf,
        claude_marker: std::path::PathBuf,
    }

    impl FakeCliDir {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "agent-office-fake-summarizer-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&root).unwrap();

            #[cfg(windows)]
            {
                std::fs::write(
                    root.join("codex.ps1"),
                    r#"$ErrorActionPreference='Stop'
[IO.File]::WriteAllLines($env:AO_FAKE_ARGS, [string[]]$args)
[IO.File]::WriteAllText($env:AO_FAKE_STDIN, (@($input) -join [Environment]::NewLine))
[IO.File]::WriteAllText($env:AO_FAKE_PID, "$PID")
$count = 0
if ([IO.File]::Exists($env:AO_FAKE_CALLS)) { $count = [int][IO.File]::ReadAllText($env:AO_FAKE_CALLS) }
[IO.File]::WriteAllText($env:AO_FAKE_CALLS, "$($count + 1)")
if ($env:AO_FAKE_SLEEP_SECONDS) { Start-Sleep -Seconds ([int]$env:AO_FAKE_SLEEP_SECONDS) }
Write-Output 'Codex fake summary'
exit ([int]$env:AO_FAKE_EXIT)
"#,
                )
                .unwrap();
                std::fs::write(
                    root.join("claude.ps1"),
                    "[IO.File]::WriteAllText($env:AO_FAKE_CLAUDE_MARKER, 'invoked')\nexit 0\n",
                )
                .unwrap();
            }

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let codex = root.join("codex");
                std::fs::write(
                    &codex,
                    r#"#!/bin/sh
printf '%s\n' "$@" > "$AO_FAKE_ARGS"
printf '%s' "$$" > "$AO_FAKE_PID"
calls=0
[ -f "$AO_FAKE_CALLS" ] && calls=$(cat "$AO_FAKE_CALLS")
calls=$((calls + 1))
printf '%s' "$calls" > "$AO_FAKE_CALLS"
cat > "$AO_FAKE_STDIN"
[ -n "$AO_FAKE_SLEEP_SECONDS" ] && sleep "$AO_FAKE_SLEEP_SECONDS"
printf '%s\n' 'Codex fake summary'
exit "$AO_FAKE_EXIT"
"#,
                )
                .unwrap();
                std::fs::set_permissions(&codex, std::fs::Permissions::from_mode(0o755)).unwrap();
                let claude = root.join("claude");
                std::fs::write(
                    &claude,
                    "#!/bin/sh\nprintf '%s' invoked > \"$AO_FAKE_CLAUDE_MARKER\"\nexit 0\n",
                )
                .unwrap();
                std::fs::set_permissions(&claude, std::fs::Permissions::from_mode(0o755)).unwrap();
            }

            Self {
                args: root.join("codex.args"),
                stdin: root.join("codex.stdin"),
                pid: root.join("codex.pid"),
                calls: root.join("codex.calls"),
                claude_marker: root.join("claude-invoked.marker"),
                root,
            }
        }

        fn environment(&self, exit: &str, sleep_seconds: &str) -> EnvGuard {
            let inherited_path = std::env::var_os("PATH").unwrap_or_default();
            let path =
                std::env::join_paths(std::iter::once(self.root.as_os_str().to_os_string()).chain(
                    std::env::split_paths(&inherited_path).map(|path| path.into_os_string()),
                ))
                .unwrap();
            EnvGuard::set(&[
                ("PATH", path),
                ("AO_FAKE_ARGS", self.args.as_os_str().to_os_string()),
                ("AO_FAKE_STDIN", self.stdin.as_os_str().to_os_string()),
                ("AO_FAKE_PID", self.pid.as_os_str().to_os_string()),
                ("AO_FAKE_CALLS", self.calls.as_os_str().to_os_string()),
                (
                    "AO_FAKE_CLAUDE_MARKER",
                    self.claude_marker.as_os_str().to_os_string(),
                ),
                ("AO_FAKE_EXIT", exit.into()),
                ("AO_FAKE_SLEEP_SECONDS", sleep_seconds.into()),
            ])
        }
    }

    impl Drop for FakeCliDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    struct DropSignal(Arc<AtomicBool>);

    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    struct InjectedProcessControl {
        exited: Arc<AtomicBool>,
        kill_failure: Option<&'static str>,
        first_failure: Arc<tokio::sync::Notify>,
    }

    impl ProcessControl for InjectedProcessControl {
        fn exit_confirmed(&mut self) -> std::io::Result<bool> {
            Ok(self.exited.load(Ordering::SeqCst))
        }

        fn request_kill(&mut self) -> std::io::Result<()> {
            if let Some(error) = self.kill_failure {
                self.first_failure.notify_one();
                return Err(std::io::Error::other(error));
            }
            Ok(())
        }

        fn wait_for_exit(&mut self) -> ProcessWaitFuture<'_> {
            Box::pin(async { Ok(()) })
        }
    }

    struct InjectedUncontainedOwner {
        process: InjectedProcessControl,
        _drop_signal: DropSignal,
    }

    impl CleanupDriver for InjectedUncontainedOwner {
        fn confirm_cleanup(&mut self) -> CleanupFuture<'_> {
            Box::pin(confirm_uncontained_cleanup(&mut self.process))
        }
    }

    #[cfg(windows)]
    #[derive(Clone, Copy)]
    enum InjectedJobFailure {
        Terminate,
        Query,
    }

    #[cfg(windows)]
    struct InjectedJobControl {
        failure: InjectedJobFailure,
        allow_confirmation: Arc<AtomicBool>,
        first_failure: Arc<tokio::sync::Notify>,
        closed: Arc<AtomicBool>,
    }

    #[cfg(windows)]
    impl JobControl for InjectedJobControl {
        fn request_termination(&self) -> std::io::Result<()> {
            if matches!(self.failure, InjectedJobFailure::Terminate)
                && !self.allow_confirmation.load(Ordering::SeqCst)
            {
                self.first_failure.notify_one();
                return Err(std::io::Error::other("injected TerminateJobObject failure"));
            }
            Ok(())
        }

        fn active_processes(&self) -> std::io::Result<u32> {
            if matches!(self.failure, InjectedJobFailure::Query)
                && !self.allow_confirmation.load(Ordering::SeqCst)
            {
                self.first_failure.notify_one();
                return Err(std::io::Error::other(
                    "injected QueryInformationJobObject failure",
                ));
            }
            Ok(0)
        }

        fn close_confirmed(&mut self) {
            self.closed.store(true, Ordering::SeqCst);
        }
    }

    #[cfg(windows)]
    struct InjectedContainedOwner {
        process: InjectedProcessControl,
        containment: InjectedJobControl,
        _drop_signal: DropSignal,
    }

    #[cfg(windows)]
    impl CleanupDriver for InjectedContainedOwner {
        fn confirm_cleanup(&mut self) -> CleanupFuture<'_> {
            Box::pin(confirm_contained_cleanup(
                &mut self.process,
                &mut self.containment,
            ))
        }
    }

    async fn assert_unconfirmed_cleanup_quarantines_capacity<D: CleanupDriver + 'static>(
        driver: D,
        first_failure: Arc<tokio::sync::Notify>,
        allow_confirmation: Arc<AtomicBool>,
        dropped: Arc<AtomicBool>,
        closed: Option<Arc<AtomicBool>>,
    ) {
        let semaphore = Arc::new(Semaphore::new(1));
        let permit = semaphore.clone().acquire_owned().await.unwrap();

        let quarantine = tokio::spawn(quarantine_until_confirmed(
            driver,
            permit,
            Duration::from_millis(1),
        ));
        first_failure.notified().await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), semaphore.acquire())
                .await
                .is_err(),
            "capacity returned after unconfirmed cleanup"
        );
        assert!(
            !dropped.load(Ordering::SeqCst),
            "process/gate owner dropped after unconfirmed cleanup"
        );
        if let Some(closed) = &closed {
            assert!(
                !closed.load(Ordering::SeqCst),
                "Job handle closed before active-process-zero confirmation"
            );
        }

        allow_confirmation.store(true, Ordering::SeqCst);
        quarantine.await.unwrap();
        assert!(dropped.load(Ordering::SeqCst));
        if let Some(closed) = closed {
            assert!(closed.load(Ordering::SeqCst));
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(200), semaphore.acquire())
                .await
                .is_ok(),
            "capacity was not returned after confirmed cleanup"
        );
    }

    #[tokio::test]
    async fn assignment_kill_failure_quarantines_gate_and_capacity_until_exit_confirmed() {
        let exited = Arc::new(AtomicBool::new(false));
        let first_failure = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let owner = InjectedUncontainedOwner {
            process: InjectedProcessControl {
                exited: exited.clone(),
                kill_failure: Some("injected start_kill failure"),
                first_failure: first_failure.clone(),
            },
            _drop_signal: DropSignal(dropped.clone()),
        };
        assert_unconfirmed_cleanup_quarantines_capacity(
            owner,
            first_failure,
            exited,
            dropped,
            None,
        )
        .await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn job_terminate_failure_quarantines_capacity_until_retry_confirms_zero() {
        assert_injected_job_failure_quarantines(InjectedJobFailure::Terminate).await;
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn job_query_failure_quarantines_capacity_until_retry_confirms_zero() {
        assert_injected_job_failure_quarantines(InjectedJobFailure::Query).await;
    }

    #[cfg(windows)]
    async fn assert_injected_job_failure_quarantines(failure: InjectedJobFailure) {
        let allow_confirmation = Arc::new(AtomicBool::new(false));
        let first_failure = Arc::new(tokio::sync::Notify::new());
        let dropped = Arc::new(AtomicBool::new(false));
        let closed = Arc::new(AtomicBool::new(false));
        let owner = InjectedContainedOwner {
            process: InjectedProcessControl {
                exited: Arc::new(AtomicBool::new(true)),
                kill_failure: None,
                first_failure: first_failure.clone(),
            },
            containment: InjectedJobControl {
                failure,
                allow_confirmation: allow_confirmation.clone(),
                first_failure: first_failure.clone(),
                closed: closed.clone(),
            },
            _drop_signal: DropSignal(dropped.clone()),
        };
        assert_unconfirmed_cleanup_quarantines_capacity(
            owner,
            first_failure,
            allow_confirmation,
            dropped,
            Some(closed),
        )
        .await;
    }

    fn command_that_does_not_read_stdin() -> ProviderCommand {
        #[cfg(windows)]
        use std::os::windows::process::CommandExt;
        #[cfg(windows)]
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut command = std::process::Command::new("node");
        command.args(["-e", "setTimeout(() => {}, 2000)"]);
        #[cfg(windows)]
        command.creation_flags(CREATE_NO_WINDOW);
        ProviderCommand {
            command,
            provider: SummaryProvider::Codex,
        }
    }

    #[cfg(windows)]
    fn windows_process_exists(pid: u32) -> bool {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let filter = format!("PID eq {pid}");
        let output = std::process::Command::new("tasklist.exe")
            .args(["/FI", &filter, "/FO", "CSV", "/NH"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).contains(&format!(",\"{pid}\","))
    }

    #[cfg(windows)]
    fn kill_process_for_test(pid: u32) {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = std::process::Command::new("taskkill.exe")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    }

    #[cfg(windows)]
    fn process_is_running(pid: u32) -> bool {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let script = format!("Get-Process -Id {pid} -ErrorAction Stop | Out-Null");
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(unix)]
    fn process_is_running(pid: u32) -> bool {
        let proc_path = std::path::PathBuf::from(format!("/proc/{pid}"));
        if std::path::Path::new("/proc").is_dir() {
            return proc_path.exists();
        }
        let pid = pid.to_string();
        std::process::Command::new("kill")
            .args(["-0", &pid])
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    async fn wait_until_stopped(pid: u32) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        while process_is_running(pid) {
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        true
    }

    #[cfg(windows)]
    async fn wait_for_pid_files(root_file: &std::path::Path, descendant_file: &std::path::Path) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !(root_file.exists() && descendant_file.exists()) {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("fake provider did not record its process tree");
    }

    #[cfg(windows)]
    fn install_fake_codex_tree(
        dir: &std::path::Path,
        root_file: &std::path::Path,
        descendant_file: &std::path::Path,
    ) -> EnvGuard {
        std::fs::write(
            dir.join("fake-provider.ps1"),
            r#"$ErrorActionPreference='Stop'
Set-Content -LiteralPath $env:AO_ROOT_PID_FILE -Value $PID -NoNewline -Encoding ascii
$descendant = Start-Process powershell.exe -NoNewWindow -PassThru -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60')
Set-Content -LiteralPath $env:AO_DESCENDANT_PID_FILE -Value $descendant.Id -NoNewline -Encoding ascii
[Console]::In.ReadToEnd() | Out-Null
Start-Sleep -Seconds 60"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("codex.cmd"),
            "@echo off\r\npowershell.exe -NoProfile -NonInteractive -File \"%~dp0fake-provider.ps1\"\r\nexit /b %ERRORLEVEL%\r\n",
        )
        .unwrap();

        let original_path = std::env::var_os("PATH").unwrap_or_default();
        let path = std::env::join_paths(
            std::iter::once(dir.to_path_buf()).chain(std::env::split_paths(&original_path)),
        )
        .unwrap();
        EnvGuard::set(&[
            ("PATH", path),
            ("AO_ROOT_PID_FILE", root_file.as_os_str().to_os_string()),
            (
                "AO_DESCENDANT_PID_FILE",
                descendant_file.as_os_str().to_os_string(),
            ),
        ])
    }

    #[tokio::test]
    async fn rejects_empty_text_before_spawning_a_provider() {
        let err = summarize(SummaryProvider::Codex, "요약하라", "   ")
            .await
            .unwrap_err();
        assert_eq!(err, "validation: text is empty");
    }

    #[test]
    fn cap_text_counts_unicode_scalars_not_bytes() {
        let input = "가".repeat(TEXT_MAX_CHARS + 5);
        assert_eq!(cap_text(&input).unwrap().chars().count(), TEXT_MAX_CHARS);
    }

    #[test]
    fn error_detail_is_bounded() {
        let bounded = bounded_detail(&"x".repeat(ERROR_MAX_CHARS + 50));
        assert_eq!(bounded.chars().count(), ERROR_MAX_CHARS);
    }

    #[tokio::test]
    async fn fake_cli_codex_dispatcher_preserves_exact_argv_stdin_and_summary() {
        let _path_lock = path_test_lock().lock().unwrap();
        let _process_lock = PROCESS_TEST_LOCK.lock().await;
        let fake = FakeCliDir::new();
        let _env = fake.environment("0", "");

        let result = summarize(SummaryProvider::Codex, "요약 지시", "한글 원문")
            .await
            .unwrap();

        assert_eq!(result, "Codex fake summary");
        assert_eq!(std::fs::read_to_string(&fake.stdin).unwrap(), "한글 원문");
        let args = std::fs::read_to_string(&fake.args)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            vec![
                "exec",
                "--ignore-user-config",
                "--ignore-rules",
                "--ephemeral",
                "--sandbox",
                "read-only",
                "--model",
                "gpt-5.4-mini",
                "--config",
                "model_reasoning_effort=\"low\"",
                "--skip-git-repo-check",
                "--color",
                "never",
                "--",
                "요약 지시",
            ]
        );
        assert_eq!(args.last().map(String::as_str), Some("요약 지시"));
        assert_eq!(std::fs::read_to_string(&fake.calls).unwrap(), "1");
    }

    #[tokio::test]
    async fn fake_cli_nonzero_codex_runs_once_without_claude_fallback() {
        let _path_lock = path_test_lock().lock().unwrap();
        let _process_lock = PROCESS_TEST_LOCK.lock().await;
        let fake = FakeCliDir::new();
        let _env = fake.environment("7", "");

        let error = summarize(SummaryProvider::Codex, "요약 지시", "원문")
            .await
            .unwrap_err();

        assert!(error.starts_with("codex exited 7:"), "{error}");
        assert_eq!(
            std::fs::read_to_string(&fake.calls).unwrap(),
            "1",
            "Codex must be invoked exactly once"
        );
        assert!(
            !fake.claude_marker.exists(),
            "Claude fallback must never run"
        );
    }

    #[tokio::test]
    async fn fake_cli_timeout_reaps_root_process_quickly() {
        let _path_lock = path_test_lock().lock().unwrap();
        let _process_lock = PROCESS_TEST_LOCK.lock().await;
        let fake = FakeCliDir::new();
        let _env = fake.environment("0", "60");
        let spec = codex::build("요약 지시");
        #[cfg(windows)]
        let fake_timeout = Duration::from_millis(500);
        #[cfg(unix)]
        let fake_timeout = Duration::from_millis(50);

        let started = std::time::Instant::now();
        let error = run_with_timeout(spec, "원문", fake_timeout)
            .await
            .unwrap_err();

        assert_eq!(error, "timeout");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "timeout boundary returned too slowly: {:?}",
            started.elapsed()
        );
        assert!(
            fake.pid.is_file(),
            "timeout expired before the fake provider boundary recorded its root PID"
        );
        assert_eq!(
            std::fs::read_to_string(&fake.calls).unwrap(),
            "1",
            "timeout must cross the fake provider boundary exactly once"
        );
        let pid: u32 = std::fs::read_to_string(&fake.pid)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            wait_until_stopped(pid).await,
            "timed-out summarizer root process survived: {pid}"
        );
    }

    #[tokio::test]
    async fn timeout_deadline_includes_stdin_write() {
        let _test_lock = PROCESS_TEST_LOCK.lock().await;
        let started = tokio::time::Instant::now();
        let error = run_with_timeout(
            command_that_does_not_read_stdin(),
            &"x".repeat(16 * 1024 * 1024),
            Duration::from_millis(100),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "timeout");
        assert!(
            started.elapsed() < Duration::from_millis(1_500),
            "deadline excluded stdin write: {:?}",
            started.elapsed()
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn timeout_kills_and_reaps_entire_windows_process_tree() {
        let _test_lock = PROCESS_TEST_LOCK.lock().await;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
Set-Content -LiteralPath $env:AO_ROOT_PID_FILE -Value $PID -NoNewline -Encoding ascii
$descendant = Start-Process powershell.exe -WindowStyle Hidden -PassThru -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60')
Set-Content -LiteralPath $env:AO_DESCENDANT_PID_FILE -Value $descendant.Id -NoNewline -Encoding ascii
[Console]::In.ReadToEnd() | Out-Null
Start-Sleep -Seconds 60"#;

        let dir = std::env::temp_dir().join(format!("ao-timeout-tree-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let root_file = dir.join("root.pid");
        let descendant_file = dir.join("descendant.pid");
        let mut command = std::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]);
        command.creation_flags(CREATE_NO_WINDOW);
        command.env("AO_ROOT_PID_FILE", &root_file);
        command.env("AO_DESCENDANT_PID_FILE", &descendant_file);

        let error = run_with_timeout(
            ProviderCommand {
                command,
                provider: SummaryProvider::Codex,
            },
            "test input",
            Duration::from_secs(3),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "timeout");

        let root_pid: u32 = std::fs::read_to_string(&root_file)
            .unwrap()
            .parse()
            .unwrap();
        let descendant_pid: u32 = std::fs::read_to_string(&descendant_file)
            .unwrap()
            .parse()
            .unwrap();
        let root_alive = windows_process_exists(root_pid);
        let descendant_alive = windows_process_exists(descendant_pid);
        if root_alive {
            kill_process_for_test(root_pid);
        }
        if descendant_alive {
            kill_process_for_test(descendant_pid);
        }
        std::fs::remove_dir_all(dir).unwrap();

        assert!(
            !root_alive,
            "timed-out root process {root_pid} still exists"
        );
        assert!(
            !descendant_alive,
            "timed-out descendant process {descendant_pid} still exists"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn successful_provider_confirms_job_active_processes_zero_before_return() {
        let _test_lock = PROCESS_TEST_LOCK.lock().await;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$descendant = Start-Process powershell.exe -WindowStyle Hidden -PassThru -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60')
Set-Content -LiteralPath $env:AO_DESCENDANT_PID_FILE -Value $descendant.Id -NoNewline -Encoding ascii
[Console]::In.ReadToEnd() | Out-Null
[Console]::Out.WriteLine('summary')"#;

        let dir = std::env::temp_dir().join(format!("ao-success-tree-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let descendant_file = dir.join("descendant.pid");
        let mut command = std::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]);
        command.creation_flags(CREATE_NO_WINDOW);
        command.env("AO_DESCENDANT_PID_FILE", &descendant_file);

        let summary = run_with_timeout(
            ProviderCommand {
                command,
                provider: SummaryProvider::Codex,
            },
            "test input",
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        let descendant_pid: u32 = std::fs::read_to_string(&descendant_file)
            .unwrap()
            .parse()
            .unwrap();
        let descendant_alive = windows_process_exists(descendant_pid);
        if descendant_alive {
            kill_process_for_test(descendant_pid);
        }
        std::fs::remove_dir_all(dir).unwrap();

        assert_eq!(summary, "summary");
        assert!(
            !descendant_alive,
            "successful return preceded active-process-zero confirmation for descendant {descendant_pid}"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn inherited_pipe_descendant_is_bounded_by_lifecycle_deadline() {
        let _test_lock = PROCESS_TEST_LOCK.lock().await;
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const SCRIPT: &str = r#"$ErrorActionPreference='Stop'
$descendant = Start-Process powershell.exe -NoNewWindow -PassThru -ArgumentList @('-NoProfile','-NonInteractive','-Command','Start-Sleep -Seconds 60')
Set-Content -LiteralPath $env:AO_DESCENDANT_PID_FILE -Value $descendant.Id -NoNewline -Encoding ascii
exit 0"#;

        let dir = std::env::temp_dir().join(format!("ao-reader-tree-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let descendant_file = dir.join("descendant.pid");
        let mut command = std::process::Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT]);
        command.creation_flags(CREATE_NO_WINDOW);
        command.env("AO_DESCENDANT_PID_FILE", &descendant_file);

        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            run_with_timeout(
                ProviderCommand {
                    command,
                    provider: SummaryProvider::Codex,
                },
                "test input",
                Duration::from_secs(2),
            ),
        )
        .await;
        let descendant_pid: u32 = std::fs::read_to_string(&descendant_file)
            .unwrap()
            .parse()
            .unwrap();
        let descendant_alive = windows_process_exists(descendant_pid);
        if descendant_alive {
            kill_process_for_test(descendant_pid);
        }
        std::fs::remove_dir_all(dir).unwrap();

        let error = outcome
            .expect("stdout/stderr readers outlived the lifecycle deadline")
            .unwrap_err();
        assert_eq!(error, "timeout");
        assert!(
            !descendant_alive,
            "reader deadline returned before descendant {descendant_pid} cleanup"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn cancelling_summarize_kills_tree_before_releasing_permit_without_reader_hang() {
        let _path_lock = path_test_lock().lock().unwrap();
        let _test_lock = PROCESS_TEST_LOCK.lock().await;
        let first_permit = permits().acquire().await.unwrap();
        let dir = std::env::temp_dir().join(format!("ao-cancel-tree-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let root_file = dir.join("root.pid");
        let descendant_file = dir.join("descendant.pid");
        let _environment = install_fake_codex_tree(&dir, &root_file, &descendant_file);

        let summarize_task = tokio::spawn(async {
            summarize(SummaryProvider::Codex, "summarize", "test input").await
        });
        wait_for_pid_files(&root_file, &descendant_file).await;
        let root_pid: u32 = std::fs::read_to_string(&root_file)
            .unwrap()
            .parse()
            .unwrap();
        let descendant_pid: u32 = std::fs::read_to_string(&descendant_file)
            .unwrap()
            .parse()
            .unwrap();

        summarize_task.abort();
        assert!(summarize_task.await.unwrap_err().is_cancelled());
        let replacement_permit = tokio::time::timeout(Duration::from_secs(5), permits().acquire())
            .await
            .expect("supervisor cleanup or inherited-pipe readers hung")
            .unwrap();
        let root_alive = windows_process_exists(root_pid);
        let descendant_alive = windows_process_exists(descendant_pid);
        if root_alive {
            kill_process_for_test(root_pid);
        }
        if descendant_alive {
            kill_process_for_test(descendant_pid);
        }
        drop(replacement_permit);
        drop(first_permit);
        std::fs::remove_dir_all(dir).unwrap();

        assert!(
            !root_alive,
            "permit released before cancelled root {root_pid} was cleaned up"
        );
        assert!(
            !descendant_alive,
            "permit released before cancelled descendant {descendant_pid} was cleaned up"
        );
    }

    #[tokio::test]
    async fn global_semaphore_allows_two_and_blocks_a_third() {
        let _test_lock = PROCESS_TEST_LOCK.lock().await;
        let semaphore = permits();
        assert!(std::ptr::eq(semaphore, permits()));
        let first = semaphore.acquire().await.unwrap();
        let second = semaphore.acquire().await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), semaphore.acquire(),)
                .await
                .is_err()
        );
        drop(first);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), semaphore.acquire(),)
                .await
                .is_ok()
        );
        drop(second);
    }
}
