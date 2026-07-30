# 알림 대사 TTS 설계

AI 에이전트가 사용자 확인을 기다리거나 작업을 마쳤을 때, 그 시스템 알림 문구를
캐릭터 말투의 짧은 대사로 리라이트하고 ElevenLabs로 합성해 **캐릭터 목소리로
읽어준다**.

기본 꺼짐(opt-in). 외부 유료 API를 호출하는 장식 기능이므로, 어느 단계가
실패해도 앱 동작에는 영향이 없어야 한다.

## 1. 발화 대상

`notificationType(source)`(`src/shared/types/notification.ts`) 기준으로 둘이다.

| 알림 | `notificationType` | `SpeakKind` | 발화 |
| --- | --- | --- | --- |
| hook | `question` | `question` | O — 사용자 판단을 청하는 말 |
| stop | `done` | `done` | O — 일을 끝내고 알리는 말 |
| bell | `info` | — | X |

`bell`은 제외한다. 그것은 대개 캐릭터의 말이 아니라 터미널이 낸 신호라 읽어줄
내용이 없다.

`SpeakKind`는 **어조를 가르는 유일한 축**이다. 같은 프롬프트로 둘 다 처리하면
완료 알림이 "이거 해도 될까요?"처럼 들린다(§4.0).

**딩은 생략하지 않는다.** 딩은 즉시 나고 대사는 리라이트+합성 왕복(수 초) 뒤에
오므로 서로 겹치지 않는다. 역할도 다르다 — 딩은 "왔다", 대사는 "무엇을".
발화가 실패해도 알림 자체를 놓치지 않는다는 이점이 크다.

## 2. 파이프라인

```
질문/완료 알림 ─▶ [렌더러] 게이트(question|done · ttsEnabled · !muted)
                │
                └─▶ 직렬 큐(voiceQueue.ts) ── invoke tts_speak ──▶ [백엔드]
                                                                      │
   ┌──────────────────────────────────────────────────────────────────┘
   │ 1) 리라이트: 공급자 체인 → 캐릭터 말투 한 줄(≤120자, 오디오 태그 0~2개)
   │            kind에 따라 시스템 프롬프트가 갈린다(요청 vs 완료 보고)
   │ 2) 보이스: 수동 지정(voiceId) → archetype 라벨 필터 → seed sha256 % 후보
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

## 4. 리라이트

### 4.0 상황별 프롬프트 (`SpeakKind`)

`rewrite::system_prompt(kind)` 하나가 API 경로와 claude CLI 경로 **양쪽**에
쓰인다 — 경로에 따라 어조가 달라지면 사용자에게는 그냥 버그로 보인다.

| kind | 지시 | 오디오 태그 예 |
| --- | --- | --- |
| `question` | "사용자 확인을 기다리며 낸 알림… 판단을 청하는 말" | `[nervous]` `[curious]` `[hesitant]` |
| `done` | "작업을 마치고 낸 알림… 묻지 말고 알려라" | `[cheerful]` `[relieved]` `[proud]` |

user 메시지에도 `상황:` 한 줄을 싣는다. 완료 알림의 원문은 짧기 마련이라
(`"작업이 완료되었습니다"`) 시스템 프롬프트만으로는 모델이 맥락을 놓치고
질문투로 쓰는 일이 있었다.

캐시 키는 **최종 텍스트** 기준이므로 kind 분기가 캐시를 오염시키지 않는다
(어조가 다르면 텍스트가 다르고, 텍스트가 같으면 같은 오디오여도 무방하다).

**각색 금지 규칙(사용자 불만 "너무 뜬금없는 말이 나온다" 대응).** 원인은
둘이었다 — ①원문 훅 문구가 빈약해 모델이 즉흥 창작으로 메움, ②archetype이
말투를 넘어 세계관 소재(마법·전투 등)로 쓰임. 두 프롬프트 모두에 규칙을
못박는다.

- "이것은 각색이 아니라 전달이다. 원문에 없는 사실·소재·사건을 지어내지 마라."
- "archetype은 어미·억양 같은 말투의 결에만 살짝 반영하라. 세계관 소품(마법·
  전투·숲 등)을 소재로 끌어오지 마라."
- `question`에만: "원문이 일반적인 문구뿐이면 꾸미지 말고 담백하게 확인만
  청하라"(`done`은 "원문에 내용이 없으면 담백하게 완료만 알린다"가 이미 있다).

**작업 맥락 주입(§4.2a).** 발화 시점에 그 에이전트가 무슨 작업을 하던 중인지
한 줄을 프롬프트에 실어 "빌드 돌려도 될까요?"처럼 상황 밀착형 대사를 유도한다.
두 프롬프트 모두에 한 줄을 추가한다: "작업 맥락이 주어지면 대사가 그 맥락에
자연스럽게 닿게 하라. 단 맥락 역시 소재의 한계다 — 없는 일을 지어내지 마라"
— 맥락도 각색 금지 규칙의 예외가 아니라는 점을 분명히 한다.

### 4.2a 작업 맥락 (`context`)

`SpeakRequest.context: Option<String>`(와이어: `TtsSpeakRequest.context?`) —
렌더러가 채우는 참고용 필드. `tts::rewrite::build_user_content`가 캐릭터
이름·archetype·상황 다음, 원문 알림 문구 앞에 블록으로 삽입한다:

```
최근 작업 맥락(참고용):
<context>
{…}
</context>

원문 알림 문구:
<notice>
{…}
</notice>
```

`context`가 `None`이거나 trim 후 빈 문자열이면 블록 자체를 생략한다(구버전
호환 — 필드가 없어도 기존 동작과 같다). 길이는 `MAX_CONTEXT_CHARS`(300자,
chars 기준)로 절단한다 — 목표(goal)보다는 문맥이 필요하지만 원문 알림 문구를
압도해선 안 되므로.

렌더러(`sound/soundManager.ts`)는 `voiceQueue.enqueue` 시점에 그 에이전트의
`taskLabels`를 `labelText.deriveTaskLabelLines`(머리 위 라벨·터미널 요약과
같은 파생 규칙)에 통과시켜 `line1`(프로젝트·목표) + `line2`(실황)을 공백으로
이어붙여 싣는다. `previewVoice`(설정 UI의 "시청" 버튼)는 context를 싣지 않는다
— 목적이 "이 캐릭터가 어떤 목소리냐"라 작업 맥락과 무관하다.

캐시 키는 여전히 **최종 텍스트** 기준이라 이 변경으로 별도 손댈 곳이 없다
(맥락이 달라 리라이트 결과 텍스트가 달라지면 자연히 다른 캐시 항목이 된다).

### 4.1 공급자 체인

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

### 4.2 API 경로 (`tts/rewrite.rs`)

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

### 4.3 claude CLI 경로 (`tts/cli.rs`) — 훅 격리

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

## 5. 보이스 캐스팅 (`tts/voice.rs`)

`assign_voice(voices, manual, archetype, key)` — 순수·결정적. 세 층이다.

```
1) 수동 지정(프로필 voiceId)이 목록에 있으면 → 그것
2) 없으면 archetype 선호 라벨로 후보를 좁히고        (filter_voices)
3) 그 후보 안에서 seed sha256 앞 8바이트 % len       (pick_voice)
```

요구는 여전히 "같은 캐릭터는 항상 같은 목소리"다. `seed`를 키로 쓰는 이유는
프로필에 영속되는 값이라 이름을 바꿔도 목소리가 유지된다는 점.

**정렬이 계약이다** — `GET /v2/voices`의 반환 순서는 보장되지 않으므로, 정렬
없이는 API가 순서만 바꿔도 캐릭터 목소리가 통째로 바뀐다. 필터는 입력 순서를
보존하므로 후보 목록도 정렬 상태다.

목록은 앱 수명 동안 1회 조회 후 캐시(`v2` → `v1` → 하드코딩 프리메이드 폴백).
목록 권한이 없는 키에서도 동작한다. `tts_list_voices`(설정 UI)와 `tts_speak`이
**같은 캐시**를 쓰므로 화면에 보이는 이름과 실제 발화 목소리가 어긋나지 않는다.

### 5.1 라벨 파싱

`/v2/voices` 응답의 `labels` 맵을 키 정렬된 `Vec<(String,String)>`으로 담는다
(맵을 쓰지 않는 이유: `VoiceRef`가 `PartialEq`여야 하고 항목이 서너 개뿐이라
선형 탐색이 더 싸다). 키·값 모두 **소문자로 정규화**한다 — 계정/버전에 따라
`"Female"`이 오기도 한다.

하드코딩 폴백 9종에도 성별·연령 라벨을 박아 둔다. 라벨이 없으면 폴백 상황에서
캐스팅이 통째로 무의미해지기 때문이다.

### 5.2 archetype → 선호 라벨

스프라이트 종족 8종(`renderer/office/gen/archetypes.ts`의 `ARCHETYPE_IDS`)과
1:1이다. 어울림은 취향이지만 근거를 붙여 고정한다.

| archetype | gender | age | 근거 |
| --- | --- | --- | --- |
| `human` | — | — | 선호 없음. 대부분의 캐릭터가 human이라 여기서 좁히면 전체 캐스팅 분산만 준다 |
| `elf` | female, neutral | young | 가늘고 맑은 결 |
| `orc` | male | middle_aged, old | 굵고 거친 결 |
| `beastfolk` | — | young | 활달함(성별 무관) |
| `robot` | neutral, male | middle_aged, old | 평평하고 낮게 |
| `android` | neutral, female | young, middle_aged | 사람을 흉내 내되 미묘하게 빈 결 |
| `slime` | female, neutral | young | 말랑하고 높은 결 |
| `ghost` | female, neutral | old | 희미하고 서늘한 결 |

미지/부재/`"auto"`는 선호 없음(= `human`). `"auto"`는 렌더러가 저장 시 확정하므로
백엔드까지 오면 확정 전 값이다.

**완화 규칙**: (성별+연령) → (성별만) → 전체. 계정마다 보유 보이스가 다르므로
늙은 여성 목소리가 하나도 없는 계정에서 ghost가 발화 불가가 되면 안 된다.
라벨이 **없는** 보이스는 어느 축에서도 배제하지 않는다(정보 부재 ≠ 불일치).

### 5.3 수동 지정

프로필 `voiceId`(`AgentProfile`, 부재/빈 값 = 자동). 프로필 다이얼로그의
드롭다운(`ProfileDialog`의 `VoiceField`)에서 고르고 그 자리에서 미리듣기한다.

- 목록은 `tts_list_voices` — `(voiceId, name, 라벨 요약)`만 내려온다.
- `ttsEnabled`가 꺼져 있거나 ElevenLabs 키가 없으면 드롭다운 비활성 + 사유 안내.
- 목록에 **없는** id(계정에서 지운 보이스, 다른 PC에서 가져온 프로필)는
  - 백엔드: 조용히 자동 배정으로 강등(발화 실패보다 다른 목소리가 낫다),
  - UI: 그래도 선택값으로 살려 둔다. select가 조용히 "자동"으로 되돌아가면
    저장 순간 지정이 날아간다.
- 캐릭터 번들(`PortableProfile`)에는 **싣지 않는다** — voiceId는 ElevenLabs
  계정에 종속된 값이라 `cwd`/`shell`과 같은 부류다.

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
2. **에이전트당 대기 1건, 최신 우선.** 같은 캐릭터가 대기 중 또 알림을 내면
   오래된 것을 버린다 — 이미 지나간 확인 요청을 읽어주는 것은 소음이다. 질문
   직후 완료가 오면 완료만 읽는다(그 사이 사용자가 이미 답했다는 뜻이다).
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
| `tts_speak` | `TtsSpeakRequest`(agentId·agentName·archetype·seed·message·kind?·voiceId?·context?) | `TtsSpeakResult`(audioBase64·line·voiceId·modelId·cached·rewritten·rewriteVia) |
| `tts_list_voices` | — | `TtsVoiceOption[]`(voiceId·name·labels 요약) |
| `tts_key_status` | — | `TtsStatus` (키 존재 여부 bool만) |
| `tts_set_keys` | `elevenlabs?`·`anthropic?` (`null`=유지, `""`=삭제) | `TtsStatus` |

`kind`/`voiceId`/`context`는 **선택**이다. 없으면 각각 `question`·자동 배정·
맥락 블록 생략 — 이 필드들이 없던 시절의 요청이 그대로 유효해야 한다
(`context`는 §4.2a).

`tts_speak`는 설정 `tts_enabled`를 **백엔드에서 최종 게이트**한다. 렌더러도
게이트하지만(불필요한 왕복 제거), 외부 API 비용이 걸린 경로라 백엔드가 권위다.
`tts_list_voices`는 게이트하지 **않는다** — 캐시된 GET 한 번이라 비용이 없고,
설정을 켜기 전에도 어떤 목소리가 있는지 보여주는 편이 낫다. 대신 키는 필요하다.

## 11. 파일 지도

| 경로 | 역할 |
| --- | --- |
| `src-tauri/src/tts/mod.rs` | 오케스트레이션, 공급자 체인 결정, 캐시, 에러 |
| `src-tauri/src/tts/keys.rs` | 0600 키 스토어 + env 폴백 + 마스킹 상태 |
| `src-tauri/src/tts/rewrite.rs` | Anthropic Messages API 경로(순수/HTTP 분리) |
| `src-tauri/src/tts/cli.rs` | `claude -p` 경로 + 훅 격리 |
| `src-tauri/src/tts/voice.rs` | 보이스 목록·라벨·archetype 캐스팅·결정적 배정 |
| `src-tauri/src/tts/synth.rs` | ElevenLabs 합성·태그 제거·모델 폴백 |
| `src-tauri/src/ipc/commands/tts.rs` | 4개 커맨드 |
| `src/shared/types/tts.ts` | 와이어 타입(frozen contract 슬라이스) |
| `src/renderer/sound/voiceQueue.ts` | 직렬 큐 |
| `src/renderer/sound/backend.ts` | `playVoice` (decodeAudioData) |
| `src/renderer/sound/soundManager.ts` | 소리 3분할 게이팅·question/done 큐 주입·`previewVoice` |
| `src/renderer/settings/SettingsDialog.tsx` | 사운드 토글 3개 + `TtsSection` |
| `src/renderer/profile/ProfileDialog.tsx` | `VoiceField`(보이스 선택 + 미리듣기) |
| `src-tauri/src/persistence/settings_store.rs` | 사운드 3분할 설정 + `migrate_sound_keys` |

## 12. 소리 설정 3분할

`soundEnabled` 하나가 타건음·딩·세션 효과음을 함께 잡고 있었다. TTS가 붙으면서
"타건 소리는 시끄러워서 껐지만 알림은 듣고 싶다"가 성립하지 않는 것이 문제가
됐다. 셋으로 쪼갠다.

| 설정 | 담당 |
| --- | --- |
| `typingSoundEnabled` | 키보드 타건음 |
| `notifySoundEnabled` | 알림 딩 + 세션 시작/종료 효과음 |
| `ttsEnabled` | 대사 발화 |

- 셋은 **서로를 보지 않는다.** 각자 독립 스위치다.
- `muted`(런타임 무음 모드)는 셋 **모두의 상위 마스터**다. 이전에는 딩만
  존중했지만, "지금 아무 소리도 내지 마라"는 의사에 타건음이 예외일 이유가 없다.
- `soundVolume`은 공통이다(마스터 게인 하나를 셋이 공유한다).

### 12.1 마이그레이션

`SettingsStore::load`가 JSON을 `Value`로 한 번 받아 `migrate_sound_keys`를
적용한 뒤 구조체로 파싱한다. serde의 필드 기본값은 **다른 필드 값을 볼 수
없으므로** 파생 매크로만으로는 표현할 수 없다.

규칙: **새 키가 없을 때만** 옛 `soundEnabled` 값으로 둘 다 채운다.

| 파일 상태 | 결과 |
| --- | --- |
| `soundEnabled: false`만 있음 | typing=false, notify=false |
| `soundEnabled: true`만 있음 | typing=true, notify=true |
| `soundEnabled` + 새 키 일부 | 있는 새 키는 보존, 없는 것만 옛 값으로 채움 |
| 셋 다 없음 | serde 기본값(둘 다 켜짐) |

옛 값이 **꺼짐**이었는데 업데이트 후 타건음이 되살아나는 것이 이 마이그레이션이
막아야 할 유일한 사고다.

`soundEnabled`는 저장 시 더 이상 쓰이지 않으므로 다음 저장에서 파일에서 사라진다
(`claudeCliEnabled` → `summarizerEnabled` 때와 같은 관례). 그래서 CLI 제어
(`ctl settings set`)의 병합 패치에서도 옛 키는 무시된다 — 마이그레이션은 로드
시점 한 번뿐이다.

### 12.2 무음 모드 혼동 방지

TTS 미리듣기는 **무음 모드에서도 울린다** — 방금 누른 버튼이 침묵하면 고장으로
보인다. 대신 미리듣기 버튼 옆에 "무음 모드가 켜져 있어 실제 알림은 발화되지
않습니다"를 표시한다(설정 다이얼로그·프로필 다이얼로그 양쪽). 무음인 줄 모르고
"왜 발화가 안 되지"로 헤매는 사고가 실제로 있었다.
