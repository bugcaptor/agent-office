//! `agent-office md` 오프라인 서브커맨드(docs/doc-view-design.md §구성 2). GUI·tokio 없이
//! Markdown 하나를 HTML로 바꿔 앱 캐시 `doc-view/`에 쓰고 경로를 찍은 뒤 기본 브라우저로
//! 연다. 열기는 먼저 `doc-view/open/`에 요청을 써서 샌드박스 밖의 앱에 맡기고, 앱이
//! 2초 안에 가져가지 않으면 직접 연다. 종료 코드: 0 성공, 1 파일·변환·열기 실패, 2 사용법 오류.

use crate::doc_view;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const USAGE: &str = "usage: office-md [--no-open] <file.md>
       office-md [--no-open] --latest [dir]
Converts a Markdown file to HTML and opens it in the default browser.
--latest picks the most recently modified .md under dir (default: the current folder).
--no-open only converts. Prints the absolute path of the generated HTML.
";

const EXIT_FAIL: i32 = 1;
const EXIT_USAGE: i32 = 2;
/// 앱 감시는 1초 간격이라 2초면 한 번은 훑는다.
const APP_WAIT: Duration = Duration::from_secs(2);
const APP_POLL: Duration = Duration::from_millis(100);

/// 실행 환경. 테스트가 프로세스 env·현재 폴더를 건드리지 않도록 값으로 받는다.
pub struct MdEnv {
    pub app_data: Option<PathBuf>,
    pub cwd: PathBuf,
}
impl MdEnv {
    fn from_process() -> Self {
        let app_data = std::env::var_os("AGENT_OFFICE_APP_DATA")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(crate::control::client::default_app_data);
        Self { app_data, cwd: std::env::current_dir().unwrap_or_default() }
    }
}

pub fn run(args: Vec<String>) -> i32 {
    let mut open = |data_dir: &Path, html: &Path| open_via_app(data_dir, html, APP_WAIT, APP_POLL, &mut open_in_browser);
    run_with(&args, &MdEnv::from_process(), &mut std::io::stdout(), &mut std::io::stderr(), &mut open)
}

pub fn run_with(
    args: &[String],
    env: &MdEnv,
    out: &mut dyn Write,
    err: &mut dyn Write,
    open: &mut dyn FnMut(&Path, &Path) -> Result<(), String>,
) -> i32 {
    let mut no_open = false;
    let mut latest = false;
    let mut positional: Vec<&str> = vec![];
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                let _ = out.write_all(USAGE.as_bytes());
                return 0;
            }
            "--no-open" => no_open = true,
            "--latest" => latest = true,
            "--" => positional.extend(iter.by_ref().map(String::as_str)),
            flag if flag.starts_with('-') && flag.len() > 1 => return usage_error(err, &format!("unknown option {flag}")),
            _ => positional.push(arg),
        }
    }
    let source = if latest {
        if positional.len() > 1 {
            return usage_error(err, "--latest takes at most one folder");
        }
        let dir = env.cwd.join(positional.first().copied().unwrap_or("."));
        if !dir.is_dir() {
            return fail(err, &format!("not a folder: {}", dir.display()));
        }
        match doc_view::latest_markdown(&dir) {
            Some(path) => path,
            None => return fail(err, &format!("no Markdown file under {}", dir.display())),
        }
    } else {
        let [file] = positional.as_slice() else { return usage_error(err, "expected one Markdown file") };
        let path = env.cwd.join(file);
        if !path.is_file() {
            return fail(err, &format!("no such file: {}", path.display()));
        }
        if !doc_view::has_markdown_extension(&path) {
            return fail(err, &format!("not a Markdown file (.md or .markdown): {}", path.display()));
        }
        path
    };
    let source = match std::fs::canonicalize(&source) {
        Ok(path) => path,
        Err(e) => return fail(err, &format!("failed to resolve {}: {e}", source.display())),
    };
    let markdown = match std::fs::read(&source) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(e) => return fail(err, &format!("failed to read {}: {e}", source.display())),
    };
    let Some(data_dir) = env.app_data.as_deref() else {
        return fail(err, "AGENT_OFFICE_APP_DATA is not set and the app data folder is unknown");
    };
    let html_path = doc_view::cache_path(data_dir, &source);
    let html = doc_view::render_document(&source, &markdown);
    if let Err(e) = crate::persistence::durable::write_atomic(&html_path, html.as_bytes()) {
        return fail(err, &format!("failed to write {}: {e}", html_path.display()));
    }
    // 열기에 실패해도 경로는 남겨 에이전트가 사용자에게 알려 줄 수 있게 한다.
    let _ = writeln!(out, "{}", html_path.display());
    if no_open {
        return 0;
    }
    match open(data_dir, &html_path) {
        Ok(()) => 0,
        Err(e) => fail(err, &format!("failed to open the browser: {e}")),
    }
}

/// 앱에 열기를 맡긴다(§구성 2). 요청을 쓰고 `wait` 동안 앱이 지우기를 기다린다. 그 뒤에도
/// 남아 있으면 요청을 지우고 `direct`로 직접 연다. 지우기 성공을 "내가 맡았다"로 삼으므로
/// (앱도 같은 규칙) 둘이 함께 여는 일은 없다. 요청을 쓸 수 없으면 바로 직접 연다.
fn open_via_app(
    data_dir: &Path,
    html: &Path,
    wait: Duration,
    poll: Duration,
    direct: &mut dyn FnMut(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let Ok(request) = doc_view::write_open_request(data_dir, html) else { return direct(html) };
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline {
        if !request.exists() {
            return Ok(());
        }
        std::thread::sleep(poll);
    }
    match std::fs::remove_file(&request) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        _ => direct(html),
    }
}

/// OS 기본 브라우저로 직접 연다 — 앱이 떠 있지 않을 때의 폴백.
fn open_in_browser(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = std::process::Command::new("cmd");
        command.args(["/c", "start", ""]);
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut command = std::process::Command::new("xdg-open");
    match command.arg(path).status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("opener exited with {status}")),
        Err(e) => Err(e.to_string()),
    }
}

fn fail(err: &mut dyn Write, reason: &str) -> i32 {
    let _ = writeln!(err, "office-md: {reason}");
    EXIT_FAIL
}

fn usage_error(err: &mut dyn Write, reason: &str) -> i32 {
    let _ = write!(err, "office-md: {reason}\n{USAGE}");
    EXIT_USAGE
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Run {
        code: i32,
        out: String,
        err: String,
        opened: Vec<PathBuf>,
    }

    fn run_in(cwd: &Path, app_data: Option<&Path>, args: &[&str], open_result: Result<(), String>) -> Run {
        let env = MdEnv { app_data: app_data.map(Path::to_path_buf), cwd: cwd.to_path_buf() };
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let (mut out, mut err, mut opened) = (vec![], vec![], vec![]);
        let code = run_with(&args, &env, &mut out, &mut err, &mut |_data: &Path, path: &Path| {
            opened.push(path.to_path_buf());
            open_result.clone()
        });
        Run { code, out: String::from_utf8(out).unwrap(), err: String::from_utf8(err).unwrap(), opened }
    }

    fn workspace() -> (tempfile::TempDir, tempfile::TempDir) {
        let work = tempfile::tempdir().unwrap();
        fs::create_dir_all(work.path().join("docs")).unwrap();
        fs::write(work.path().join("docs/plan.md"), "# Plan\n\nbody\n").unwrap();
        (work, tempfile::tempdir().unwrap())
    }

    #[test]
    fn no_open_writes_cache_html_and_prints_its_path() {
        let (work, data) = workspace();
        let run = run_in(work.path(), Some(data.path()), &["--no-open", "docs/plan.md"], Ok(()));
        assert_eq!(run.code, 0, "{}", run.err);
        let source = fs::canonicalize(work.path().join("docs/plan.md")).unwrap();
        let html = doc_view::cache_path(data.path(), &source);
        assert_eq!(run.out, format!("{}\n", html.display()));
        assert!(fs::read_to_string(&html).unwrap().contains("<title>Plan</title>"));
        assert!(run.opened.is_empty());
    }

    #[test]
    fn opens_after_conversion_and_reports_open_failure_with_path() {
        let (work, data) = workspace();
        let ok = run_in(work.path(), Some(data.path()), &["docs/plan.md"], Ok(()));
        assert_eq!(ok.code, 0, "{}", ok.err);
        assert_eq!(ok.opened, vec![PathBuf::from(ok.out.trim_end())]);
        let failed = run_in(work.path(), Some(data.path()), &["docs/plan.md"], Err("no browser".into()));
        assert_eq!(failed.code, 1);
        assert_eq!(failed.out, ok.out, "path is printed even when opening fails");
        assert!(failed.err.contains("no browser"), "{}", failed.err);
    }

    #[test]
    fn latest_uses_cwd_or_given_folder() {
        let (work, data) = workspace();
        fs::write(work.path().join("docs/newer.md"), "# Newer\n").unwrap();
        let file = fs::File::options().write(true).open(work.path().join("docs/plan.md")).unwrap();
        file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(60)).unwrap();
        for args in [&["--latest", "--no-open"][..], &["--no-open", "--latest", "docs"][..]] {
            let run = run_in(work.path(), Some(data.path()), args, Ok(()));
            assert_eq!(run.code, 0, "{args:?}: {}", run.err);
            assert!(run.out.trim_end().ends_with("-newer.html"), "{args:?}: {}", run.out);
        }
    }

    #[test]
    fn missing_wrong_or_empty_inputs_exit_1() {
        let (work, data) = workspace();
        fs::write(work.path().join("notes.txt"), "x").unwrap();
        fs::create_dir_all(work.path().join("empty")).unwrap();
        for args in [
            &["missing.md"][..],
            &["notes.txt"][..],
            &["docs"][..],
            &["--latest", "empty"][..],
            &["--latest", "nowhere"][..],
        ] {
            let run = run_in(work.path(), Some(data.path()), args, Ok(()));
            assert_eq!(run.code, 1, "{args:?}");
            assert!(run.err.starts_with("office-md: "), "{args:?}: {}", run.err);
            assert!(run.out.is_empty() && run.opened.is_empty(), "{args:?}");
        }
        let no_data = run_in(work.path(), None, &["docs/plan.md"], Ok(()));
        assert_eq!(no_data.code, 1);
        assert!(no_data.err.contains("AGENT_OFFICE_APP_DATA"), "{}", no_data.err);
    }

    #[test]
    fn usage_errors_exit_2_and_help_exits_0() {
        let (work, data) = workspace();
        for args in [&[][..], &["a.md", "b.md"][..], &["--latest", "a", "b"][..], &["--bogus", "docs/plan.md"][..]] {
            let run = run_in(work.path(), Some(data.path()), args, Ok(()));
            assert_eq!(run.code, 2, "{args:?}");
            assert!(run.err.contains("usage: office-md"), "{args:?}: {}", run.err);
        }
        let help = run_in(work.path(), Some(data.path()), &["--help"], Ok(()));
        assert_eq!(help.code, 0);
        assert!(help.out.starts_with("usage: office-md"));
        // `--` 뒤는 대시로 시작해도 파일 이름이다.
        fs::write(work.path().join("-dash.md"), "x").unwrap();
        let dashed = run_in(work.path(), Some(data.path()), &["--no-open", "--", "-dash.md"], Ok(()));
        assert_eq!(dashed.code, 0, "{}", dashed.err);
    }

    fn cached_html(data: &Path) -> PathBuf {
        let html = data.join(doc_view::CACHE_DIR).join("abc-plan.html");
        fs::create_dir_all(html.parent().unwrap()).unwrap();
        fs::write(&html, "<html>").unwrap();
        html
    }

    fn requests(data: &Path) -> Vec<PathBuf> {
        fs::read_dir(doc_view::request_dir(data)).map(|d| d.map(|e| e.unwrap().path()).collect()).unwrap_or_default()
    }

    #[test]
    fn app_takes_request_so_cli_does_not_open_directly() {
        let data = tempfile::tempdir().unwrap();
        let html = cached_html(data.path());
        let app_data = data.path().to_path_buf();
        let app = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let taken = doc_view::take_open_requests(&app_data);
                if !taken.is_empty() || Instant::now() > deadline {
                    return taken;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let mut direct = vec![];
        let result = open_via_app(data.path(), &html, Duration::from_secs(5), Duration::from_millis(10), &mut |p: &Path| {
            direct.push(p.to_path_buf());
            Ok(())
        });
        assert_eq!(result, Ok(()));
        assert!(direct.is_empty(), "the app opened it");
        assert_eq!(app.join().unwrap(), vec![fs::canonicalize(&html).unwrap()]);
        assert!(requests(data.path()).is_empty());
    }

    #[test]
    fn without_app_cli_removes_request_and_opens_directly() {
        let data = tempfile::tempdir().unwrap();
        let html = cached_html(data.path());
        for outcome in [Ok(()), Err("no browser".to_string())] {
            let mut direct = vec![];
            let result = open_via_app(data.path(), &html, Duration::from_millis(50), Duration::from_millis(10), &mut |p: &Path| {
                direct.push(p.to_path_buf());
                outcome.clone()
            });
            assert_eq!(result, outcome);
            assert_eq!(direct, vec![html.clone()]);
            assert!(requests(data.path()).is_empty(), "the request is withdrawn");
        }
    }

    #[test]
    fn unwritable_request_folder_falls_back_to_direct_open() {
        let data = tempfile::tempdir().unwrap();
        let html = cached_html(data.path());
        // 요청 폴더 자리에 파일이 있으면 요청을 쓸 수 없다.
        fs::write(doc_view::request_dir(data.path()), "x").unwrap();
        let mut direct = 0;
        let result = open_via_app(data.path(), &html, Duration::from_secs(5), Duration::from_millis(10), &mut |_: &Path| {
            direct += 1;
            Ok(())
        });
        assert_eq!((result, direct), (Ok(()), 1));
    }
}
