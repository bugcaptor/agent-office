// src-tauri/src/legacy_cleanup.rs
//
// 제거된 기능이 남긴 앱 데이터 잔여물을 기동 때 지운다(정리 계획 §7.F F10).
// 읽거나 쓰는 코드가 없어진 경로만 이름으로 못박아 지운다 -- 디렉터리를
// 훑어 추측으로 지우지 않는다. 옛 설치본이 다시 만들어도 다음 기동에 또 지운다.
//
// - `<app_data>/talks/`: 동료 대화(C26) 대화 로그
// - `<app_data>/claude-plugin/`: 동료 대화 Claude 스킬 플러그인
// - `<temp>/agent-office/bin/office-talk[.cmd]`: 동료 대화 shim(`office-task` 옆)

use std::path::{Path, PathBuf};

/// 지울 잔여물 경로. `temp_bin`은 shim이 놓이던 `<temp>/agent-office/bin`.
fn targets(app_data: &Path, temp_bin: &Path) -> Vec<PathBuf> {
    let talk_shim = if cfg!(windows) { "office-talk.cmd" } else { "office-talk" };
    vec![
        app_data.join("talks"),
        app_data.join("claude-plugin"),
        temp_bin.join(talk_shim),
    ]
}

/// 잔여물을 지우고 지운 경로를 돌려준다. 없거나 지우기에 실패한 경로는 건너뛴다.
/// 심볼릭 링크는 따라가지 않고 링크만 지운다.
pub fn remove_legacy_artifacts(app_data: &Path, temp_bin: &Path) -> Vec<PathBuf> {
    let mut removed = Vec::new();
    for path in targets(app_data, temp_bin) {
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        let result = if meta.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        match result {
            Ok(()) => removed.push(path),
            Err(error) => eprintln!("legacy cleanup: {} 삭제 실패: {error}", path.display()),
        }
    }
    removed
}

/// 기동 경로를 막지 않게 백그라운드 스레드에서 한 번 돈다.
pub fn spawn(app_data: PathBuf) {
    let temp_bin = std::env::temp_dir().join("agent-office").join("bin");
    std::thread::Builder::new()
        .name("legacy-cleanup".into())
        .spawn(move || {
            for path in remove_legacy_artifacts(&app_data, &temp_bin) {
                eprintln!("legacy cleanup: {} 삭제", path.display());
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("agent-office-legacy-cleanup-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn removes_known_leftovers_and_keeps_everything_else() {
        let root = scratch("known");
        let app_data = root.join("app");
        let bin = root.join("bin");
        let skill = app_data.join("claude-plugin").join("agent-office").join("skills").join("talk");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "x").unwrap();
        std::fs::create_dir_all(app_data.join("talks")).unwrap();
        std::fs::write(app_data.join("talks").join("2026-08-22.jsonl"), "{}").unwrap();
        std::fs::create_dir_all(app_data.join("work-inbox")).unwrap();
        std::fs::write(app_data.join("tts-keys.json"), "{}").unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        let talk_shim = if cfg!(windows) { "office-talk.cmd" } else { "office-talk" };
        let task_shim = if cfg!(windows) { "office-task.cmd" } else { "office-task" };
        std::fs::write(bin.join(talk_shim), "x").unwrap();
        std::fs::write(bin.join(task_shim), "x").unwrap();

        let removed = remove_legacy_artifacts(&app_data, &bin);

        assert_eq!(removed.len(), 3);
        assert!(!app_data.join("talks").exists());
        assert!(!app_data.join("claude-plugin").exists());
        assert!(!bin.join(talk_shim).exists());
        assert!(app_data.join("work-inbox").is_dir());
        assert!(app_data.join("tts-keys.json").is_file());
        assert!(bin.join(task_shim).is_file());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_leftovers_are_a_no_op() {
        let root = scratch("missing");
        assert!(remove_legacy_artifacts(&root.join("app"), &root.join("bin")).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_leftover_is_unlinked_without_touching_its_target() {
        let root = scratch("symlink");
        let app_data = root.join("app");
        let outside = root.join("outside");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), "x").unwrap();
        std::os::unix::fs::symlink(&outside, app_data.join("talks")).unwrap();

        remove_legacy_artifacts(&app_data, &root.join("bin"));

        assert!(std::fs::symlink_metadata(app_data.join("talks")).is_err());
        assert!(outside.join("keep.txt").is_file());
        let _ = std::fs::remove_dir_all(&root);
    }
}
