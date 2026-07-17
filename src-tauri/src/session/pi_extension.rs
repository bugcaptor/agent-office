// src-tauri/src/session/pi_extension.rs
//
// Pi(pi.dev) CLI 작업상태 감지용 확장 파일 배포.
// Claude Code는 세션별 `--settings` 훅 파일이 필요하지만(notification::hook_settings),
// Pi는 세션id/포트를 프로세스 env(AGENT_OFFICE_SESSION / AGENT_OFFICE_HOOK_URL)에서
// 읽는 정적 확장 하나로 충분하다 — 따라서 세션별 write/cleanup이 없고
// `zsh_wrapper::write_shim`(session::zsh_wrapper)과 동일하게 정적 파일을 blind
// overwrite한다. `pi()` 셸 래퍼(zsh/bash/PowerShell)가 이 파일 경로를 `-e`로
// 주입하면, 확장이 Pi 라이프사이클 이벤트를 기존 `/hook` 엔드포인트로 POST한다
// (다운스트림 hook_server/hub/turnReducer 무수정 재사용).
//
// 매핑(Phase 0 스파이크로 실측 확정 — docs/pi-support-design.md §9):
//   before_agent_start → source=prompt {"prompt": ...}
//   tool_execution_end → source=tool
//   agent_end          → source=stop   {"message":"Pi finished a task"}
//   session_shutdown   → source=stop   {"message":"Pi session ended"}

use std::io;
use std::path::{Path, PathBuf};

/// pi 프로세스 내에서 jiti로 로드되는 확장 소스(default export 팩토리).
/// pi 패키지에서 타입을 import하지 않는다(버전 드리프트 격리) — ExtensionAPI는
/// 구조적 타이핑(`pi: any`)으로만 쓴다. env(HOOK_URL/SESSION)가 없으면 전부
/// no-op이라 agent-office 밖에서 사용자가 이 확장을 로드해도 무해하다.
const PI_EXTENSION_TS: &str = r#"// agent-office-pi.ts — agent-office가 생성. 편집 금지(부팅 시 덮어씀).
// Pi 라이프사이클 이벤트를 agent-office 로컬 훅 서버로 POST해 작업상태를 알린다.
export default function agentOffice(pi: any) {
  const url = process.env.AGENT_OFFICE_HOOK_URL;
  const session = process.env.AGENT_OFFICE_SESSION;
  if (!url || !session) return; // agent-office 밖: 완전 no-op

  const g = globalThis as any;
  if (g.__AGENT_OFFICE_PI_HOOKED__) return; // -e 중복 지정 방어
  g.__AGENT_OFFICE_PI_HOOKED__ = true;

  // POST 직렬화 큐: prompt→tool 역전으로 백엔드 at 타임스탬프가 뒤집히는 것 방지.
  let chain: Promise<unknown> = Promise.resolve();
  const post = (source: string, body: unknown) => {
    chain = chain.then(() =>
      fetch(`${url}?session=${session}&source=${source}&agent=pi`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body ?? {}),
        signal: AbortSignal.timeout(2000),
      }).catch(() => {}) // 앱이 내려가 있어도 pi는 무사
    );
  };

  const on = (ev: string, fn: (e: any) => void) => {
    try { pi.on(ev, fn); } catch { /* 미래 pi에서 이벤트가 사라져도 생존 */ }
  };
  on("before_agent_start", (e) => post("prompt", { prompt: e?.prompt ?? "" }));
  on("tool_execution_end", () => post("tool", {}));
  on("agent_end", () => post("stop", { message: "Pi finished a task" }));
  on("session_shutdown", () => post("stop", { message: "Pi session ended" }));
}
"#;

const EXTENSION_FILENAME: &str = "agent-office-pi.ts";

/// Writes the static Pi extension into `base` (created if missing), overwriting
/// any existing copy — content is static, so blind overwrite is fine (same
/// pattern as `zsh_wrapper::write_shim`). Returns the extension FILE path (the
/// value injected as env `AGENT_OFFICE_PI_EXT`), not the directory.
pub fn write_extension(base: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(base)?;
    let p = base.join(EXTENSION_FILENAME);
    std::fs::write(&p, PI_EXTENSION_TS)?;
    Ok(p)
}

/// Writes the extension into the process-wide scratch location
/// (`<tmp>/agent-office/pi/agent-office-pi.ts`) and returns its path. Safe to
/// call once per session — every call rewrites the same static file.
pub fn ensure_extension() -> io::Result<PathBuf> {
    let base = std::env::temp_dir().join("agent-office").join("pi");
    write_extension(&base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-pi-ext-test-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn write_extension_creates_the_ts_file_and_returns_its_path() {
        let base = scratch_dir();
        let p = write_extension(&base).expect("write_extension succeeds");

        assert_eq!(p, base.join("agent-office-pi.ts"));
        assert!(p.is_file(), "extension file must exist");
        let contents = std::fs::read_to_string(&p).unwrap();
        assert_eq!(contents, PI_EXTENSION_TS, "file must contain the embedded source verbatim");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn write_extension_is_idempotent_and_overwrites_cleanly() {
        let base = scratch_dir();
        write_extension(&base).unwrap();
        let p = write_extension(&base).expect("2nd write must not error");
        assert!(p.is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn extension_source_subscribes_to_the_four_confirmed_events() {
        for ev in ["before_agent_start", "tool_execution_end", "agent_end", "session_shutdown"] {
            assert!(PI_EXTENSION_TS.contains(ev), "extension must subscribe to `{ev}`");
        }
    }

    #[test]
    fn extension_source_guards_env_and_double_load_and_tags_agent() {
        // env 가드: agent-office 밖에서 no-op
        assert!(PI_EXTENSION_TS.contains("AGENT_OFFICE_HOOK_URL"));
        assert!(PI_EXTENSION_TS.contains("AGENT_OFFICE_SESSION"));
        // -e 중복 지정 방어
        assert!(PI_EXTENSION_TS.contains("__AGENT_OFFICE_PI_HOOKED__"));
        // 후일 CLI 구분용 선제 태깅 (§5)
        assert!(PI_EXTENSION_TS.contains("&agent=pi"));
        // jiti default-export 팩토리 계약 (loader.js가 함수 여부만 확인)
        assert!(PI_EXTENSION_TS.contains("export default function"));
    }

    #[test]
    fn extension_source_posts_the_three_downstream_sources() {
        assert!(PI_EXTENSION_TS.contains("post(\"prompt\""));
        assert!(PI_EXTENSION_TS.contains("post(\"tool\""));
        assert!(PI_EXTENSION_TS.contains("post(\"stop\""));
    }
}
