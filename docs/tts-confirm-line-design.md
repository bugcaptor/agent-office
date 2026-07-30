# 확인 요청 대사 TTS 설계

AI 에이전트가 사용자 확인을 기다릴 때, 그 시스템 알림 문구를 캐릭터 말투의 짧은
대사로 리라이트하고 ElevenLabs로 합성해 **캐릭터 목소리로 읽어준다**.

기본 꺼짐(opt-in). 외부 유료 API를 호출하는 장식 기능이므로, 어느 단계가
실패해도 앱 동작에는 영향이 없어야 한다.

## 1. 발화 대상

`notificationType(source)`(`src/shared/types/notification.ts`)가 `"question"`인
알림 **하나뿐**이다 — 즉 `source === "hook"`. `stop`(작업 완료 = `"done"`)과
`bell`(`"info"`)은 발화하지 않는다. 확인을 기다리는 순간에만 말을 거는 것이
이 기능의 전부이고, 완료 알림까지 읽으면 소음이 된다.

**딩은 생략하지 않는다.** 딩은 즉시 나고 대사는 리라이트+합성 왕복(수 초) 뒤에
오므로 서로 겹치지 않는다. 역할도 다르다 — 딩은 "왔다", 대사는 "무엇을 묻는지".
발화가 실패해도 알림 자체를 놓치지 않는다는 이점이 크다.

## 2. 파이프라인

```
질문 알림 ─▶ [렌더러] 게이트(question · ttsEnabled · !muted)
                │
                └─▶ 직렬 큐(voiceQueue.ts) ── invoke tts_speak ──▶ [백엔드]
                                                                      │
   ┌──────────────────────────────────────────────────────────────────┘
   │ 1) 리라이트: 공급자 체인 → 캐릭터 말투 한 줄(≤120자, 오디오 태그 0~2개)
   │ 2) 보이스: seed(없으면 agentId) sha256 % 정렬된 보이스 목록
   │ 3) 캐시: (voice_id, model_id, 최종 텍스트) 해시 → <app-data>/tts-cache/*.mp3
   │ 4) 합성: eleven_v3 → (불가 시) 태그 제거 + eleven_multilingual_v2
   └─▶ base64 mp3 ──▶ [렌더러] decodeAudioData → 기존 게인/컴프레서 체인
```

## 3. 보안 계약 — 키는 웹뷰를 넘지 않는다

`get_app_settings`는 `AppSettings`를 **통째로** 렌더러에 돌려준다. 그래서 API 키는
설정에 두지 않는다. 키를 설정에 넣으면 그 순간부터 웹뷰 컨텍스트에 평문 키가
상주하고, devtools·크래시 리포트·XSS 표면 전부에 노출된다.

- 키 보관: `<app-data>/tts-keys.json` (unix 0600, temp+rename 원자 쓰기).
  `src-tauri/src/tts/keys.rs`.
- 렌더러가 얻는 것: `TtsStatus` — **존재 여부 bool**과 env 유래 여부, claude CLI
  가용성, 실제 선택될 리라이트 경로뿐.
- 합성은 전부 백엔드에서 끝나고 렌더러는 오디오 바이트만 받는다.
- 저장값이 비면 env 폴백: `ELEVENLABS_API_KEY` / `ANTHROPIC_API_KEY`.

`AppSettings`에는 `ttsEnabled` / `ttsRewriteModel` / `ttsRewriteProvider`만 둔다.
셋 다 `Copy` 가능한 enum·bool이다 — `AppSettings`는 `Copy`이고 커맨드들이
`*settings.read().unwrap()`으로 값 복사를 하므로 `String` 필드를 추가하면 그
패턴이 전부 깨진다. 그래서 모델/공급자는 serde rename으로 **문자열처럼 보이는
enum**이다(`"claude-haiku-4-5"`, `"auto"` …).

## 4. 리라이트 공급자 체인

`tts::resolve_rewrite_route(provider, anthropic_key, claude_cli_available)` — 순수
함수. 설정(사용자 의도) + 지금 쓸 수 있는 자원 → 실제 경로.

| 설정 `ttsRewriteProvider` | 결과 |
| --- | --- |
| `auto`(기본) | 저장 키 → `ANTHROPIC_API_KEY` env → claude CLI → 생략 |
| `api` | Messages API만. 키 없으면 **생략**(CLI로 몰래 넘어가 구독을 쓰지 않는다) |
| `claude-cli` | `claude -p`만. CLI 부재 시 생략 |
| `none` | 원문 문구를 그대로 읽는다 |

`auto`가 API를 먼저 두는 이유: 키가 있으면 가장 빠르고(6초 타임아웃), 사용자
구독 사용량을 건드리지 않는다.

### 4.1 API 경로 (`tts/rewrite.rs`)

`POST https://api.anthropic.com/v1/messages` 원시 HTTP(Rust에는 공식 SDK가 없다).
헤더 `x-api-key` + `anthropic-version: 2023-06-01`.

요청 형태에서 고정한 세 가지:

1. **`temperature`/`top_p`/`top_k`를 보내지 않는다** — `claude-sonnet-5`·
   `claude-opus-5`는 이 파라미터를 받으면 400이다. 모델을 설정에서 고를 수 있으므로
   어떤 선택에서도 안전한 형태로 고정한다.
2. **`thinking`을 보내지 않는다.** 대신 `max_tokens`를 1024로 넉넉히 준다 —
   `claude-opus-5`는 thinking이 기본 ON이고 `max_tokens`가 thinking+본문을 함께
   캡하므로, 300 같은 값이면 사고에 다 먹혀 본문이 빈 채로 잘린다.
3. 응답의 **`text` 블록만** 이어붙인다(thinking 블록은 건너뛴다).
   `stop_reason: "refusal"`은 에러로 처리한다.

### 4.2 claude CLI 경로 (`tts/cli.rs`) — 훅 격리

`claude -p ... --output-format text --max-turns 1 --system-prompt <대사 작가 프롬프트>`.
API 키 없이 구독(OAuth)만으로 리라이트가 되지만 **구독 사용량을 소모한다**.

이 앱은 claude 훅으로 세션을 감시한다. 앱이 띄운 `claude -p`가 그 훅을 발화하면
유령 세션/알림이 생긴다. 격리 근거를 세 겹으로 둔다.

1. **훅은 전역 등록이 아니다.** `observer/claude.rs`는 세션마다
   `<app-data>/…/<sessionId>.settings.json`을 쓰고, PTY 세션 셸에 심은 `claude`
   **래퍼 함수**가 `--settings $AGENT_OFFICE_SETTINGS`를 앞에 붙여 전달한다
   (`AdapterSessionPlan.wrappers`). `~/.claude/settings.json`은 전혀 건드리지
   않는다. 우리는 로그인 셸을 거치지 않고 `claude` 바이너리를 직접 spawn하므로
   그 래퍼 함수 자체가 존재하지 않는다.
2. **`--settings '{"hooks":{}}'`** 인라인 JSON 오버라이드를 명시적으로 전달한다
   (`claude --help`: `--settings <file-or-json>`).
3. **포워더 봉인.** 그래도 훅이 발화한다면 실행되는 것은
   `observer/forwarder.rs`이고, 그것은 `AGENT_OFFICE_SESSION` +
   (`AGENT_OFFICE_HOOK_URL` | `AGENT_OFFICE_APP_DATA`)가 있어야 허브에 닿는다.
   자식 env에서 이 변수들(+`AGENT_OFFICE_SETTINGS`)을 **제거**하므로 포워더는
   즉시 no-op으로 죽는다.

**`--bare`는 쓰지 않는다.** help가 "skip hooks"라고 하지만 같은 문서가
"Anthropic auth는 엄격히 `ANTHROPIC_API_KEY` 또는 apiKeyHelper이고 OAuth/키체인은
절대 읽지 않는다"고 못 박는다. 이 경로의 존재 이유가 "API 키 없이 구독으로"이므로
`--bare`는 기능을 무력화한다.

Windows는 `claude`가 직접 실행 가능한 이미지가 아니라 셸 shim이라 powershell을
거친다(`summarizer/claude.rs`와 같은 관례·같은 UTF-8 인코딩 처리).

## 5. 보이스 결정적 배정 (`tts/voice.rs`)

요구는 "같은 캐릭터는 항상 같은 목소리". `seed`(없으면 `agentId`)의 sha256 앞
8바이트를 **정렬된** 보이스 목록 길이로 모듈로 한다.

정렬이 계약이다 — `GET /v2/voices`의 반환 순서는 보장되지 않으므로, 정렬 없이는
API가 순서만 바꿔도 캐릭터 목소리가 통째로 바뀐다. `seed`를 키로 쓰는 이유는
프로필에 영속되는 값이라 이름을 바꿔도 목소리가 유지된다는 점.

목록은 앱 수명 동안 1회 조회 후 캐시(`v2` → `v1` → 하드코딩 프리메이드 폴백).
목록 권한이 없는 키에서도 동작한다.

## 6. 합성과 모델 폴백 (`tts/synth.rs`)

`POST https://api.elevenlabs.io/v1/text-to-speech/{voice_id}` (헤더 `xi-api-key`),
`output_format=mp3_44100_128`.

- 1차 `eleven_v3` — 오디오 태그(`[nervous]` 등)를 감정 지시로 해석하는 유일한
  모델. 리라이트 프롬프트가 태그를 넣는 이유가 이것이다.
- 폴백 `eleven_multilingual_v2` — v3 불가 계정. 이때는 **반드시 대괄호 태그를
  제거**한다. v2는 태그를 감정이 아니라 글자로 읽어버린다.

폴백 판정은 `SynthError::should_retry_without_v3()` — 403(모델 권한)/422/400
(model_id 검증)만. 401·429·5xx는 폴백해도 소용없으므로 즉시 포기한다.

## 7. 캐시

키 = `sha256(voice_id \0 model_id \0 최종 텍스트)` 앞 16바이트 hex →
`<app-data>/tts-cache/<key>.mp3`. `\0` 구분자가 없으면 `("ab","c")`와
`("a","bc")`가 충돌한다.

v3 시도와 v2 폴백은 **텍스트(태그 유무)와 model_id가 둘 다 달라** 서로의 캐시를
히트하지 않는다 — 안 그러면 태그를 글자로 읽어버린 오디오가 재사용된다.

쓰기는 temp+rename(동시 합성 시 반쯤 쓰인 mp3 노출 방지). 파일 수가 300을 넘으면
mtime 오래된 것부터 지운다.

## 8. 렌더러 재생

- **`decodeAudioData`만 쓴다.** `tauri.conf.json`의 CSP에는 `media-src`가 없으므로
  blob/data URL + `<audio>` 경로는 CSP 변경을 강요한다. base64 → `ArrayBuffer` →
  `ctx.decodeAudioData` → `BufferSource`로 기존 마스터 게인·컴프레서 체인에
  물린다(타이핑 소리와 겹쳐도 과대음량이 억제된다).
- `SoundBackend.playVoice(mp3)`는 **재생이 끝나면** resolve한다. 그것이 직렬 큐의
  타이밍 기준이다. `ended`가 오지 않는 경우(컨텍스트 사망 등)에도 큐가 막히지
  않도록 버퍼 길이 + 1초 백스톱 타이머를 둔다.
- `decodeAudioData`는 전달된 `ArrayBuffer`를 detach하므로 사본(`slice(0)`)을 넘긴다.

### 8.1 직렬 큐 (`sound/voiceQueue.ts`)

1. **한 번에 하나.** 합성과 재생을 한 슬롯으로 묶어 직렬화한다. 합성만 병렬로
   돌리면 재생 순서가 응답 시간에 따라 뒤집히고 동시요청 제한에도 걸린다.
2. **에이전트당 대기 1건, 최신 우선.** 같은 캐릭터가 대기 중 또 물어보면 오래된
   문구를 버린다 — 이미 지나간 확인 요청을 읽어주는 것은 소음이다.
   (`Map`은 삽입 순서를 보존하므로 그 자체가 FIFO이고, 키 재대입은 순서를 유지한
   채 값만 교체한다.)
3. **상한 6건.** 넘으면 오래된 것부터 버린다.
4. 큐에서 꺼내는 시점에 게이트를 **다시** 확인한다 — 합성 왕복 중 사용자가
   무음/TTS OFF로 바꾸면 그 뒤 대기 항목은 발화하지 않는다.

## 9. 실패 정책

| 단계 | 실패 시 |
| --- | --- |
| 리라이트(모든 경로) | 원문 문구를 그대로 읽는다(우아한 강등). `rewriteVia: "none"` |
| 보이스 목록 조회 | 하드코딩 프리메이드 폴백 |
| v3 합성(모델 사유) | 태그 제거 + v2 재시도 |
| 그 외 합성 실패 | 렌더러에 `"{code}: {상세}"` 에러. 큐는 경고만 남기고 다음으로 |
| 캐시 저장 | 무시(최적화일 뿐) |

에러 문자열에 키 값이나 외부 응답 body를 싣지 않는다. 외부 body는 200자로 캡한다.

## 10. IPC

| 커맨드 | 입력 | 출력 |
| --- | --- | --- |
| `tts_speak` | `TtsSpeakRequest`(agentId·agentName·archetype·seed·message) | `TtsSpeakResult`(audioBase64·line·voiceId·modelId·cached·rewritten·rewriteVia) |
| `tts_key_status` | — | `TtsStatus` (키 존재 여부 bool만) |
| `tts_set_keys` | `elevenlabs?`·`anthropic?` (`null`=유지, `""`=삭제) | `TtsStatus` |

`tts_speak`는 설정 `tts_enabled`를 **백엔드에서 최종 게이트**한다. 렌더러도
게이트하지만(불필요한 왕복 제거), 외부 API 비용이 걸린 경로라 백엔드가 권위다.

## 11. 파일 지도

| 경로 | 역할 |
| --- | --- |
| `src-tauri/src/tts/mod.rs` | 오케스트레이션, 공급자 체인 결정, 캐시, 에러 |
| `src-tauri/src/tts/keys.rs` | 0600 키 스토어 + env 폴백 + 마스킹 상태 |
| `src-tauri/src/tts/rewrite.rs` | Anthropic Messages API 경로(순수/HTTP 분리) |
| `src-tauri/src/tts/cli.rs` | `claude -p` 경로 + 훅 격리 |
| `src-tauri/src/tts/voice.rs` | 보이스 목록·결정적 배정 |
| `src-tauri/src/tts/synth.rs` | ElevenLabs 합성·태그 제거·모델 폴백 |
| `src-tauri/src/ipc/commands/tts.rs` | 3개 커맨드 |
| `src/shared/types/tts.ts` | 와이어 타입(frozen contract 슬라이스) |
| `src/renderer/sound/voiceQueue.ts` | 직렬 큐 |
| `src/renderer/sound/backend.ts` | `playVoice` (decodeAudioData) |
| `src/renderer/sound/soundManager.ts` | question 게이팅·큐 주입·`previewVoice` |
| `src/renderer/settings/SettingsDialog.tsx` | `TtsSection` 설정 UI |
