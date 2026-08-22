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
// 매핑(pi v0.84.2 실측 재확정 — 최초 설계는 docs/pi-support-design.md §9, v0.80.3):
//   before_agent_start  → source=prompt {"prompt": ..., "cwd": ...}
//   tool_execution_start→ source=tool   {"tool_name": ..., "tool_input": {...}}
//   message_end(assistant, 5s 스로틀)
//                       → source=tool   {"assistant": "..."}
//   agent_settled       → source=stop   {"message":"Pi finished a task"}
//   session_shutdown    → source=stop   {"message":"Pi session ended"} (열린 턴일 때만)
//
// v1 대비 바뀐 판정 두 가지(둘 다 v0.84.2 코드/실측 근거):
//
// 1. 완료 신호를 `agent_end` → `agent_settled`로 옮겼다. `agent_end`는 *에이전트
//    루프 한 번*이 끝날 때마다 나오고, pi는 자동 재시도(재시도 가능한 API 오류)·
//    자동 컨텍스트 압축·스트리밍 중 큐잉된 후속 메시지가 있으면 같은 사용자 요청
//    안에서 루프를 다시 돌린다(agent-session.js `_runAgentPrompt`의
//    `while (await this._handlePostAgentRun()) await this.agent.continue()`).
//    즉 `agent_end`로 정산하면 아직 일하는 중에 캐릭터가 idle이 되고 "완료" 알림이
//    먼저 뜬다. `agent_settled`는 그 루프의 `finally`에서 정확히 한 번 발화하며
//    (ESC 중단·오류 종료 포함) "더 이상 자동 후속이 없다"를 뜻한다.
//    `agent_settled`가 없는 pi 버전을 위해 `agent_end` 지연 폴백만 남겨 둔다.
// 2. 도구 하트비트를 `tool_execution_end` → `tool_execution_start`로 옮겼다.
//    end 이벤트에는 `args`가 없어(ToolExecutionEndEvent = toolCallId/toolName/
//    result/isError) 라벨에 "지금 무엇을 하는 중"을 실을 수 없고, start가 더 이른
//    시점이라 오래 걸리는 도구에서도 즉시 working으로 보인다.

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

  // 핸들러 예외가 pi 턴을 깨지 않도록 등록·호출 양쪽을 감싼다.
  const on = (ev: string, fn: (e: any) => void) => {
    try {
      pi.on(ev, (e: any) => {
        try { fn(e); } catch { /* 관찰 실패는 삼킨다 */ }
      });
    } catch { /* 미래 pi에서 이벤트가 사라져도 생존 */ }
  };

  // assistant 메시지 content에서 텍스트 블록만 이어 붙인다(toolCall/thinking 제외).
  const assistantText = (content: any): string => {
    if (typeof content === "string") return content.trim();
    if (!Array.isArray(content)) return "";
    return content
      .filter((b: any) => b && b.type === "text" && typeof b.text === "string")
      .map((b: any) => b.text)
      .join(" ")
      .trim();
  };

  const NARRATION_INTERVAL_MS = 5000; // 내레이션 스로틀(claude 전사 tail과 같은 간격)
  const AGENT_END_FALLBACK_MS = 1500; // agent_settled 없는 pi 대비 폴백 지연

  let runOpen = false;   // 사용자 요청 1건이 진행 중인가
  let sawSettled = false; // 이 프로세스에서 agent_settled를 본 적이 있는가
  let endFallback: any;
  let lastNarrationAt = 0;

  const finishRun = (message: string) => {
    if (endFallback) { clearTimeout(endFallback); endFallback = undefined; }
    if (!runOpen) return; // 이미 정산됨 — 중복 완료 알림 금지
    runOpen = false;
    post("stop", { message });
  };

  on("before_agent_start", (e) => {
    runOpen = true;
    lastNarrationAt = 0;
    post("prompt", { prompt: e?.prompt ?? "", cwd: process.cwd() });
  });

  on("tool_execution_start", (e) => {
    post("tool", { tool_name: e?.toolName ?? "", tool_input: e?.args ?? {} });
  });

  on("message_end", (e) => {
    if (!runOpen) return;
    const message = e?.message;
    if (!message || message.role !== "assistant") return;
    const now = Date.now();
    if (now - lastNarrationAt < NARRATION_INTERVAL_MS) return;
    const text = assistantText(message.content);
    if (!text) return;
    lastNarrationAt = now;
    post("tool", { assistant: text });
  });

  // agent_end는 한 요청 안에서도 여러 번(재시도/압축/큐 소진) 나온다 — 완료 신호가
  // 아니다. agent_settled가 없는 pi 버전에서만 지연 폴백으로 쓴다.
  on("agent_end", () => {
    if (sawSettled || !runOpen) return;
    if (endFallback) clearTimeout(endFallback);
    endFallback = setTimeout(() => {
      endFallback = undefined;
      if (!sawSettled) finishRun("Pi finished a task");
    }, AGENT_END_FALLBACK_MS);
    endFallback?.unref?.();
  });

  on("agent_settled", () => {
    sawSettled = true;
    finishRun("Pi finished a task");
  });

  // 안전망: 열린 턴을 남긴 채 pi가 내려가면(quit/reload/new/resume/fork) 정산한다.
  // 이미 정산됐으면 finishRun이 무시하므로 평범한 종료에서 헛알림이 뜨지 않는다.
  on("session_shutdown", () => finishRun("Pi session ended"));
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
    fn extension_source_subscribes_to_the_confirmed_events() {
        for ev in [
            "before_agent_start",
            "tool_execution_start",
            "message_end",
            "agent_end",
            "agent_settled",
            "session_shutdown",
        ] {
            assert!(PI_EXTENSION_TS.contains(ev), "extension must subscribe to `{ev}`");
        }
    }

    /// 완료 판정 회귀 방지: `agent_settled`가 완료 신호이고 `agent_end`는 폴백
    /// 경로(지연 + sawSettled 가드)로만 쓰여야 한다. `agent_end`가 다시 직접
    /// stop을 쏘면 재시도/압축/큐 소진 중에 캐릭터가 idle로 튄다.
    #[test]
    fn extension_source_settles_on_agent_settled_not_agent_end() {
        assert!(PI_EXTENSION_TS.contains("sawSettled"));
        assert!(PI_EXTENSION_TS.contains("AGENT_END_FALLBACK_MS"));
        assert!(
            PI_EXTENSION_TS.contains("on(\"agent_settled\", () => {"),
            "agent_settled must be the settle path",
        );
        assert!(
            !PI_EXTENSION_TS.contains("on(\"agent_end\", () => post(\"stop\""),
            "agent_end must not post stop directly",
        );
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
    fn extension_source_posts_the_three_downstream_sources_with_label_payloads() {
        assert!(PI_EXTENSION_TS.contains("post(\"prompt\""));
        assert!(PI_EXTENSION_TS.contains("post(\"tool\""));
        assert!(PI_EXTENSION_TS.contains("post(\"stop\""));
        // 라벨 파이프라인이 읽는 body 키(observer::event 의 파서와 짝)
        assert!(PI_EXTENSION_TS.contains("cwd: process.cwd()"));
        assert!(PI_EXTENSION_TS.contains("tool_name:"));
        assert!(PI_EXTENSION_TS.contains("tool_input:"));
        assert!(PI_EXTENSION_TS.contains("assistant: text"));
    }
}
