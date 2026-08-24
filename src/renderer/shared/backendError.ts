// src/renderer/shared/backendError.ts
//
// 백엔드(Rust) 에러 문자열 → 사용자 문구.
//
// **방침: 백엔드는 코드를 내려보내고 번역은 프런트에서 한다.** Rust에 번역
// 카탈로그를 두지 않기로 한 것(`src-tauri/src/i18n.rs` 머리말)과 같은 결정이다.
// 커맨드는 `"{code}"` 또는 `"{code}: {기술적 상세}"`로 실패하고, 여기서 코드를
// 카탈로그 키로 바꾼다.
//
// **기술적 상세는 번역하지 않고 그대로 붙인다.** 경로·OS 오류 문자열·포트
// 번호처럼 사용자가 개발자에게 그대로 전달할 원문이라, 옮기면 오히려 쓸모가
// 준다.
//
// 매핑에 없는 코드는 **원문을 그대로 보여준다** — 상류 오류(CLI stderr, HTTP
// 상태 등)는 종류가 열려 있어 전부 열거할 수 없다. `SUMMARY_TEST_ERROR_KEY`가
// 먼저 세운 관례를 일반화한 것이다.
import { t } from "@renderer/i18n";

/**
 * 이 앱의 Rust 커맨드가 내는 안정적 에러 코드 전부. **유니언이라 코드가 늘면
 * `BACKEND_ERROR_KEY`에 문구를 넣지 않는 한 tsc가 잡는다**(Phase 3c의
 * `CharacterBundleError`와 같은 장치).
 *
 * 여기 없는 문자열도 정상이다(위 머리말 참고) — 이 목록은 "번역해 줄 코드"이지
 * "백엔드가 낼 수 있는 전부"가 아니다.
 */
export type BackendErrorCode =
  // 경로 봉쇄 — markdown.rs / session_log 커맨드 공통
  | "path-outside-root"
  | "root-not-found"
  | "root-not-a-directory"
  | "file-not-found"
  | "not-a-file"
  | "not-utf8"
  | "file-too-large"
  | "stat-failed"
  | "read-failed"
  | "write-failed"
  | "no-parent-dir"
  // 세션 로그 · 학습자료(session_log/study.rs). `empty-log`·`summarizer-disabled`는
  // 화면마다 안내 문구가 달라(설정 유도 vs 목록 안내) 여기 두지 않고 각 호출부가
  // `overrides`로 다룬다.
  | "log-read-failed"
  | "study-dir-failed"
  | "study-write-failed"
  // TTS(tts/mod.rs TtsError::code, ipc/commands/tts.rs)
  | "tts_disabled"
  | "missing_elevenlabs_key"
  | "empty_message"
  | "no_voice"
  | "key-save-failed"
  // tailscale serve(ipc/commands/tailscale.rs)
  | "web-remote-down"
  | "tailscale-cli-not-found"
  | "serve-port-conflict"
  | "tailscale-spawn-failed"
  | "tailscale-cli-error"
  // 요약기·CLI 공통
  | "timeout";

/** 코드 → `common` 네임스페이스 카탈로그 키. */
export const BACKEND_ERROR_KEY: Record<BackendErrorCode, string> = {
  "path-outside-root": "common:errors.pathOutsideRoot",
  "root-not-found": "common:errors.rootNotFound",
  "root-not-a-directory": "common:errors.rootNotADirectory",
  "file-not-found": "common:errors.fileNotFound",
  "not-a-file": "common:errors.notAFile",
  "not-utf8": "common:errors.notUtf8",
  "file-too-large": "common:errors.fileTooLarge",
  "stat-failed": "common:errors.statFailed",
  "read-failed": "common:errors.readFailed",
  "write-failed": "common:errors.writeFailed",
  "no-parent-dir": "common:errors.noParentDir",
  "log-read-failed": "common:errors.logReadFailed",
  "study-dir-failed": "common:errors.studySaveFailed",
  "study-write-failed": "common:errors.studySaveFailed",
  tts_disabled: "common:errors.ttsDisabled",
  missing_elevenlabs_key: "common:errors.ttsKeyMissing",
  empty_message: "common:errors.ttsEmptyMessage",
  no_voice: "common:errors.ttsNoVoice",
  "key-save-failed": "common:errors.keySaveFailed",
  "web-remote-down": "common:errors.webRemoteDown",
  "tailscale-cli-not-found": "common:errors.tailscaleCliNotFound",
  "serve-port-conflict": "common:errors.tailscaleServeConflict",
  "tailscale-spawn-failed": "common:errors.tailscaleSpawnFailed",
  "tailscale-cli-error": "common:errors.tailscaleCliError",
  timeout: "common:errors.timeout",
};

/** `"{code}: {detail}"`을 가른 결과. 콜론이 없으면 `detail`은 빈 문자열이다. */
export interface BackendErrorParts {
  code: string;
  detail: string;
}

/**
 * reject 값(문자열 · Error · 그 밖)을 `{code, detail}`로 가른다.
 *
 * 첫 `:`만 본다 — 상세에 콜론이 흔하고(`C:\…`, `HTTP 401: …`), 코드에는 없다.
 */
export function parseBackendError(err: unknown): BackendErrorParts {
  const raw = err instanceof Error ? err.message : String(err);
  const at = raw.indexOf(":");
  if (at < 0) return { code: raw.trim(), detail: "" };
  return { code: raw.slice(0, at).trim(), detail: raw.slice(at + 1).trim() };
}

/**
 * 사용자에게 보여줄 한 줄. 코드는 카탈로그 문구로, 상세는 원문 그대로 괄호에
 * 붙인다. **호출 시점에** `t()`를 부르므로 언어를 바꾸면 다음 호출부터 새
 * 문구가 나온다(모듈 최상위에서 굳히면 안 된다).
 *
 * `overrides`는 그 화면에서만 다르게 말해야 하는 코드용이다(예: 초상화 생성
 * 패널은 `profile:codex.*`를 쓴다). 없으면 공통 매핑만 본다.
 */
export function backendErrorText(
  err: unknown,
  overrides?: Readonly<Record<string, string>>
): string {
  const { code, detail } = parseBackendError(err);
  const key = overrides?.[code] ?? BACKEND_ERROR_KEY[code as BackendErrorCode];
  if (!key) {
    // 모르는 코드 = 상류 오류. 원문이 가장 정보가 많다.
    return err instanceof Error ? err.message : String(err);
  }
  const message = t(key);
  return detail ? `${message} (${detail})` : message;
}
