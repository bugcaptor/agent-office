// src-tauri/src/session_log/gc.rs
//
// 세션 로그 보존 정리. docs/session-log-design.md §4.2 가 정본.
//
//   1) 기간: mtime이 30일보다 오래된 파일 삭제.
//   2) 용량: 남은 총합이 2GB를 넘으면 오래된 것부터 그 아래로 내려갈 때까지 삭제.
//
// shell_export::gc_dir의 best-effort 관례를 따른다 -- 개별 실패·디렉터리 부재는
// 무시하고 넘어간다. 정리 실패가 앱 동작을 막을 이유는 없다.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 기본 보존 기간(30일). 사용자가 "최대 한 달"로 정한 상한이다.
pub const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);
/// 기본 용량 상한(2GiB). 기간 안이라도 이걸 넘으면 오래된 것부터 잘린다.
pub const MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// GC 주기(6시간). 부팅 직후 1회 + 이 간격으로 반복.
pub const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// 최근 이만큼 안에 쓰인 파일은 용량 정리에서 제외한다 -- 지금 기록 중인
/// 세션의 파일을 지워 writer가 허공에 쓰게 만들지 않으려는 보호다.
const ACTIVE_GRACE: Duration = Duration::from_secs(300);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    pub removed_by_age: usize,
    pub removed_by_size: usize,
    pub remaining_bytes: u64,
}

/// 루트 아래의 모든 로그·학습자료를 훑어 보존 정책을 적용한다.
pub fn sweep(root: &Path, max_age: Duration, max_total: u64, now: SystemTime) -> GcReport {
    let mut report = GcReport::default();
    let mut files = collect(root);

    // 1) 기간
    files.retain(|f| {
        let too_old = now
            .duration_since(f.modified)
            .map(|age| age > max_age)
            .unwrap_or(false);
        if too_old && std::fs::remove_file(&f.path).is_ok() {
            report.removed_by_age += 1;
            return false;
        }
        true
    });

    // 2) 용량 -- 오래된 것부터.
    let mut total: u64 = files.iter().map(|f| f.bytes).sum();
    if total > max_total {
        files.sort_by_key(|f| f.modified);
        for f in &files {
            if total <= max_total {
                break;
            }
            let recently_written = now
                .duration_since(f.modified)
                .map(|age| age < ACTIVE_GRACE)
                .unwrap_or(true);
            if recently_written {
                continue; // 기록 중일 수 있는 파일은 남긴다
            }
            if std::fs::remove_file(&f.path).is_ok() {
                total = total.saturating_sub(f.bytes);
                report.removed_by_size += 1;
            }
        }
    }
    report.remaining_bytes = total;

    prune_empty_dirs(root);
    report
}

struct Entry {
    path: PathBuf,
    bytes: u64,
    modified: SystemTime,
}

/// 루트 바로 아래 디렉터리(캐릭터별 + study) 안의 파일들을 모은다. 깊이 1까지만
/// 본다 -- 저장소 레이아웃이 그 이상 깊어지지 않는다.
fn collect(root: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    let Ok(dirs) = std::fs::read_dir(root) else {
        return out;
    };
    for dir in dirs.flatten() {
        let Ok(kind) = dir.file_type() else { continue };
        if !kind.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(dir.path()) else {
            continue;
        };
        for file in files.flatten() {
            let Ok(meta) = file.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let Ok(modified) = meta.modified() else {
                continue;
            };
            out.push(Entry {
                path: file.path(),
                bytes: meta.len(),
                modified,
            });
        }
    }
    out
}

/// 파일이 하나도 남지 않은 캐릭터 디렉터리를 치운다(퇴근한 캐릭터의 껍데기).
fn prune_empty_dirs(root: &Path) {
    let Ok(dirs) = std::fs::read_dir(root) else {
        return;
    };
    for dir in dirs.flatten() {
        if dir.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let empty = std::fs::read_dir(dir.path())
                .map(|mut it| it.next().is_none())
                .unwrap_or(false);
            if empty {
                let _ = std::fs::remove_dir(dir.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn scratch() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "agent-office-session-log-gc-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// mtime을 과거로 밀어 둔 파일 하나.
    fn make(root: &Path, agent: &str, name: &str, bytes: usize, age: Duration) {
        let dir = root.join(agent);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, vec![b'x'; bytes]).unwrap();
        let when = filetime_from(SystemTime::now() - age);
        set_mtime(&path, when);
    }

    #[cfg(unix)]
    fn filetime_from(t: SystemTime) -> SystemTime {
        t
    }
    #[cfg(not(unix))]
    fn filetime_from(t: SystemTime) -> SystemTime {
        t
    }

    /// std만으로 mtime을 바꿀 방법이 없으므로 unix에서는 utimensat을 쓴다.
    /// 다른 OS에서는 mtime 조작이 필요한 테스트를 건너뛴다.
    #[cfg(unix)]
    fn set_mtime(path: &Path, when: SystemTime) {
        use std::os::unix::ffi::OsStrExt;
        let secs = when
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let times = [
            libc::timespec {
                tv_sec: secs,
                tv_nsec: 0,
            },
            libc::timespec {
                tv_sec: secs,
                tv_nsec: 0,
            },
        ];
        let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        unsafe {
            libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), 0);
        }
    }
    #[cfg(not(unix))]
    fn set_mtime(_path: &Path, _when: SystemTime) {}

    #[cfg(unix)]
    #[test]
    fn removes_files_older_than_max_age() {
        let root = scratch();
        make(&root, "a", "old.log", 10, Duration::from_secs(40 * 24 * 3600));
        make(&root, "a", "new.log", 10, Duration::from_secs(3600));
        let report = sweep(&root, MAX_AGE, MAX_TOTAL_BYTES, SystemTime::now());
        assert_eq!(report.removed_by_age, 1);
        assert!(!root.join("a/old.log").exists());
        assert!(root.join("a/new.log").exists());
        fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn evicts_oldest_until_under_size_cap() {
        let root = scratch();
        // 각 1000바이트, 나이가 다른 파일 3개. 상한 2500 -> 가장 오래된 1개 삭제.
        make(&root, "a", "1.log", 1000, Duration::from_secs(3 * 3600));
        make(&root, "a", "2.log", 1000, Duration::from_secs(2 * 3600));
        make(&root, "a", "3.log", 1000, Duration::from_secs(1 * 3600));
        let report = sweep(&root, MAX_AGE, 2500, SystemTime::now());
        assert_eq!(report.removed_by_size, 1);
        assert!(!root.join("a/1.log").exists());
        assert!(root.join("a/3.log").exists());
        assert!(report.remaining_bytes <= 2500);
        fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn recently_written_files_survive_size_eviction() {
        let root = scratch();
        // 방금 쓴 파일뿐이면 상한을 넘어도 지우지 않는다(기록 중일 수 있다).
        make(&root, "a", "1.log", 1000, Duration::from_secs(1));
        make(&root, "a", "2.log", 1000, Duration::from_secs(2));
        let report = sweep(&root, MAX_AGE, 500, SystemTime::now());
        assert_eq!(report.removed_by_size, 0);
        assert!(root.join("a/1.log").exists());
        fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn empty_agent_dirs_are_pruned() {
        let root = scratch();
        make(&root, "gone", "old.log", 10, Duration::from_secs(40 * 24 * 3600));
        sweep(&root, MAX_AGE, MAX_TOTAL_BYTES, SystemTime::now());
        assert!(!root.join("gone").exists());
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn missing_root_is_not_an_error() {
        let root = std::env::temp_dir().join("agent-office-no-such-dir-xyz");
        let report = sweep(&root, MAX_AGE, MAX_TOTAL_BYTES, SystemTime::now());
        assert_eq!(report, GcReport::default());
    }
}
