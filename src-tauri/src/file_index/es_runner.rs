// src-tauri/src/file_index/es_runner.rs
//
// es.exe(Voidtools "Everything" CLI, https://www.voidtools.com/support/everything/command_line_interface/)
// 서브프로세스 실행. workdir/git_runner.rs::run_git 패턴을 복제하되 대상이
// es.exe다: Windows 전용, `CREATE_NO_WINDOW`로 콘솔 창 깜빡임 방지, stdout은
// 별도 스레드로 읽어 파이프 교착을 막고, 3초 타임아웃을 넘기면 kill.
//
// 실패는 전부 조용히 `None`으로 폴백한다(에러가 아니다) -- es.exe 부재(비
// Windows·미설치)·스폰 실패·타임아웃·비정상 종료·32MB 초과 출력 전부. 호출부
// (file_index/mod.rs)는 `None`을 받으면 기존 워커(WalkBuilder) 경로로
// 그대로 넘어간다.

use std::path::{Path, PathBuf};

/// es.exe stdout을 무제한으로 버퍼링하지 않기 위한 상한. 이 이상 나오면
/// 인덱스가 통째로 어긋났거나(잘못된 쿼리) 비정상 상황으로 보고 폴백한다.
const MAX_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
/// es.exe 응답 타임아웃. 로컬 인덱스 조회이므로 수 초 안에 끝나야 정상이다.
const TIMEOUT_SECS: u64 = 3;

/// root 아래 마크다운 후보(절대경로)를 es.exe로 찾는다. `-path`는 부분일치라
/// 형제 디렉터리도 걸릴 수 있으므로, 호출부가 아니라 여기서 이미
/// `starts_with(canon_root)`로 재검증해 반환한다.
pub fn find_markdown_candidates(canon_root: &Path) -> Option<Vec<PathBuf>> {
    let output = run_es(canon_root, &["ext:md;mdx;markdown"])?;
    Some(parse_paths_under_root(canon_root, &output))
}

/// root 아래 `.gitignore` 파일(절대경로)을 es.exe로 찾는다. basename이 정확히
/// `.gitignore`인 것만 남긴다(`-path`류 필터의 부분일치 오탐 제거).
pub fn find_gitignore_files(canon_root: &Path) -> Option<Vec<PathBuf>> {
    let output = run_es(canon_root, &[".gitignore"])?;
    let mut files = parse_paths_under_root(canon_root, &output);
    files.retain(|p| p.file_name().and_then(|n| n.to_str()) == Some(".gitignore"));
    Some(files)
}

/// 팔레트 검색어(공백 구분 토큰)를 es.exe argv로 변환한다. 각 토큰의 `"`
/// 문자를 제거한 뒤 `path:<tok>`로 만들고, 맨 앞에 `file:`(디렉터리 제외,
/// 파일만 매칭)을 붙인다. 예: `workdir tsx` -> `["file:", "path:workdir",
/// "path:tsx"]`. 토큰이 하나도 없으면(빈 문자열/공백뿐) `None` -- 호출부가
/// 검색을 시도조차 하지 않게 한다.
///
/// es.exe는 `file:`·`path:...`를 하나의 문자열이 아니라 **별도 argv 원소**로
/// 받아야 파싱한다(단일 인자로 합쳐 넘기면 0건). 또한 값에 `"` 리터럴이
/// 섞이면 es가 파싱에 실패해 역시 0건이 되므로 따옴표는 절대 붙이지 않는다.
///
/// Everything 검색 연산자 문자(`|` `<` `>` `!` 등)는 의도적으로 이스케이프
/// 하지 않는다 -- 최악의 경우 es.exe가 비정상 종료해 `run_es`가 `None`으로
/// 폴백하거나, 의도와 다른 조건으로 0건을 반환할 뿐이다. 어느 쪽이든 호출부
/// (프론트 팔레트)는 클라이언트 fuzzy 검색으로 넘어가고, 그 fuzzy 검색도
/// 이런 연산자를 지원하지 않으므로 동작이 일관된다.
pub fn build_search_args(user_query: &str) -> Option<Vec<String>> {
    let tokens: Vec<String> = user_query
        .split_whitespace()
        .map(|tok| tok.replace('"', ""))
        .filter(|tok| !tok.is_empty())
        .collect();
    if tokens.is_empty() {
        return None;
    }
    let mut args = vec!["file:".to_string()];
    args.extend(tokens.iter().map(|tok| format!("path:{tok}")));
    Some(args)
}

/// root 아래에서 사용자 검색어(팔레트 입력)와 일치하는 파일(절대경로)을
/// es.exe로 찾는다. `build_search_args`가 `None`이면(빈 검색어) 여기서도
/// `None` -- 호출부가 "검색 안 함"과 "es.exe 실패"를 구분하지 않고 동일하게
/// 폴백 처리할 수 있게 한다.
pub fn find_files_matching(canon_root: &Path, user_query: &str) -> Option<Vec<PathBuf>> {
    let args = build_search_args(user_query)?;
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let output = run_es(canon_root, &refs)?;
    Some(parse_paths_under_root(canon_root, &output))
}

/// stdout 원문을 줄 단위로 잘라 canon_root 하위의 절대경로만 남긴다.
fn parse_paths_under_root(canon_root: &Path, output: &[u8]) -> Vec<PathBuf> {
    decode_es_stdout(output)
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.starts_with(canon_root))
        .collect()
}

/// bytes를 지정 Windows 코드페이지로 디코딩한다. 실패(미지원 코드페이지 등)
/// 시 `None`.
///
/// dwFlags=0 -- 잘못된 바이트는 대체문자로 관대하게 처리한다
/// (`MB_ERR_INVALID_CHARS`를 쓰지 않음).
#[cfg(windows)]
fn decode_with_codepage(codepage: u32, bytes: &[u8]) -> Option<String> {
    use windows_sys::Win32::Globalization::MultiByteToWideChar;

    if bytes.is_empty() {
        return Some(String::new()); // MultiByteToWideChar는 길이 0 입력을 에러로 취급.
    }
    let len = unsafe {
        MultiByteToWideChar(
            codepage,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            std::ptr::null_mut(),
            0,
        )
    };
    if len <= 0 {
        return None;
    }
    let mut wide = vec![0u16; len as usize];
    let written = unsafe {
        MultiByteToWideChar(
            codepage,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            wide.as_mut_ptr(),
            len,
        )
    };
    if written <= 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&wide[..written as usize]))
}

/// es.exe stdout을 시스템 ANSI 코드페이지(CP_ACP)로 디코딩한다. es
/// v1.1.0.37은 stdout을 UTF-8이 아닌 시스템 ANSI 코드페이지(한글 Windows면
/// CP949)로 출력하므로, `from_utf8_lossy`를 쓰면 한글 등 비ASCII 경로가
/// 깨져 `starts_with(canon_root)` 매칭에서 탈락한다. 디코딩 자체가 실패하면
/// (있을 수 없는 코드페이지 등) UTF-8 lossy로 안전하게 폴백한다.
#[cfg(windows)]
fn decode_es_stdout(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::CP_ACP;

    decode_with_codepage(CP_ACP, bytes).unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(not(windows))]
fn decode_es_stdout(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 비-Windows에서는 es.exe가 존재할 수 없으므로 항상 `None`.
#[cfg(not(windows))]
fn run_es(_canon_root: &Path, _es_args: &[&str]) -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
fn run_es(canon_root: &Path, es_args: &[&str]) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::{Duration, Instant};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let root_str = canon_root.to_str()?;

    let mut cmd = Command::new("es.exe");
    cmd.arg("-path")
        .arg(root_str)
        .args(es_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(_) => return None, // es.exe 부재 등 -- 조용한 폴백.
    };
    let mut stdout = match child.stdout.take() {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    };

    let overflowed = Arc::new(AtomicBool::new(false));
    let overflowed_reader = overflowed.clone();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        loop {
            match stdout.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if buf.len() + n > MAX_OUTPUT_BYTES {
                        overflowed_reader.store(true, Ordering::Relaxed);
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(buf);
    });

    let deadline = Instant::now() + Duration::from_secs(TIMEOUT_SECS);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {
                if Instant::now() >= deadline || overflowed.load(Ordering::Relaxed) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };

    let buf = rx.recv().unwrap_or_default();
    let _ = reader.join();

    if overflowed.load(Ordering::Relaxed) {
        return None;
    }
    match status {
        Some(s) if s.success() => Some(buf),
        _ => None, // 비정상 종료·타임아웃 -- 조용한 폴백.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// es.exe가 없는 환경(CI/대부분의 개발 머신)에서도 패닉 없이 None을
    /// 돌려주는지 -- 실제 조용한 폴백 계약의 최소 확인.
    #[test]
    fn missing_es_binary_falls_back_to_none_or_some() {
        let dir = tempfile::tempdir().unwrap();
        let canon = std::fs::canonicalize(dir.path()).unwrap();
        // es.exe가 설치돼 있지 않은 한 None. 설치돼 있어도 패닉하지 않고
        // Vec을 돌려주면 계약을 만족한다(둘 다 허용) -- 이 테스트의 핵심은
        // "죽지 않는다".
        let _ = find_markdown_candidates(&canon);
        let _ = find_gitignore_files(&canon);
    }

    #[test]
    fn parse_paths_under_root_rejects_sibling_paths() {
        let dir = tempfile::tempdir().unwrap();
        let canon = std::fs::canonicalize(dir.path()).unwrap();
        let sibling = canon.with_file_name("sibling-not-under-root");
        let inside = canon.join("a.md");
        let raw = format!("{}\n{}\n", sibling.display(), inside.display());
        let parsed = parse_paths_under_root(&canon, raw.as_bytes());
        assert_eq!(parsed, vec![inside]);
    }

    #[test]
    fn parse_paths_under_root_skips_blank_lines() {
        let dir = tempfile::tempdir().unwrap();
        let canon = std::fs::canonicalize(dir.path()).unwrap();
        let inside = canon.join("a.md");
        let raw = format!("\n{}\n\n", inside.display());
        let parsed = parse_paths_under_root(&canon, raw.as_bytes());
        assert_eq!(parsed, vec![inside]);
    }

    #[test]
    fn build_search_args_wraps_single_token() {
        assert_eq!(
            build_search_args("workdir"),
            Some(vec!["file:".to_string(), "path:workdir".to_string()])
        );
    }

    #[test]
    fn build_search_args_joins_multiple_tokens() {
        assert_eq!(
            build_search_args("workdir tsx"),
            Some(vec![
                "file:".to_string(),
                "path:workdir".to_string(),
                "path:tsx".to_string()
            ])
        );
    }

    #[test]
    fn build_search_args_strips_quote_characters() {
        // 토큰 안의 `"`는 es.exe 파싱을 깨뜨리므로 제거한다.
        assert_eq!(
            build_search_args(r#"ab"c"#),
            Some(vec!["file:".to_string(), "path:abc".to_string()])
        );
    }

    #[test]
    fn build_search_args_whitespace_only_is_none() {
        assert_eq!(build_search_args("   "), None);
        assert_eq!(build_search_args(""), None);
    }

    /// CP949 인코딩된 "한글" 바이트 -- PowerShell
    /// `[Text.Encoding]::GetEncoding(949).GetBytes("한글")`로 확인:
    /// 199,209,177,219.
    #[test]
    #[cfg(windows)]
    fn decode_with_codepage_decodes_cp949_hangul() {
        assert_eq!(
            decode_with_codepage(949, &[0xC7, 0xD1, 0xB1, 0xDB]),
            Some("한글".to_string())
        );
    }

    #[test]
    #[cfg(windows)]
    fn decode_with_codepage_passes_through_ascii() {
        assert_eq!(
            decode_with_codepage(949, b"C:\\dev\\a.md"),
            Some("C:\\dev\\a.md".to_string())
        );
    }

    #[test]
    #[cfg(windows)]
    fn decode_with_codepage_empty_input_is_empty_string() {
        assert_eq!(decode_with_codepage(949, b""), Some(String::new()));
    }
}
