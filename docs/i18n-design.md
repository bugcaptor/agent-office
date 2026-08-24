# UI 다국어(i18n) 설계 — 한국어 / 영어

상태: 정본 — 구현 완료 (2026-08-25). 이행은 커밋 6개로 나뉘어 있다: `44b0f56`(인프라·언어 설정) → `aa18f5e`(설정·프로필 화면) → `c4284c3`(터미널·작업 폴더·마크다운) → `dbf428d`(남은 UI + 뷰모델 `TextKey`) → `57d04d3`(AI 프롬프트·언어 규칙 프로필) → `d6b735a`(백엔드 프롬프트 언어 + 에러 코드화).

구현 파일: 카탈로그 `src/shared/i18n/catalog.ts` + `src/shared/i18n/locales/{ko,en}/*.json`(9 네임스페이스 × 2 언어 = 18개). 런타임 글루 `src/renderer/i18n/index.ts`. 언어별 코드 자산 `src/renderer/i18n/{promptProfiles,textRules,wordlists}.ts`. 뷰모델 장치 `src/renderer/shared/textKey.ts`, 에러 매핑 `src/renderer/shared/backendError.ts`. 백엔드 `src-tauri/src/i18n.rs`. 설정 필드는 `AppSettings.language`(`src/shared/types/settings.ts` ↔ `src-tauri/src/persistence/settings_store.rs`).

이 앱은 한국어 전용으로 만들어졌다. 이 문서는 그 전제를 **한국어를 정본으로 둔 채** 걷어낸 방법과, 걷다가 밟은 함정을 남긴다.

## 1. 목표 / 비목표

**목표**

- 데스크톱 renderer의 모든 표시 문자열을 카탈로그로 옮긴다. 하드코딩 한글은 테스트가 막는다.
- **언어 추가 = `locales/`에 폴더 하나 추가**. 코드·타입·설정 드롭다운을 고치지 않는다.
- React 밖(PixiJS 씬, zustand 스토어, 순수 TS 모듈)에서도 같은 `t()`를 쓴다.
- AI에게 보내는 프롬프트도 언어를 따라간다. 단 **ko 프롬프트는 한 글자도 바뀌지 않는다**(이번 작업은 이행이지 튜닝이 아니다).

**비목표**

- 웹 리모트(`src/web`)·VSCode 확장(`vscode-ext/`)의 다국어화 — §8.
- Rust 쪽 번역 카탈로그 — §2.5.
- 에이전트(CLI)에게 나가는 문구 — 사용자 화면이 아니다. §8.
- 영어 화면의 자동 문구 단언. 영어는 눈검증으로 본다(§6 마지막).

## 2. 결정과 근거

### 2.1 라이브러리 — i18next + react-i18next

결정적 제약은 하나다: **React 밖에서도 `t()`를 불러야 한다.** 이 앱에서 표시 문자열을 만드는 곳은 컴포넌트만이 아니다.

- PixiJS 씬 — `OfficeScene.ts`의 사장 책상 팻말(`office:sign.vacation`)은 `Text` 표시객체다.
- zustand 스토어 — `workdirStore`·`markdownStore`·`sessionLogStore`가 상태에 담을 안내/에러 문구를 만든다.
- 순수 TS 모듈 — `speechGenerator`·`diaryGenerator`·`summarizer`·`relativeTime`·`themes.ts`.

i18next는 인스턴스가 곧 번역기라 `import { t }` 한 줄로 어디서나 쓸 수 있고, React 안에서는 `react-i18next`의 `useTranslation()`이 `languageChanged`에 리렌더를 걸어 준다. 같은 카탈로그를 두 세계가 공유한다.

- **lingui를 뺀 이유**: 매크로/추출기가 빌드 파이프라인에 들어온다. 이 저장소는 Vite + vitest만으로 도는 얇은 구성이고 eslint조차 없다(§6). 카탈로그를 손으로 관리해도 파리티 테스트가 지켜 주므로 추출기의 값이 크지 않다.
- **자체 구현을 뺀 이유**: 복수형(`_one`/`_other`)·보간·네임스페이스·폴백 체인을 다시 만드는 일이다. 특히 폴백은 "키가 없을 때 무엇이 화면에 나가는가"를 정하는 안전장치라 직접 짜면 반드시 새는 경로가 생긴다.

인스턴스는 전역 싱글턴이 아니라 `i18next.createInstance()`다 — 테스트가 서로의 언어 상태를 밟지 않도록 재초기화 지점(`initI18nForTest`)을 명시적으로 두려는 것이다.

### 2.2 카탈로그 배치 — `shared` + `import.meta.glob` eager

```
src/shared/i18n/locales/<언어>/<네임스페이스>.json
```

`catalog.ts`가 `import.meta.glob("./locales/*/*.json", { eager: true })` 결과를 경로 정규식으로 갈라 i18next `resources` 모양으로 조립한다. 여기서 나오는 성질이 이 설계의 핵심이다.

- `SUPPORTED_LANGUAGES` = `Object.keys(resources)`(정본 ko가 항상 맨 앞).
- 설정 드롭다운 항목 = `availableLanguages()` = 같은 목록 + 각 언어 `common.json`의 `_meta.label`.
- 폴백 해석(`matchLanguage`)·파리티 테스트 대상도 전부 같은 목록.

즉 **어디에도 언어 목록을 손으로 적어 둔 곳이 없다.** 폴더를 만들면 목록이 늘고, 지우면 준다.

`eager: true`인 이유는 §2.6(동기 초기화)이다. 리소스가 정적 import라 로딩 상태가 없으므로 `react.useSuspense: false`로 둘 수 있다.

**renderer가 아니라 shared에 둔 이유**: 웹 리모트(`src/web`)는 별도 번들이라 `@renderer/*`를 import할 수 없다. 지금은 범위 밖이지만(§8), 나중에 다국어화할 때 자기 글루(i18next 인스턴스)만 만들면 **같은 JSON을 그대로 재사용**할 수 있게 남겨 둔 자리다. shared에는 renderer 의존이 없어야 하므로 `catalog.ts`에는 i18next import조차 없다 — 순수한 리소스 조립기다.

### 2.3 `AppSettings.language`는 유니언이 아니라 자유 문자열

```ts
language: string;   // "system" | "ko" | "en" | …
```

`"system" | "ko" | "en"` 유니언으로 두면 언어를 하나 추가할 때마다 **TS 타입·Rust 미러·컨트랙트 픽스처**를 함께 고쳐야 한다. 그건 §2.2에서 세운 "폴더만 추가하면 된다"를 정면으로 깨는 계약이다. 그래서 양쪽 다 열어 뒀다:

- TS `src/shared/types/settings.ts` → `language: string`
- Rust `src-tauri/src/persistence/settings_store.rs` → `#[serde(default = "default_language")] pub language: String`

기존 설정 파일에 키가 없으면 `serde(default)`로 `"system"`이 되므로 마이그레이션이 필요 없다.

**`"system"` 해석 규칙**: 프런트는 `resolveLanguage()` → `matchLanguage(navigator.language)`, 백엔드는 `i18n::resolve_lang()` → `sys_locale::get_locale()`. 매칭은 양쪽 다 같은 순서다.

1. 정확 일치 (`ko` → ko)
2. 프리픽스 일치 (`en-GB` → en, `ko-KR` → ko)
3. 폴백 (`en`)

백엔드는 POSIX 표기(`ko_KR.UTF-8`)도 받으므로 `-`·`_`·`.` 셋으로 자른다. 카탈로그에 없는 값(`"ja"`, 손상된 설정 파일의 쓰레기 값)은 **판단하지 않고 조용히 폴백한다** — 자유 문자열 계약의 뒷면이다.

### 2.4 정본은 ko (`fallbackLng: "ko"`)

`SOURCE_LANGUAGE = "ko"`. 앱의 모든 문구가 먼저 한국어로 쓰였고, 카탈로그 ko는 그 문구를 **옮겨 담기만** 한 것이라 사실상 원본이다. en은 ko에서 파생된다.

`fallbackLng`를 ko로 두면 en 카탈로그에 키가 빠져도 사용자에게는 한국어가 보인다 — 키 문자열(`workdir:palette.retry`)이 화면에 새는 것보다 낫다. 누락 자체는 `catalogParity` 테스트가 커밋 전에 잡으므로 이 폴백이 발동할 일은 실질적으로 없다(개발 중 새 키를 ko에만 넣은 순간뿐이고, 그때는 `missingKeyHandler`가 콘솔에 찍는다).

`FALLBACK_LANGUAGE = "en"`은 성격이 다르다. 이쪽은 "**카탈로그가 없는 언어**로 떨어졌을 때 무엇을 보여줄까"이고(`matchLanguage`의 마지막 단계), 프롬프트 프로필·`textRules`·`wordlists`의 폴백도 같은 값을 쓴다. 정본이 ko인데 폴백이 en인 게 어색해 보이지만, 두 값이 답하는 질문이 다르다: `fallbackLng`는 "키가 없을 때", `FALLBACK_LANGUAGE`는 "언어 자체가 없을 때".

### 2.5 Rust에 카탈로그를 만들지 않는다

`src-tauri/src/i18n.rs`는 `Lang { Ko, En }` 열거형과 해석 함수뿐이다. **번역 카탈로그가 아니다.**

언어를 타는 Rust 문자열은 손에 꼽는다.

- TTS 리라이트 프롬프트 (`tts/rewrite.rs`, `tts/openrouter.rs`)
- 학습자료 시스템 프롬프트 (`session_log/study.rs`의 `study_system_prompt`)
- 동료 대화 주입 템플릿 (`talk/mod.rs`)
- 요약기 중략 마커 (`summarizer/mod.rs`의 `truncation_marker`)
- 훅 알림 기본 문구 (`notification/hub.rs`의 `attention_fallback`/`stop_fallback`)

이걸 위해 카탈로그 인프라(로더·파리티·키 네이밍)를 세우는 것은 과설계다. 각 모듈이 언어별 `&'static str` 상수를 **자기 옆에** 두고 `Lang`으로만 갈라 쓴다.

**사용자 화면에 나가는 에러도 여기서 번역하지 않는다.** 백엔드는 코드를 내려보내고 번역은 프런트가 한다:

```
"path-outside-root"
"tailscale-cli-error: exit status 1"
```

이건 새 규약이 아니라 **기존 `"{code}: {detail}"` 관례를 일반화한 것**이다. TTS는 이미 `TtsError::code()`로 `tts_disabled`·`no_voice` 같은 코드를 내고 있었고, 요약기 테스트도 코드 문자열을 쓰고 있었다. `d6b735a`는 markdown·session_log·tailscale·media·persistence 커맨드의 남은 한국어 문구를 같은 모양으로 바꾼 것이다. 프런트 매핑은 `src/renderer/shared/backendError.ts`(§3.5).

이 방침의 대가 하나: 백엔드가 UI 언어를 알아야 하는 지점(프롬프트)에서는 설정을 읽어야 한다. `lib.rs`가 부팅·설정 저장 시 `hub.set_lang(i18n::ui_lang(&settings))`로 밀어 넣고, TTS는 `RewriteConfig::from_settings`가 한 번 해석해 들고 다닌다.

### 2.6 첫 페인트 플래시 방지 — localStorage 캐시로 **동기** 초기화

진짜 설정(`AppSettings.language`)은 Tauri invoke라 **비동기**다. 첫 render 때는 아직 없다. 그대로 두면 한국어로 그렸다가 영어로 갈아엎히는 플래시가 난다.

그래서 3단이다.

1. `main.tsx`가 `App`보다 **먼저** `import "./i18n"` — 모듈 로드 시점에 i18next가 동기로 init된다(리소스가 정적 import라 가능하다).
2. `lng`는 `readCachedLanguage() ?? resolveLanguage("system")`. 캐시 키는 `agent-office.lang`(`LANGUAGE_STORAGE_KEY`).
3. 부팅이 설정을 받으면 `bootstrap.ts`가 `applyLanguageSetting(settings.language)`로 교정한다. 보통은 캐시와 같아서 no-op이고, 다르면(첫 실행·설정 파일을 밖에서 고침) 여기서 한 번 전환되며 캐시도 갱신돼 **다음 부팅부터는 플래시가 없다.**

테마(`loadStoredThemeId`)·터미널 뷰 모드(`terminalViewMode.ts`)가 이미 쓰는 관례를 그대로 따른 것이다. `applyLanguageSetting`은 이미 그 언어면 아무것도 하지 않는다 — 흔한 no-op 경로에서 불필요한 `languageChanged` 방출을 막는다(§7의 Pixi 리스너·`useTranslation` 리렌더가 전부 여기 매달려 있다).

## 3. 구조 지도

### 3.1 파일과 역할

| 파일 | 역할 |
|---|---|
| `src/shared/i18n/catalog.ts` | glob 로더. `resources`·`SUPPORTED_LANGUAGES`·`SOURCE_LANGUAGE`·`FALLBACK_LANGUAGE`·`languageLabel`·`matchLanguage`. renderer 비의존(순수 조립기) |
| `src/shared/i18n/locales/{ko,en}/*.json` | 카탈로그 본체. 9 네임스페이스 |
| `src/renderer/i18n/index.ts` | i18next 인스턴스 + 동기 init. `t`·`i18n`·`currentLocale`·`resolveLanguage`·`applyLanguageSetting`·`availableLanguages`·`LANGUAGE_SYSTEM`·`initI18nForTest` |
| `src/renderer/i18n/promptProfiles.ts` | AI 프롬프트 + 딸린 상수 묶음(언어별) |
| `src/renderer/i18n/textRules.ts` | 사용자 입력 판정 규칙(언어별) |
| `src/renderer/i18n/wordlists.ts` | 랜덤 프로필 초안용 낱말 데이터(언어별) |
| `src/renderer/shared/textKey.ts` | 순수 뷰모델이 돌려주는 `{key, params}` 설명자 + `renderText` |
| `src/renderer/shared/backendError.ts` | 백엔드 에러 코드 → 카탈로그 키 |
| `src-tauri/src/i18n.rs` | `Lang`·`ui_lang`·`resolve_lang`. `sys-locale` 크레이트로 OS 로케일 |

`src/renderer/i18n/` 아래 셋(프롬프트·규칙·낱말)은 **한글 스캐너의 제외 경로**다(§6). 여기 한글 리터럴이 있는 게 정상인 유일한 곳이다.

### 3.2 네임스페이스 9개

`defaultNS: "common"`. 나머지는 `ns:키` 로 지정한다.

| 네임스페이스 | 담당 | 상위 키 |
|---|---|---|
| `common` | 앱 셸 공통 + 백엔드 에러 문구 | `_meta`, `time`, `topBar`, `bottomBar`, `ticker`, `notification`, `errors` |
| `app` | 정보 다이얼로그·확인 대화상자·셸 출력 | `about`, `confirm`, `shell` |
| `settings` | 설정 다이얼로그 전체(첫 실행·페어링·모델 픽커 포함) | `language`, `dialog`, `keys`, `general`, `sound`, `tts`, `system`, `control`, `webRemote`, `talk`, `firstRun`, `pairing`, `modelPicker` |
| `profile` | 캐릭터 프로필 편집(초상·스프라이트·미니미·번들 입출력) | `dialog`, `identity`, `appearance`, `keyColor`, `portrait`, `sprite`, `minimi`, `terminal`, `bot`, `voice`, `io`, `codex`, `archetype`, `color`, `editor`, `generate` |
| `office` | PixiJS 씬이 쓰는 이름들 — 풍경·테마·종족·키 컬러·사운드팩·팻말·책상 | `scene`, `theme`, `archetype`, `keyColor`, `soundPack`, `sign`, `desk` |
| `terminal` | 탭 스트립·우클릭 메뉴·봇 오버레이·호스트·팔레트 | `tab`, `menu`, `bot`, `host`, `palette` |
| `workdir` | 작업 폴더 팔레트·상세·diff·커밋 로그·git 상태·마크다운 뷰어 | `palette`, `detail`, `diff`, `repoLog`, `status`, `markdown` |
| `activity` | 사용량·타임라인·활동 분석·동료 대화 로그·세션 로그 | `usage`, `timeline`, `analytics`, `talk`, `sessionLog` |
| `journal` | 일기·수상(이 달의 우수사원)·포스트잇 메모 | `diary`, `awards`, `memo` |

`office`가 별도인 이유는 **씬 쪽 소비자가 React 밖**이라서다. `themes.ts`·`packs.ts`·씬 레지스트리가 모듈 최상위 상수에 `labelKey`만 담고(§4), 그 키가 전부 이 네임스페이스에 모인다.

`common.json`의 `_meta`는 번역문이 아니라 카탈로그 메타데이터라 `_` 접두사로 구분하고, 파리티 테스트의 값 비교 대상에서 제외된다.

### 3.3 프롬프트는 카탈로그가 아니다 — `promptProfiles.ts`

라벨 요약·일기·수상 소감의 시스템 프롬프트는 카탈로그에 없다. 여기 있는 것은 **문자열 하나가 아니라 묶음**이기 때문이다. 예를 들어 `SpeechPromptProfile` 하나에 들어 있는 것:

- `systemPrompt` — 프롬프트 본문
- `headers` — 입력 블록 머리말 (`[성격]` / `[Personality]`)
- `noneText` / `noDiaryText` — 자리 표시 sentinel
- `promptBudgetChars`, `personalityMaxChars`, `speechMaxChars`, `speechMaxSentences` — 숫자
- `excerptLimits { maxEntries, perEntryChars, totalChars }`, `excerptMinBodyChars`
- `formatAwardInfo(v)` — 수치를 그 언어의 문장으로 조립하는 **함수**

`LabelPromptProfile`에는 여기에 더해 `metaMarkers`(모델 거부 문구 목록)와 `linePrefixPattern`(머리말 제거 **정규식**)이 있다. 카탈로그는 문자열만 담고 키 파리티를 요구하므로 맞지 않는다. 게다가 프롬프트 본문에는 보간이 아닌 중괄호·따옴표·화살표가 섞여 있어 i18next 보간 규칙과 충돌한다.

가장 중요한 이유는 성격이다. **이 값들은 UI 문구가 아니라 모델 입력이다.** "번역이 정확한가"가 아니라 "그 언어에서 원하는 출력이 나오는가"로 평가해야 하고, 그래서 en은 ko의 번역이 아니다. 실제로 다르게 설계한 곳들:

- 라벨 길이 제약을 **글자 수 → 단어 수**로 바꿨다. 한글 12자가 픽셀 라벨의 예산인데 그건 영문 24자쯤이고, "24 characters"는 모델이 지키기 어려운 지시다(공백 포함 여부부터 흔들린다). `4 words`/`6 words`가 같은 폭을 더 안정적으로 맞춘다.
- 프롬프트 안의 예시를 영어 예시로 갈아 끼웠다. 예시는 출력 **형식**을 가르치는 장치인데, 한국어 예시를 남기면 "한국어로 답하라"는 신호로 읽힌다.
- `metaMarkers`는 낱말 대응이 아니다. ko는 `["인코딩", "죄송", "할 수 없"]`, en은 `sorry`/`i cannot`/`as an ai` 같은 거부 정형구다. `cannot`/`unable`을 홀로 두면 "cannot reproduce" 같은 **정상 라벨**을 잡아먹는다.
- 일기는 분량 제약이 문장 수(3~8)라 언어를 타지 않아 ko와 같은 값을 유지했지만, `bodyMinChars`는 4 → 8로 키웠다(공백 뺀 한글 4자 ≈ 영문 8자).

`wordlists.ts`도 같은 판단이다. 캐릭터 이름/역할/성격 수식어는 UI 라벨이 아니라 **그 언어권에서 그럴듯한 자료**다. 키 대 키로 대응하지 않고("야근요정"의 번역이 있을 리 없다) 항목 수도 언어마다 달라도 된다.

**프로필의 폴백은 카탈로그와 다르다.** 프로필이 없는 언어는 en 프로필로 돈다: 정확 일치 → 프리픽스 일치 → `FALLBACK_LANGUAGE` → `SOURCE_LANGUAGE`. 카탈로그에 언어를 추가하고 프로필을 만들지 않으면 UI는 그 언어인데 요약·일기·소감만 영어로 나오는데, **이건 버그가 아니라 정상 동작이다** — 기계 번역한 프롬프트로 출력 품질을 망치느니 영어가 낫다.

### 3.4 입력 판정은 또 다른 자리 — `textRules.ts`

화면 문구도(카탈로그) 프롬프트도(프로필) 아닌 셋째 부류: **사용자가 친 텍스트를 판정하는 언어별 상수.**

이 모듈이 따로 있는 이유는 한 문장으로 정리된다. **한국어는 요청이 어미에 실리고 영어는 문두에 실려서, 규칙의 닻이 통째로 뒤집힌다.**

```ts
// ko — 어미에 $ 를 건다
/줘$/, /주세요$/, /(하고\s?싶|좋겠)[가-힣]*$/, /[가-힣]해$/, …

// en — 문두에 ^ 를 건다
/^(please|pls)\b/i, /^(can|could|would|will) you\b/i, /^let'?s\b/i,
/^(?:(?:then|next|also|now|first|finally|after that)[,\s]+)?(add|build|…|write)\b/i,
```

en 쪽 마지막 패턴의 이음말 스킵(`then`/`next`/`also`…)에는 ko 대응물이 아예 없다 — 어미 판정이라 문두에 뭐가 오든 상관없기 때문이다. 그래서 "어미 목록" 같은 이름을 쓰지 않고 `requestPatterns`(판정 규칙 묶음)라고 부른다. 각 정규식이 **자기 닻을 스스로 갖는다**는 게 이 필드의 계약이다.

나머지 필드들도 같은 성격이다.

- `backchannelStart` — 맞장구 판정. **토큰 경계를 요구한다**: 한국어는 "네"가 "네트워크"의 앞머리라 경계 없이 접두 일치만 보면 "네트워크 설정 고쳐줘"가 맞장구로 오분류된다. 영어도 "ok"는 "okra"의, "sure"는 "surefire"의 앞머리다. 다만 한글 사이에는 `\b`가 생기지 않으므로 ko는 명시적 구두점 lookahead를, en은 `\b`를 쓴다. en에만 `i` 플래그가 붙는다(한글에는 대소문자가 없다).
- `goalFallbackMinChars` — ko 6 / en 12. 한글 6자면 두세 낱말이지만 영문 6자는 낱말 하나도 안 된다.
- `greetingStart` / `greetingMaxChars` — ko 12 / en 24. 같은 비율.

en 목록을 짤 때의 원칙은 ko와 같다: **오탐이 미탐보다 나쁘다.** "good"처럼 명령문의 첫 낱말로도 자연스러운 말은 뺐다("good, now…" vs "good first issue를 …"). 가점을 못 받아도 "마지막 조각" 규칙이 여전히 답을 낸다.

소비처는 `appStore.ts`의 `isMeaningfulGoalFallback`과 `labels/labelText.ts`다.

### 3.5 뷰모델은 문장 대신 키를 돌려준다 — `textKey.ts`

`usage/usageView.ts`·`sessionlog/format.ts` 같은 순수 모듈이 완성된 문장을 돌려주면 두 가지가 깨진다.

1. 언어를 바꿔도 **이미 계산돼 상태에 담긴 문자열**은 안 바뀐다.
2. 단위 테스트가 문구에 묶인다.

그래서 이 모듈들은 `TextKey = { key, params? }`만 돌려주고, 실제 번역은 `t`를 쥔 렌더 컴포넌트(`usage/UsageDialog.tsx`, `analytics/DailyBarChart.tsx`)가 `renderText(text, t)`로 한다.

문장을 만들지 않아도 되면 더 가벼운 방법이 낫다. `timeline/agentStats.ts`는 예전에 라벨을 `` `${id.slice(0,8)}… (퇴사)` ``로 조립했는데, 이제 `label`(순수 값)과 `departed: boolean`을 따로 돌려주고 "(퇴사)" 문구는 렌더 쪽이 `activity:timeline.departed`로 붙인다. **정렬도 이 순수 값 기준**이라 언어에 흔들리지 않는다(§7.2와 같은 이야기). 즉 규칙은 "뷰모델은 `TextKey`를 돌려준다"가 아니라 **"뷰모델은 문구를 만들지 않는다"**이고, `TextKey`는 그중 키 선택까지 순수 함수가 해야 할 때 쓰는 도구다.

**`params` 값에 다시 `TextKey`를 넣을 수 있다.** "마지막 시도 {{ago}}"처럼 문장 안에 또 다른 번역 조각이 들어가고, **그 조각도 순수 함수가 골라야** 하는 자리를 위해서다. `renderText`가 재귀로 푼다.

i18next에도 `$t()` 중첩이 있지만 역할이 다르다: `$t()`는 카탈로그 값 안에 **고정된** 다른 키를 끼워 넣는 것이고, 여기 중첩은 **런타임에 고른** 키를 끼워 넣는다. 후자는 카탈로그 값에 미리 적을 수 없다.

### 3.6 백엔드 에러 코드 → 카탈로그 키 — `backendError.ts`

`parseBackendError(err)`가 reject 값을 `{code, detail}`로 가른다. **첫 `:`만 본다** — 상세에는 콜론이 흔하고(`C:\…`, `HTTP 401: …`) 코드에는 없기 때문이다.

`backendErrorText(err, overrides?)`가 코드를 `BACKEND_ERROR_KEY`(→ `common:errors.*`)로 옮기고, **기술적 상세는 번역하지 않고 괄호에 원문 그대로 붙인다.** 경로·OS 오류 문자열·포트 번호는 사용자가 개발자에게 그대로 전달할 것이라 옮기면 오히려 쓸모가 준다.

두 가지 열림/닫힘이 섞여 있다.

- `BackendErrorCode`는 **유니언**이다. 코드를 추가하면 `BACKEND_ERROR_KEY`에 문구를 넣기 전까지 tsc가 잡는다.
- 그런데 **매핑에 없는 코드가 와도 정상**이다 — 상류 오류(CLI stderr, HTTP 상태)는 종류가 열려 있어 전부 열거할 수 없다. 그때는 원문을 그대로 보여준다. 즉 유니언은 "번역해 줄 코드 목록"이지 "백엔드가 낼 수 있는 전부"가 아니다.

`overrides`(두 번째 인자)는 그 화면에서만 다르게 말해야 하는 코드용이다. 지금 쓰는 곳은 한 군데 — `SettingsDialog`의 요약 테스트가 `SUMMARY_TEST_ERROR_KEY`(`summarizer-disabled`·`openrouter-key-missing`)를 넘긴다. 이게 이 관례를 처음 세운 자리이고, `BACKEND_ERROR_KEY`는 그것을 공통 매핑으로 일반화한 것이다.

화면 전용 문구가 더 복잡하면 `overrides`를 쓰지 않고 자기 캡션 함수를 두되, **분해와 최종 폴백은 같은 모듈에 맡긴다**.

- `sessionLogStore.ts`의 `SESSION_LOG_ERROR_KEY` — `empty-log`·`summarizer-disabled`는 공통 매핑에 두지 않았다. 같은 코드라도 설정 화면(설정 유도)과 세션 로그 목록(목록 안내)에서 해야 할 안내가 다르기 때문이다. 모르는 코드는 `activity:sessionLog.errGeneric`에 `backendErrorText(err)` 결과를 끼워 넣는다.
- `CodexGenPanel.tsx`의 `codexGenErrorCaption` — `parseBackendError`로 코드를 뽑아 `profile:codex.*`로 옮기고, `{provider}-not-found` 관례는 포함 검사로 잡는다. 최종 폴백은 역시 `backendErrorText(err)`.

같은 관례가 `src/shared`에도 적용됐다. `parseCharacterBundle`은 예전에 한국어 오류 문자열을 돌려줬는데 이제 `CharacterBundleError` 코드를 돌려준다 — **`src/shared`는 renderer에 의존할 수 없어 `t()`를 부를 수 없고**, 부를 수 있더라도 그 결과는 그리는 쪽(`ProfileDialog`)이 자기 언어로 옮겨야 한다.

## 4. 규칙

이 절만 읽고도 새 화면을 i18n에 맞게 쓸 수 있어야 한다.

### 4.1 세 자리의 경계

| 넣을 것 | 갈 곳 |
|---|---|
| 화면에 나가는 문구 | `src/shared/i18n/locales/ko/*.json` (+ en) |
| AI에게 보내는 프롬프트와 그에 딸린 상수 | `src/renderer/i18n/promptProfiles.ts` |
| 사용자 입력 판정 규칙 | `src/renderer/i18n/textRules.ts` |

캐릭터 이름 같은 언어별 자료는 `wordlists.ts`. 이 넷 말고 다른 곳에 한글 리터럴을 두면 테스트가 막는다(§6).

### 4.2 키 네이밍과 복수형

키는 `ns:영역.항목`. 영역은 대개 화면/컴포넌트 단위다(`workdir:palette.retry`, `activity:sessionLog.errSummarizerOff`).

복수형은 i18next 규칙대로 `키_one` / `키_other`를 만들고 `t(key, { count })`로 부른다. **en에 만들면 ko에도 같은 키가 필요하다** — 한국어에 복수형이 없어도 파리티 테스트가 키 집합의 완전 일치를 요구하기 때문이다. ko 쪽은 두 값이 같아도 된다:

```jsonc
// ko/journal.json
"days_one":   "{{count}}일",
"days_other": "{{count}}일",
```

값이 똑같아 보여 지우고 싶어지지만, 지우면 파리티가 깨진다.

### 4.3 모듈 최상위에서 `t()`를 부르지 마라

부르면 **모듈 로드 시점의 언어로 굳는다.** 설정에서 언어를 바꿔도 그 값은 안 바뀐다.

상수 배열/레지스트리에는 번역문 대신 **`labelKey`(번역 키)를 담고**, 번역은 소비처가 렌더 시점에 한다. 이 저장소에서 그렇게 고친 곳: `theme/themes.ts`(`THEMES[id].labelKey`), `sound/packs.ts`(`PACK_META`), `terminal/palettes.ts`, `office/scenes/*.ts`, `office/gen/archetypes.ts`.

같은 이유로 언어별 프로필 선택도 **호출 시점에** 한다 — `currentTextRules()`·`currentWordlists()`·`labelPromptProfile()`은 매번 `i18n.language`를 읽는다. 모듈 최상위에서 `const rules = textRulesFor(...)`로 굳히면 안 된다.

### 4.4 스토어·영속 데이터에 번역문을 저장하지 마라

키와 데이터만 담는다. 문장은 그리는 쪽에서 만든다(§3.5).

**단, AI 생성물과 사용자가 쓴 글은 콘텐츠다.** 이건 번역 대상이 아니라 보존 대상이라 원문 그대로 둔다 — 재번역도, 재생성도 하지 않는다. 경계는 이렇게 갈린다.

| 대상 | 처리 | 근거 |
|---|---|---|
| 일기 본문(`DiaryEntry.body`) | 원문 보존 | 생성 당시 언어로 쓰인 사료다. `DiaryDialog`는 껍데기(제목·버튼·빈 상태)만 번역한다 |
| 일기 알림 | 제목은 번역, 본문은 원문 | `diaryAutoWriter`가 `t("journal:diary.title", {name})` + `previewBody(entry.body)` |
| 수상 소감 | 원문 보존 | 같은 이유. 언어를 바꿨다고 지난달 소감을 다시 뽑지 않는다 |
| 포스트잇 메모 | 원문 보존 | 사용자가 쓴 글이다 |
| 동료 대화 로그 본문 | 원문 보존. `kindLabel`(로그 종류)만 번역 | 대화 내용은 콘텐츠, 종류는 UI |
| 태스크 라벨(요약) | 원문 보존, 다음 요약부터 새 언어 | 모델이 생성한 값이고 스토어에 있다 |

즉 **"UI가 말하는 것"과 "앱이 보관하는 것"의 경계**다. UI가 말하는 것은 전부 키로, 앱이 보관하는 것은 전부 원문으로.

### 4.5 순수 모듈이 `t`를 얻는 두 가지 방식

- **`t`를 인자로 주입** — 그 함수의 결과가 곧 화면 문구이고, 호출자가 React 컴포넌트인 경우. 컴포넌트의 `useTranslation()`이 준 `t`를 넘겨야 **언어 변경 시 리렌더가 걸린다**. 예: `talkLogView.kindLabel(kind, t)`, `renderText(text, t)`.
- **모듈 `t`를 호출 시점에 사용** — 호출자가 React 밖이거나(스토어·Pixi·부팅 경로) 결과가 곧바로 쓰이고 버려지는 경우. 예: `backendErrorText`, `formatRelativeTime`, `themeLabel`, `OfficeScene`의 팻말.

어느 쪽이든 금지는 하나다: **모듈 로드 시점에 부르지 않는다.**

`themes.ts`의 `themeLabel(id)`가 두 방식이 만나는 자리다 — 모듈 `t`를 쓰지만, React 컴포넌트는 이걸 쓰지 말고 자기 `t(THEMES[id].labelKey)`로 번역해야 리렌더가 걸린다고 주석이 못 박아 뒀다.

### 4.6 `Intl.*`은 `currentLocale()`로

날짜·상대시간·숫자 포맷에 넘길 로케일은 `@renderer/i18n`의 `currentLocale()`에서 얻는다.

- **`navigator.language`를 직접 참조하지 마라.** 사용자가 설정에서 오버라이드한 언어를 무시하게 된다.
- **모듈 최상위에 `Intl` 인스턴스를 만들지 마라.** 언어가 굳는다. 굳이 캐시하고 싶으면 `relativeTime.ts`처럼 **로케일을 키로 한 Map**에 담는다 — 생성 비용만 아끼고 언어는 따라간다.

## 5. 언어를 추가하는 법

`fr`을 예로 든다.

1. **`src/shared/i18n/locales/fr/` 를 만들고 JSON 9개를 넣는다.** 파일 이름·키 구조는 `ko/`와 정확히 같아야 한다(빠뜨리면 파리티 테스트가 무엇이 없는지 알려 준다).
2. **`fr/common.json`에 `_meta.label`을 넣는다.** 값은 **그 언어로 표기한 언어 이름**(`"Français"`). 설정 드롭다운에 이 값이 그대로 나간다. 빠뜨리면 코드(`fr`)가 표시되고 파리티 테스트가 실패한다.
3. **복수형 키를 맞춘다.** ko/en에 `_one`/`_other`가 있는 키는 fr에도 있어야 한다.
4. `npx vitest run --dir src` — 파리티 테스트가 키 누락·보간 이름 불일치·빈 값·en 아닌 언어의 한글 잔류를 잡는다.

**코드를 고칠 필요가 없는 것**: 지원 언어 목록, 설정 드롭다운, `AppSettings.language` 타입(TS·Rust 양쪽), 폴백 해석. 전부 카탈로그 폴더에서 도출된다(§2.2, §2.3).

**코드라서 자동으로 안 따라오는 것**(전부 en 폴백으로 돈다):

| 자산 | 없으면 |
|---|---|
| `promptProfiles.ts` 프로필 | 요약·일기·소감이 **영어로** 생성된다 |
| `textRules.ts` 규칙 | 영어 판정 규칙으로 목표 추출이 돈다(프랑스어 요청 문장을 잘 못 잡는다) |
| `wordlists.ts` 낱말 | 랜덤 프로필 초안이 영어 이름/역할로 나온다 |
| `src-tauri/src/i18n.rs`의 `Lang` variant | 백엔드 프롬프트(TTS 리라이트·학습자료·중략 마커·훅 알림 기본 문구)가 영어로 돈다 |

**이건 미완성이 아니라 설계된 상태다.** 프롬프트는 기계 번역해서 넣으면 출력 품질이 무너지므로, 그 언어를 쓰는 사람이 실제 출력을 보며 튜닝해서 넣어야 한다. UI만 먼저 프랑스어로 쓰고 프롬프트는 영어로 도는 상태가 유효한 중간 지점이다.

`Lang` variant를 추가할 때는 `notification/hub.rs`의 fallback 상수도 함께 늘려야 한다(파리티 테스트가 카탈로그와 대조한다 — §6).

## 6. 검사 장치

이 저장소에는 eslint가 없다. 그래서 i18n 규율은 전부 **테스트**가 지킨다.

### 6.1 `src/shared/i18n/__tests__/catalogParity.test.ts`

`SUPPORTED_LANGUAGES`를 돌며(= 언어를 추가하면 자동으로 검사 대상이 된다) 여섯 가지를 본다.

- 정본(ko) 카탈로그가 존재한다
- 모든 언어가 같은 **네임스페이스 집합**을 갖는다
- 모든 언어가 정본과 같은 **키 집합**을 갖는다(missing/extra를 목록으로 보여준다)
- **보간 플레이스홀더 이름 집합**이 언어마다 같다 — `{{count}}`를 en에서 `{{n}}`으로 바꿔 쓰면 여기서 걸린다
- **빈 문자열인 번역이 없다**
- **정본이 아닌 언어에 한글이 남아 있지 않다** — 번역을 빠뜨리고 ko 값을 복사해 둔 흔적을 잡는다

`_meta`는 카탈로그 메타데이터라 값 비교에서 제외된다.

### 6.2 `src/renderer/__tests__/noHardcodedHangul.test.ts` + `hangulScan.ts`

`src/renderer`·`src/shared`의 `.ts`/`.tsx`를 **TypeScript 컴파일러 API로 AST 순회**하며 문자열 리터럴·템플릿 리터럴·JSX 텍스트 노드 안의 한글만 센다.

정규식 grep이 아니라 AST를 쓰는 이유는 하나다: **이 저장소는 한국어 주석이 아주 많다.** grep이면 주석 오탐이 압도적이라 도구가 무용지물이 된다. AST는 주석·JSDoc을 원천적으로 배제한다. 반대로 JSX 텍스트 노드(`<b>설정</b>`)는 화면에 그대로 나가므로 함께 잡는다.

제외 경로는 셋뿐이다 — `__tests__`(픽스처·기대값), `/i18n/`(카탈로그와 언어별 코드 자산), `*.d.ts`.

**래칫 98 → 0**: 이행 시작 시점에 위반 파일이 98개였다. `hangulBaseline.json`에 그 목록을 적어 두고 phase마다 줄여 왔다. 세 방향으로 조인다.

1. 베이스라인에 없는 파일에서 한글이 나오면 실패 (새 위반 금지)
2. 베이스라인에 있는데 이제 깨끗하면 실패 (목록에서 빼라 — 조이기)
3. **베이스라인 자체가 비어 있어야 한다**

지금 베이스라인은 `{"files": []}`다. 즉 실질적으로 **전면 금지**이고, 예외를 다시 만들려면 3번 테스트를 의식적으로 깨야 한다 — 파일 하나가 슬그머니 목록에 얹히는 걸 막는 장치다.

### 6.3 `src/test-setup.ts`

vitest `setupFiles`. 테스트의 UI 언어를 정본(ko)으로 못박는다.

이게 없으면 **jsdom의 `navigator.language`가 `en-US`**라 컴포넌트가 영어로 렌더되고, 화면 문구를 한국어로 단언하는 기존 테스트가 전부 깨진다.

방향을 뒤집어 테스트를 영어로 고치는 선택지도 있었지만, ko가 정본 카탈로그이므로 **테스트가 곧 ko 문구의 명세**로 남는 편이 낫다고 봤다. 영어 화면은 자동 단언이 아니라 phase별 눈검증으로 확인했다 — 문구가 맞는지보다 **길이가 레이아웃을 깨지 않는지**가 관건이라 어차피 눈이 필요하다.

특정 테스트에서 다른 언어를 보려면 그 파일에서 `initI18nForTest("en")`을 부르고 `afterEach`로 되돌린다.

### 6.4 Rust ↔ 프런트 짝 고정

`src-tauri/src/notification/hub/tests.rs`의 `attention_fallback_matches_the_frontend_preview_message`가 **`src/shared/i18n/locales/{ko,en}/common.json`을 직접 읽어** `ATTENTION_FALLBACK_KO`/`_EN` 상수와 대조한다.

이 짝이 필요한 이유: 설정의 알림음 "시청" 버튼이 내는 미리듣기 문구(`soundManager.previewMessage()`)는 카탈로그의 `common:notification.attentionFallback`에서 오고, 실제 훅 알림의 기본 문구는 Rust 상수에서 온다. 두 값이 같아야 "시청으로 들리는 것 = 실제 알림"이 성립한다. 어느 한쪽만 고치면 테스트가 깨진다.

같은 파일의 `fallback_messages_split_by_language`는 en 문구에 한글이 섞이지 않았는지도 본다.

### 6.5 ko 프롬프트 동결

`src/renderer/i18n/__tests__/promptProfiles.test.ts`에 이행 직전(`dbf428d` 시점)의 ko 프롬프트 **사본**이 상수로 박혀 있고, 바이트 단위로 비교한다. 딸린 숫자·마커·정규식·머리말도 함께 못 박는다(`summaryMaxChars: 40`, `metaMarkers: ["인코딩","죄송","할 수 없"]`, `linePrefixPattern` 등).

프롬프트 한 글자가 요약·일기·소감의 품질을 바꾼다. 이 테스트가 깨지면 둘 중 하나다 — 실수로 건드렸거나(되돌려라), 의도한 튜닝이거나(그 커밋에서 사본도 함께 갱신하고 커밋 메시지에 **품질 변경**임을 남겨라).

같은 파일이 폴백 규칙(프로필 없는 언어 → en)도 검증한다. `textRules.test.ts`는 ko/en 판정 규칙의 오탐 사례("네트워크 설정 고쳐줘"가 맞장구가 아님 등)를 지킨다.

## 7. 함정

실제로 밟은 것들이다.

### 7.1 JSX 공백 접힘

여러 줄로 흩어져 있던 JSX 텍스트를 카탈로그 값 한 줄로 옮길 때, **기준은 `textContent`**다. JSX는 줄바꿈과 그 앞뒤 들여쓰기를 공백 하나로 접으므로, 소스만 보고 옮기면 있어야 할 공백이 사라진다.

특히 **문구와 인라인 요소 사이의 선행/후행 공백**이 위험하다. `{t(...)}` 와 `<button>` 이 붙어 버린다.

```jsx
// 이행 전 — "git 상태 조회를 취소했습니다. [다시 시도]"
git 상태 조회를 취소했습니다.{" "}
<button …>다시 시도</button>

// 이행 후 — {" "}를 반드시 남긴다
{t("palette.gitCanceledNote")}{" "}
<button …>{t("palette.retry")}</button>
```

`{" "}`를 명시적으로 남기지 않으면 붙는다. `WorkdirPalette`·`WorkdirDetailPane`·`WorkdirRepoLogPane`·`SettingsDialog`·`WebRemoteSection`·`UsageWidget`·`PostItWidget`·`AwardsDialog`·`SessionTimePanel`에 이 패턴이 남아 있다.

### 7.2 모듈 최상위 정렬이 언어를 굳힌다

`sound/packs.ts`의 `KEYBOARD_SOUND_PACK_OPTIONS`는 모듈 최상위 상수인데, 처음에는 **번역된 라벨**을 `localeCompare`로 정렬하고 있었다. 그러면 모듈 로드 시점의 언어로 순서가 굳어, 설정에서 언어를 바꿔도 드롭다운 순서가 안 바뀐다.

고친 방법은 정렬 기준을 **번역 키**로 바꾼 것이다(`sortKey = labelKey ?? id`). 키 순서는 언어와 무관하게 안정적이다. 지금 등록된 네 팩은 마침 ko/en 라벨순과도 같아서 사용자 눈에는 달라진 게 없다.

일반화하면: **모듈 최상위에서 만드는 목록은 정렬 기준까지 언어 독립이어야 한다.** `t()`를 안 불렀더라도 `localeCompare`가 언어를 끌어들일 수 있다.

### 7.3 `Intl.RelativeTimeFormat`의 `numeric: "auto"` vs `"always"`

`relativeTime.ts`는 `"always"`를 쓴다. 기본값처럼 보이는 `"auto"`로 두면 ko가 하루/이틀 전을 **"어제"/"그저께"**로 바꿔 버린다.

이 함수의 쓰임은 팔레트 헤더의 **캐시 신선도 표시**("N일 전 기준")다. 관용 표현보다 경과 시간을 숫자로 읽히는 쪽이 맞다(en도 "yesterday"보다 "1 day ago"가 낫다). 분·시 단위는 두 모드가 같은 결과를 내므로, `"always"`를 고르면 손조립 시절의 ko 출력이 그대로 보존된다는 부수 효과도 있다.

`Intl`에 맡긴 대가는 감수한 것이다 — 언어를 추가할 때 상대시간 표기가 카탈로그 한 줄 없이 따라온다. 카탈로그에 남은 건 `Intl`이 만들지 못하는 임계 구간 문구(`common:time.justNow`)뿐이다.

### 7.4 `promptBudgetChars = 1900`은 언어별로 키우면 안 된다

`speechPromptProfile.promptBudgetChars`는 ko/en **둘 다 1,900**이다. 다른 분량 상수는 전부 영어에서 키웠는데(`personalityMaxChars` 300→500, `speechMaxChars` 240→480, `perEntryChars` 200→400) 이것만 그대로다.

근거가 다르기 때문이다. 이 값은 **한글 밀도가 아니라 백엔드 상한**에서 나왔다. `src-tauri/src/summarizer/mod.rs`의 `TEXT_MAX_CHARS = 2_000`을 넘으면 `cap_text`가 머리 60% + 중략 마커 + 꼬리 40%로 잘라 버린다 — 소감 프롬프트가 잘리면 월 중간의 일기 발췌가 통째로 날아간다. 1,900은 거기서 100자 여유를 둔 값이고, **그 100자가 곧 중략 마커의 길이 예산**이다(`MARKER_MAX_CHARS = TEXT_MAX_CHARS - 1_900`).

Rust 쪽 `truncation_marker_fits_the_frontend_prompt_budget` 테스트가 이 관계를 못 박는다. 언어를 추가할 때 마커가 100자를 넘으면 거기서 깨진다.

영어에서 같은 예산이 담는 정보가 적어지는 건 사실이지만, 예산을 키우면 백엔드가 자른다. 그래서 en 프로필은 **고정 예산 안에서 배분만 조정**했다 — 성격 500 + 발췌(8편 × 400자, 총 1,500 상한).

### 7.5 Pixi 텍스트의 `languageChanged` 리스너는 두 곳에서 떼야 한다

`OfficeScene`의 사장 책상 팻말은 React 밖이라 언어를 바꿔도 저절로 안 바뀐다. `i18n.on("languageChanged", …)`로 `.text`만 갈아 끼운다(씬 재구축 없이).

문제는 해제다. 이 씬은 **풍경 전환 시 부분 재구축**(팻말 teardown)과 **씬 파기**(destroy) 두 경로를 갖는다. `offLanguage`는 **양쪽 모두에서** 호출해야 한다.

- teardown에서 안 떼면: `buildBossDesk`가 다시 구독해 리스너가 중복 누적되고, 죽은 `Text`를 건드린다.
- destroy에서 안 떼면: 씬이 사라져도 i18next가 리스너를 붙들고 있어 누수된다.

해제 순서도 정해져 있다 — `offLanguage?.()`를 **`bossSign.destroy()` 전에** 부른다. 뒤로 미루면 파기된 `Text`에 쓰기가 갈 수 있다. `OfficeScene.test.ts`가 i18next 내부 `observers.languageChanged`의 구독자 수를 세어 이걸 지킨다.

`offVacation`(휴가 모드 구독)이 이미 같은 모양이었으므로 관례를 따른 것이다.

### 7.6 마스코트 창은 언어 전환이 실시간 전파되지 않는다

마스코트 창은 **별도 webview**라 메인 창의 i18next 인스턴스를 물려받지 못한다. 그래서 `src/renderer/mascot/main.tsx`가 자기 몫으로 `import "../i18n"`을 한다.

그런데 이 창은 설정을 읽지 않는다(main이 밀어주는 상태를 그리기만 한다). 따라서 **localStorage 캐시(`agent-office.lang`, 메인 창이 남긴 값)가 곧 언어**다. 설정에서 언어를 바꾸면 메인 창은 즉시 바뀌지만 **이미 떠 있는 마스코트 창은 안 바뀐다** — 다음 창 생성 때 반영된다.

의도적으로 남긴 한계다. 실시간 전파는 §8(후속 과제).

## 8. 범위 밖 (후속 과제)

| 표면 | 상태 | 비고 |
|---|---|---|
| 웹 리모트 프런트 `src/web` | 한국어 하드코딩 유지 | 별도 번들. 카탈로그는 `shared`에 있으므로 자기 i18next 글루만 만들면 재사용 가능(§2.2) |
| 웹 리모트 백엔드 `src-tauri/src/webremote/**` | 한국어 유지 | 위와 한 몸 |
| VSCode 확장 `vscode-ext/` | 한국어 유지 | 별개 빌드·별개 배포 |
| 마스코트 창 언어 실시간 전파 | 미구현 | §7.6 |
| 에이전트(CLI)에게 나가는 메시지 — `src-tauri/src/control/client.rs`, `talk/skill.rs`, `bot/**` | **의도적 제외** | 사용자 화면이 아니라 **모델·CLI가 읽는 문자열**이다. UI 언어와 무관하게 안정적이어야 하고, 바꾸면 스킬·봇 프롬프트의 동작이 바뀐다 |

마지막 항목이 중요하다. `control/client.rs`·`talk/skill.rs`에 한국어가 100줄 넘게 남아 있는 것은 이행 누락이 아니라 **경계**다. 이 문자열들의 독자는 사용자가 아니라 에이전트다.

## 9. 핵심 설계 결정 요약

| 항목 | 결정 | 이유 |
|---|---|---|
| 라이브러리 | i18next + react-i18next | React 밖(Pixi·zustand·순수 모듈)에서도 `t()`가 필요 |
| 인스턴스 | `createInstance()` (전역 싱글턴 아님) | 테스트가 언어 상태를 서로 밟지 않게 |
| 카탈로그 위치 | `src/shared/i18n/locales/` | 웹 리모트 재사용 여지 |
| 로딩 | `import.meta.glob` eager | 동기 init 가능 → 첫 페인트 플래시 없음, Suspense 불필요 |
| 언어 목록 | glob에서 도출 | 언어 추가 = 폴더 추가 |
| `AppSettings.language` | 자유 문자열 | 언어를 늘려도 TS·Rust 타입 계약이 안 바뀜 |
| 정본 / 폴백 | `SOURCE_LANGUAGE=ko` = `fallbackLng`, `FALLBACK_LANGUAGE=en` | 키 누락 시 한국어, 언어 자체가 없을 때 영어 |
| 부팅 언어 | localStorage 캐시 → 설정 도착 후 교정 | 테마·터미널 뷰 모드와 같은 관례 |
| Rust 번역 | 카탈로그 없음, 코드만 내려보냄 | 언어를 타는 문자열이 프롬프트 몇 개뿐. 기존 `"{code}: {detail}"` 관례의 확장 |
| 프롬프트 | 카탈로그가 아니라 `promptProfiles.ts` | 문자열이 아니라 숫자·정규식·포매터가 딸린 묶음. en은 ko의 번역이 아님 |
| 입력 판정 | `textRules.ts` | 요청의 닻이 ko는 어미, en은 문두 — 규칙이 통째로 뒤집힘 |
| 프로필 폴백 | 없는 언어는 en 프로필 | 기계 번역 프롬프트로 출력 품질 망치느니 영어 |
| 뷰모델 반환 | `TextKey {key, params}` | 언어 변경 추종 + 테스트가 문구에 안 묶임. 보간값에 `TextKey` 중첩 가능 |
| 하드코딩 방지 | TS AST 스캐너 + 빈 베이스라인 | eslint 부재. 정규식 grep은 한국어 주석 오탐으로 무용 |
| 테스트 언어 | ko 고정 (`test-setup.ts`) | jsdom `navigator.language`는 `en-US`. ko 테스트 = ko 문구 명세 |
| ko 프롬프트 | 바이트 단위 동결 | 이행이지 튜닝이 아님. 튜닝은 별도 작업 |
