// src-tauri/src/persistence/settings_store.rs
//
// 앱 전역 설정(`settings.json`, Tauri app data dir) 영속화. 파일 부재 =
// 첫 실행(first_run=true) — 렌더러가 첫 실행 동의 다이얼로그를 띄우는
// 신호다. 파일이 있는데 못 읽으면 first_run은 false(파일이 존재했다는 것 자체가
// 온보딩 완료의 증거 — 유저를 온보딩으로 다시 괴롭히지 않는다).
//
// 손상 처리(수정 F5): 다음 설정 변경이 기본값으로 원본을 덮으므로, 못 읽은
// 파일은 `settings.json.corrupt-<시각>`으로 보존하고 로그를 남긴다. JSON
// 객체이고 버전이 1이면 필드 단위로 살린다 — 타입이 틀린 최상위 필드만
// 기본값이 된다(`parse_settings`). JSON이 아니거나 버전이 다르면 전부 기본값.
// 쓰기는 ProfileStore와 같은 fsync+rename 원자 쓰기(`durable::write_atomic`).
//
// 사운드 설정은 한 덩어리(`soundEnabled`)에서 타건음/알림음으로 쪼개졌다(한때
// 셋째 갈래였던 알림 대사 TTS는 점검·정리 §7.D C23으로 제거). 파일에 새 키가
// 없으면 옛 `soundEnabled` 값으로 둘 다 초기화한다(`migrate_sound_keys`) — 버전
// 올림 없이 로드 시점에 한 번 접어 넣는 방식이라 옛 앱으로 되돌아가도 파일이
// 깨지지 않는다. 옛 `tts*` 키들은 serde가 무시하고 다음 저장에서 사라진다.

use std::fs;
use std::path::PathBuf;

use super::durable::{self, CorruptBackup};

fn default_true() -> bool {
    true
}
fn default_sound_volume() -> f32 {
    0.5
}
fn default_attention_hold_ms() -> u64 {
    5000
}

/// 요약(라벨·일기·학습자료)에 사용할 제공자. 기존 설정과의 호환을 위해
/// 기본은 Claude.
///
/// 앞의 다섯은 로컬 CLI를 서브프로세스로 부르고, `Openrouter`만 HTTP(OpenAI 호환
/// chat/completions) 경로다 — CLI 설치 없이 한 키로 여러 벤더 모델을 쓰려는
/// 사용자를 위한 갈래이고, 키는 0600 키 스토어(`key_store`)에 둔다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SummaryProvider {
    #[default]
    Claude,
    Codex,
    Agy,
    Gemini,
    /// opencode CLI(sst/opencode). 자체 구독(opencode-go)과 여러 벤더를 한
    /// CLI로 묶어 쓰는 갈래 — 모델 id가 `<provider>/<model>` 표기다.
    Opencode,
    Openrouter,
}

impl SummaryProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Agy => "agy",
            Self::Gemini => "gemini",
            Self::Opencode => "opencode",
            Self::Openrouter => "openrouter",
        }
    }
}

/// "OS 터미널로 열기"가 사용할 외부 터미널 앱. macOS에는 시스템 차원의
/// "기본 터미널" 개념이 없어 앱 설정으로 고른다 — 기본은 OS 제공
/// Terminal.app. macOS에서만 의미가 있다(다른 OS는 무시).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalTerminal {
    #[default]
    Terminal,
    Iterm,
}

/// 마스코트 신호등(docs/mascot-lights-design.md) 대상 선택 모드. 기본은 꺼짐 —
/// `mascot_enabled`가 상위 게이트이고 이 값이 `off`가 아니어야 strip이 뜬다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MascotLightsMode {
    #[default]
    Off,
    Agents,
    Projects,
}

/// 신호등 칸 얼굴 원판에 무엇을 띄울지. 기본은 스프라이트(현행) — `Portrait`를
/// 골라도 초상이 없는 에이전트(`portrait_updated_at` 없음)는 렌더러가 스프라이트
/// 얼굴로 폴백한다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MascotLightsFace {
    #[default]
    Sprite,
    Portrait,
}

/// 신호등 칸에 표시할 텍스트. 기본 `Auto` = 현행 그대로(에이전트 모드는 에이전트
/// 이름, 프로젝트 모드는 폴더 basename). 고른 값이 그 칸에서 비어 있으면(예:
/// cwd 없는 에이전트에 `Project`) 렌더러가 `Auto`로 폴백한다 — 빈 칸을 만들지
/// 않는다. `ProjectTask`는 칸을 두 줄로 그린다 — 첫 줄 프로젝트명, 둘째 줄
/// 작업명(렌더러는 이때 타일을 세로로도 키운다, `LIGHT_TILE_H_TALL`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MascotLightsLabel {
    #[default]
    Auto,
    Agent,
    Project,
    Task,
    ProjectTask,
}

/// 셸 출력 내보내기(.txt)를 열 외부 에디터. 기본은 OS 기본 연결(open/xdg-open).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExternalEditor {
    #[default]
    System,
    Vscode,
}

/// 파일 목록 스캔 백엔드(이슈 #67). `Walker`는 `ignore::WalkBuilder` 병렬
/// 스캔(기본), `Everything`은 Windows 전용 es.exe(Voidtools Everything CLI)로
/// 후보를 빠르게 얻은 뒤 gitignore 등가성 필터를 거친다 -- md 팔레트에만
/// 적용되고, es.exe 부재/실패/타임아웃 시 조용히 Walker로 폴백한다. 기존
/// 설정 파일에 이 키가 없으면(구버전) `#[serde(default)]`로 Walker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FileIndexBackend {
    #[default]
    Walker,
    Everything,
}

/// 요약기 provider 하나의 모델 오버라이드. **빈 문자열 = 오버라이드 없음**
/// (`summarizer::SummaryPurpose`의 하드코딩 기본값을 그대로 쓴다) — Option 대신
/// 빈 문자열을 쓰는 이유는 키 스토어와 같다: 렌더러에서 "지우기"가
/// 빈 입력으로 자연스럽게 표현된다.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryModelOverride {
    /// 라벨·일기(짧은 변환)에 쓸 모델 id.
    #[serde(default)]
    pub light: String,
    /// 학습자료(긴 전사 구조화)에 쓸 모델 id.
    #[serde(default)]
    pub heavy: String,
    /// 이 provider의 CLI를 부를 **실행 명령**. 빈 문자열이면 기본 이름
    /// (`SummaryProvider::as_str()` — `claude`, `codex`, …).
    ///
    /// 값은 프로그램 하나다(PATH에서 찾을 이름 또는 절대경로). 인자는 받지
    /// 않는다 — 요약 호출의 인자 규약은 provider 모듈이 쥐고 있어야 하고,
    /// 인자가 필요한 사용자는 래퍼 스크립트를 만들면 된다. 이 필드가 있는
    /// 이유가 바로 그런 래퍼다(별개 계정으로 붙는 `claude-t` 같은 것).
    ///
    /// `Openrouter`는 HTTP 경로라 이 값을 쓰지 않는다.
    #[serde(default)]
    pub command: String,
}

/// 요약기 provider별 모델 오버라이드 모음. 전 필드 `#[serde(default)]`라
/// 기존 설정 파일에 키가 없으면 전부 "오버라이드 없음"으로 로드된다.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SummaryModels {
    #[serde(default)]
    pub claude: SummaryModelOverride,
    #[serde(default)]
    pub codex: SummaryModelOverride,
    #[serde(default)]
    pub agy: SummaryModelOverride,
    #[serde(default)]
    pub gemini: SummaryModelOverride,
    /// opencode 모델 id(`<provider>/<model>` 표기 — `opencode models` 출력 형식).
    #[serde(default)]
    pub opencode: SummaryModelOverride,
    /// OpenRouter 모델 id(`<벤더>/<모델>` 표기). 다른 슬롯과 마찬가지로
    /// `#[serde(default)]`라 이 키가 없는 구 설정 파일도 그대로 로드된다.
    #[serde(default)]
    pub openrouter: SummaryModelOverride,
}

impl SummaryModels {
    /// provider 하나의 오버라이드.
    pub fn for_provider(&self, provider: SummaryProvider) -> &SummaryModelOverride {
        match provider {
            SummaryProvider::Claude => &self.claude,
            SummaryProvider::Codex => &self.codex,
            SummaryProvider::Agy => &self.agy,
            SummaryProvider::Gemini => &self.gemini,
            SummaryProvider::Opencode => &self.opencode,
            SummaryProvider::Openrouter => &self.openrouter,
        }
    }
}

/// 앱 전역 설정. 요약과 관찰자 연동은 기본 OFF이고, 사운드는 기본 ON이다.
///
/// **`Copy`가 아니다**(모델 id 같은 자유 문자열 필드가 있다). 캐시에서 꺼낼 때는
/// `*settings.read().unwrap()`이 아니라 `.clone()`을 쓰고, 필요한 스칼라 필드
/// 하나만 필요하면 가드에서 그 필드만 복사해 즉시 가드를 놓는다(락을 `.await`
/// 넘어 들고 가지 않는 crate 전역 계약과 같은 이유).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub version: u32,
    /// UI 언어 — `"system"`(OS 로케일 따름, 기본) 또는 BCP47 코드(`"ko"`, `"en"`).
    /// 유효값을 여기서 열거하지 않는 게 의도다: 번역 카탈로그 폴더만 추가하면
    /// 언어가 늘어나는 구조라(`src/shared/i18n/catalog.ts`) 언어 추가 때 Rust를
    /// 고치게 만들지 않는다. 카탈로그에 없는 값은 렌더러가 조용히 폴백한다.
    /// 백엔드에서는 AI 프롬프트의 출력 언어를 정하는 데 쓴다(`crate::i18n`).
    /// `#[serde(default)]`라 기존 설정 파일에 키가 없으면 `"system"`.
    #[serde(default = "default_language")]
    pub language: String,
    #[serde(default, alias = "claudeCliEnabled")]
    pub summarizer_enabled: bool,
    #[serde(default)]
    pub summary_provider: SummaryProvider,
    /// 요약기 provider별 모델 오버라이드(빈 문자열 = 기본 모델). 목적별 기본
    /// 모델은 `summarizer::SummaryPurpose`에 하드코딩돼 있고, 여기 값이 비어
    /// 있지 않으면 그것을 이긴다.
    #[serde(default)]
    pub summary_models: SummaryModels,
    /// 캐릭터 일기(#56) 자동 생성 허용. 요약기와 같은 provider·CLI를 쓰므로
    /// 크레딧을 소모한다 → opt-in. 기본 꺼짐.
    #[serde(default)]
    pub diary_enabled: bool,
    #[serde(default, alias = "claudeHooksEnabled")]
    pub observer_enabled: bool,
    /// 키보드 타건음(캐릭터가 출력을 뿜을 때 나는 타이핑 소리).
    /// 레거시 `soundEnabled` 하나가 담당하던 것을 타건/알림으로 쪼갠
    /// 결과다 — 마이그레이션은 `migrate_sound_keys` 참고.
    #[serde(default = "default_true")]
    pub typing_sound_enabled: bool,
    /// 알림 딩 + 세션 시작/종료 효과음.
    #[serde(default = "default_true")]
    pub notify_sound_enabled: bool,
    /// 마스터 볼륨 0.0~1.0.
    #[serde(default = "default_sound_volume")]
    pub sound_volume: f32,
    /// "OS 터미널로 열기"가 사용할 외부 터미널 앱(macOS 전용).
    #[serde(default)]
    pub external_terminal: ExternalTerminal,
    /// 셸 출력 내보내기(.txt)를 열 외부 에디터. 기본은 OS 기본 연결.
    #[serde(default)]
    pub external_editor: ExternalEditor,
    /// 질문(Hook) 알림을 방출 전 보류하는 시간(ms). 그 사이 세션이 계속
    /// 일하면(오토모드 자동 승인 등) 알림을 조용히 폐기한다. 0이면 즉시 알림.
    #[serde(default = "default_attention_hold_ms")]
    pub attention_hold_ms: u64,
    /// "작업 폴더 보기"(이슈 #11)에서 파일별 git 상태 뱃지를 조회할지. 거대
    /// 저장소에서 git status가 무거울 수 있어 끌 수 있게 한다. 기본 켜짐.
    #[serde(default = "default_true")]
    pub git_status_enabled: bool,
    /// "작업 폴더 보기"에서 `.gitignore`로 무시된 파일과 숨김(dot) 파일까지
    /// 목록에 담을지. 켜면 `target/`·`node_modules/` 같은 산출물이 통째로
    /// 들어와 5000개 상한을 금방 채울 수 있어 기본 꺼짐 -- `#[serde(default)]`라
    /// 기존 설정 파일에 키가 없으면 false.
    #[serde(default)]
    pub workdir_show_ignored: bool,
    /// 파일 목록(마크다운 팔레트) 스캔 백엔드(이슈 #67). `#[serde(default)]`라
    /// 기존 설정 파일에 키가 없으면 `FileIndexBackend::Walker`(기본값)로
    /// 폴백한다.
    #[serde(default)]
    pub file_index_backend: FileIndexBackend,
    /// 로컬 CLI 제어 서버(이슈 #55, docs/cli-control-design.md)를 띄울지.
    /// 켜면 `127.0.0.1`에 임의 포트로 control 서버가 뜨고 `control-port`가
    /// 기록된다. 하지만 실제 명령 수행은 앱에서 **명시적 승인**(control-token
    /// 발급)이 있어야 한다 — 2단계 옵트인. 보안 표면이므로 기본 꺼짐.
    #[serde(default)]
    pub cli_enabled: bool,
    /// 캐릭터가 작업 중일 때 시스템 유휴 잠자기를 막을지(이슈 #68). macOS는
    /// IOKit `PreventUserIdleSystemSleep`, Windows는 `ES_SYSTEM_REQUIRED` — 둘
    /// 다 **디스플레이 잠자기는 허용**한다(화면은 꺼져도 에이전트는 계속 돈다).
    /// 렌더러가 "일하는 캐릭터 있음"을 통지할 때만 실제로 잠자기를 막는다.
    /// `#[serde(default)]`라 기존 설정 파일에 키가 없으면 false. 기본 꺼짐.
    #[serde(default)]
    pub keep_awake_enabled: bool,
    /// 터미널 세션 로그를 파일로 상시 기록할지(docs/session-log-design.md).
    /// 나중에 회고하려면 "그때 이미 켜져 있었어야" 하므로 기본 켜짐 —
    /// 이 앱에서 기본값이 켜짐인 몇 안 되는 opt-out 기능이다. 보존은 30일·2GB로
    /// 스스로 제한한다.
    #[serde(default = "default_true")]
    pub session_log_enabled: bool,
    /// 프로젝트별 실행 레시피 메뉴를 보일지. 캐릭터 세션 stdin에 문자열을 넣는
    /// 기능이라 기본은 꺼짐이며, 끄는 것은 저장한 레시피를 지우지 않는다.
    #[serde(default)]
    pub run_recipes_enabled: bool,
    /// 좌우 업무 패널 opt-in. 수집·보존은 이 설정과 무관하다.
    #[serde(default)]
    pub work_panels_enabled: bool,
    /// 데스크톱 마스코트 창(이슈 #72, docs/mascot-window-design.md)을 띄울지.
    /// 활동 중인 캐릭터 1명을 앱 창과 별개의 투명·최상단 창으로 보여준다.
    /// 화면을 상시 점유하는 시스템 표면이라 opt-in — 기본 꺼짐.
    #[serde(default)]
    pub mascot_enabled: bool,
    /// 마스코트 신호등(docs/mascot-lights-design.md) 대상 선택 모드. `mascot_enabled`가
    /// 상위 게이트이고, 이 값이 `off`가 아니면 활동이 없어도 신호등 때문에
    /// 마스코트 창이 떠 있을 수 있다. `#[serde(default)]`라 기존 설정 파일에
    /// 키가 없으면 `off`.
    #[serde(default)]
    pub mascot_lights_mode: MascotLightsMode,
    /// 신호등 칸을 세로로 배열할지(기본 가로).
    #[serde(default)]
    pub mascot_lights_vertical: bool,
    /// 신호등 프로젝트 모드에서 칸을 받을 저장소 폴더 목록(표시 순서 = 저장
    /// 순서). 절대경로 문자열 — `mascotLightsMode==="projects"`일 때만 쓰인다.
    #[serde(default)]
    pub mascot_lights_projects: Vec<String>,
    /// 신호등 칸 얼굴 원판에 스프라이트 대신 초상화를 띄울지. `#[serde(default)]`라
    /// 기존 설정 파일에 키가 없으면 `MascotLightsFace::Sprite`(기본값).
    #[serde(default)]
    pub mascot_lights_face: MascotLightsFace,
    /// 신호등 칸에 표시할 텍스트 선택. `#[serde(default)]`라 기존 설정 파일에
    /// 키가 없으면 `MascotLightsLabel::Auto`(기본값).
    #[serde(default)]
    pub mascot_lights_label: MascotLightsLabel,
    /// 터미널 오버레이 filled 모드(이슈 #69)에서 패널 우측 아래에 반투명 LLM
    /// 사용량 플로트(`UsageFloat`, docs/usage-design.md §13)를 띄울지. 꺼도
    /// 하단바 사용량 뱃지는 그대로 보인다. 기존 설정 파일에 키가 없으면
    /// 기존 동작을 유지해야 하므로 `#[serde(default)]`(bool 기본 false)가
    /// 아니라 `default_true`를 쓴다.
    #[serde(default = "default_true")]
    pub usage_float_enabled: bool,
    /// 터미널 위 요약 바 오른쪽에 현재 세션의 누적 토큰과 공개 API 요율로 환산한
    /// 추정 비용을 띄울지. 이미 수집 중인 턴 토큰(`SessionEventTokens`)을 재사용할
    /// 뿐 추가 호출이 없어 기본 켜짐(opt-out). 표시되는 숫자는 추정치일 뿐
    /// 구독제 실제 청구액이 아니다.
    #[serde(default = "default_true")]
    pub session_cost_enabled: bool,
}

fn default_language() -> String {
    "system".to_string()
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            version: 1,
            language: default_language(),
            summarizer_enabled: false,
            summary_provider: SummaryProvider::Claude,
            summary_models: SummaryModels::default(),
            diary_enabled: false,
            observer_enabled: false,
            typing_sound_enabled: true,
            notify_sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: ExternalTerminal::Terminal,
            external_editor: ExternalEditor::System,
            attention_hold_ms: 5000,
            git_status_enabled: true,
            workdir_show_ignored: false,
            file_index_backend: FileIndexBackend::Walker,
            cli_enabled: false,
            keep_awake_enabled: false,
            session_log_enabled: true,
            run_recipes_enabled: false,
            work_panels_enabled: false,
            mascot_enabled: false,
            mascot_lights_mode: MascotLightsMode::Off,
            mascot_lights_vertical: false,
            mascot_lights_projects: Vec::new(),
            mascot_lights_face: MascotLightsFace::Sprite,
            mascot_lights_label: MascotLightsLabel::Auto,
            usage_float_enabled: true,
            session_cost_enabled: true,
        }
    }
}

/// 레거시 사운드 키 접기. `soundEnabled` 하나가 담당하던 것을
/// `typingSoundEnabled`(타건음) + `notifySoundEnabled`(딩·세션 효과음)로 쪼갰다.
///
/// 규칙: **새 키가 없을 때만** 옛 `soundEnabled` 값으로 채운다. 새 키가 이미
/// 있으면 손대지 않는다(사용자가 셋을 따로 만져둔 상태를 옛 키가 덮어쓰면 안
/// 된다). 옛 키도 없으면 아무것도 하지 않고 serde 기본값(켜짐)에 맡긴다.
/// `soundEnabled`는 저장 시 더 이상 쓰이지 않으므로 다음 저장에서 사라진다
/// (`claudeCliEnabled` → `summarizerEnabled` 때와 같은 관례).
///
/// 순수 — JSON 값만 만진다.
pub fn migrate_sound_keys(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let Some(legacy) = obj.get("soundEnabled").and_then(|v| v.as_bool()) else {
        return;
    };
    for key in ["typingSoundEnabled", "notifySoundEnabled"] {
        if !obj.contains_key(key) {
            obj.insert(key.to_string(), serde_json::Value::Bool(legacy));
        }
    }
}

/// settings.json 바이트를 해석한다(순수). 문제가 있었으면 사유를 함께 돌려준다 —
/// 사유가 Some이면 호출자는 원본을 백업하고 로그를 남긴다.
///
/// 전체 파싱이 실패하면 필드 단위로 구조한다: 버전만 든 객체에서 시작해 파일의
/// 최상위 키를 하나씩 얹어 보고, 얹었을 때 파싱이 깨지는 키만 버린다(그 필드는
/// serde 기본값). 기본값 객체가 아니라 빈 객체에서 시작하는 이유는 별칭
/// (`claudeCliEnabled` 등)이 정식 키와 겹쳐 duplicate field가 나지 않게 하려는 것.
/// 중첩 객체(`summaryModels`)는 통째로 한 필드로 다룬다.
fn parse_settings(bytes: &[u8]) -> (AppSettings, Option<String>) {
    let mut value = match serde_json::from_slice::<serde_json::Value>(bytes) {
        Ok(v) => v,
        Err(e) => return (AppSettings::default(), Some(format!("JSON 해석 실패: {e}"))),
    };
    migrate_sound_keys(&mut value);
    let Some(obj) = value.as_object() else {
        return (AppSettings::default(), Some("최상위가 JSON 객체가 아님".into()));
    };
    match obj.get("version").and_then(|v| v.as_u64()) {
        Some(1) => {}
        Some(v) => return (AppSettings::default(), Some(format!("지원하지 않는 버전 {v}"))),
        None => return (AppSettings::default(), Some("version 필드가 없거나 숫자가 아님".into())),
    }
    if let Ok(settings) = serde_json::from_value::<AppSettings>(value.clone()) {
        return (settings, None);
    }

    let parses = |m: &serde_json::Map<String, serde_json::Value>| {
        serde_json::from_value::<AppSettings>(serde_json::Value::Object(m.clone())).ok()
    };
    let mut kept = serde_json::Map::new();
    kept.insert("version".into(), 1.into());
    let mut dropped = Vec::new();
    for (key, v) in obj.iter().filter(|(k, _)| *k != "version") {
        let mut candidate = kept.clone();
        candidate.insert(key.clone(), v.clone());
        if parses(&candidate).is_some() {
            kept = candidate;
        } else {
            dropped.push(key.clone());
        }
    }
    let mut settings = parses(&kept).unwrap_or_default();
    // 모르는 provider(새 버전 앱이 쓴 값 등)를 Claude 기본값으로 바꿔 놓고
    // 요약·일기를 켠 채 두면 사용자가 고르지 않은 provider의 크레딧을 쓴다.
    if dropped.iter().any(|k| k == "summaryProvider") {
        settings.summarizer_enabled = false;
        settings.diary_enabled = false;
    }
    let reason = format!("값이 맞지 않는 필드를 기본값으로 대체: {}", dropped.join(", "));
    (settings, Some(reason))
}

#[derive(Clone)]
pub struct SettingsStore {
    file: PathBuf,
    /// 이번 프로세스에서 만든 손상 원본 백업(클론끼리 공유).
    backup: CorruptBackup,
}

impl SettingsStore {
    pub fn new(file: PathBuf) -> Self {
        Self { file, backup: CorruptBackup::default() }
    }

    /// (설정, first_run). first_run은 "파일이 아예 없다"일 때만 true.
    ///
    /// 구조체로 바로 파싱하지 않고 `Value`를 한 번 거치는 이유는 레거시 키
    /// 접기(`migrate_sound_keys`)와 필드 단위 구조(`parse_settings`) 때문이다 —
    /// serde의 필드 기본값은 **다른 필드 값을 볼 수 없으므로** 파생 매크로만으로는
    /// 표현할 수 없다.
    pub fn load(&self) -> (AppSettings, bool) {
        let bytes = match fs::read(&self.file) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (AppSettings::default(), true)
            }
            Err(e) => {
                eprintln!("agent-office: settings.json을 읽을 수 없어 기본값을 씁니다: {e}");
                return (AppSettings::default(), false);
            }
        };
        let (settings, problem) = parse_settings(&bytes);
        if let Some(reason) = problem {
            let backup = match self.backup.backup(&self.file) {
                Some(path) => format!(" (원본 보존: {})", path.display()),
                None => String::new(),
            };
            eprintln!("agent-office: settings.json 손상 — {reason}{backup}");
        }
        (settings, false)
    }

    pub fn save(&self, settings: &AppSettings) -> std::io::Result<()> {
        let bytes = serde_json::to_vec_pretty(settings)?;
        durable::write_atomic(&self.file, &bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn scratch_file() -> PathBuf {
        std::env::temp_dir()
            .join(format!(
                "agent-office-settings-store-test-{}",
                uuid::Uuid::new_v4()
            ))
            .join("settings.json")
    }

    /// 점검·정리(docs/archive/health-check-cleanup-plan.md §7.G)로 없어진 기능의 설정
    /// 키가 남아 있는 옛 settings.json도 그대로 읽히고, 다음 저장에서 사라진다.
    #[test]
    fn settings_with_removed_feature_keys_still_load_and_drop_them_on_save() {
        let mut value = serde_json::to_value(AppSettings::default()).unwrap();
        let obj = value.as_object_mut().unwrap();
        for (key, v) in [
            ("ideConnectionEnabled", serde_json::json!(true)),
            ("remoteServerConnectionEnabled", serde_json::json!(true)),
            ("webRemoteEnabled", serde_json::json!(true)),
            ("webRemoteBind", serde_json::json!("all")),
            ("webRemotePort", serde_json::json!(47800)),
            ("talkEnabled", serde_json::json!(true)),
            ("talkMaxTurns", serde_json::json!(6)),
            ("ttsEnabled", serde_json::json!(true)),
            ("ttsRewriteProvider", serde_json::json!("claude-cli")),
            ("ttsRewriteModel", serde_json::json!("claude-sonnet-5")),
        ] {
            obj.insert(key.into(), v);
        }
        let legacy: AppSettings = serde_json::from_value(value).unwrap();
        assert_eq!(legacy, AppSettings::default());
        let saved = serde_json::to_value(&legacy).unwrap();
        for key in [
            "ideConnectionEnabled", "remoteServerConnectionEnabled", "webRemoteEnabled", "webRemoteBind",
            "webRemotePort", "talkEnabled", "talkMaxTurns", "ttsEnabled", "ttsRewriteProvider", "ttsRewriteModel",
        ] {
            assert!(saved.get(key).is_none(), "{key} must not be written back");
        }
    }

    #[test]
    fn load_missing_file_returns_defaults_and_first_run_true() {
        let store = SettingsStore::new(scratch_file());
        let (s, first_run) = store.load();
        assert_eq!(s, AppSettings::default());
        assert!(!s.summarizer_enabled);
        assert_eq!(s.summary_provider, SummaryProvider::Claude);
        assert!(!s.observer_enabled);
        assert!(first_run);
    }

    #[test]
    fn save_then_load_roundtrips_and_first_run_false() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        let s = AppSettings {
            version: 1,
            language: "system".to_string(),
            summarizer_enabled: true,
            summary_provider: SummaryProvider::Claude,
            summary_models: SummaryModels::default(),
            diary_enabled: false,
            observer_enabled: true,
            typing_sound_enabled: true,
            notify_sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: ExternalTerminal::Terminal,
            external_editor: ExternalEditor::System,
            attention_hold_ms: 5000,
            git_status_enabled: true,
            workdir_show_ignored: false,
            file_index_backend: FileIndexBackend::Walker,
            cli_enabled: false,
            keep_awake_enabled: false,
            session_log_enabled: true,
            run_recipes_enabled: false,
            work_panels_enabled: false,
            mascot_enabled: false,
            mascot_lights_mode: MascotLightsMode::Off,
            mascot_lights_vertical: false,
            mascot_lights_projects: Vec::new(),
            mascot_lights_face: MascotLightsFace::Sprite,
            mascot_lights_label: MascotLightsLabel::Auto,
            usage_float_enabled: true,
            session_cost_enabled: true,
        };
        store.save(&s).expect("save succeeds");
        let (loaded, first_run) = store.load();
        assert_eq!(loaded, s);
        assert!(!first_run);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    fn corrupt_backups(file: &PathBuf) -> Vec<PathBuf> {
        fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| {
                p.file_name().unwrap().to_str().unwrap().starts_with("settings.json.corrupt-")
            })
            .collect()
    }

    /// 손상 파일을 써 두고 로드한다. 원본 바이트가 백업에 그대로 보존되는지까지 본다.
    fn load_raw(bytes: &[u8]) -> (PathBuf, AppSettings, bool, Vec<PathBuf>) {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, bytes).unwrap();
        let (s, first_run) = SettingsStore::new(file.clone()).load();
        let backups = corrupt_backups(&file);
        for backup in &backups {
            assert_eq!(fs::read(backup).unwrap(), bytes, "backup holds the original bytes");
        }
        (file, s, first_run, backups)
    }

    #[test]
    fn load_corrupt_file_returns_defaults_backs_up_and_first_run_false() {
        // 파손 파일은 기본값(전부 OFF)으로 안전 폴백하되, 온보딩을 다시
        // 띄우지 않는다(first_run=false) — 파일이 존재했다는 사실 자체가
        // 온보딩 완료의 증거. 다음 저장이 덮기 전에 원본을 보존한다(수정 F5).
        let (file, s, first_run, backups) = load_raw(b"not json");
        assert_eq!(s, AppSettings::default());
        assert!(!first_run);
        assert_eq!(backups.len(), 1);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn truncated_file_is_backed_up_before_the_next_save_overwrites_it() {
        let mut edited = AppSettings::default();
        edited.observer_enabled = true;
        let full = serde_json::to_vec_pretty(&edited).unwrap();
        let truncated = &full[..full.len() / 2];
        let (file, s, _, backups) = load_raw(truncated);
        assert_eq!(s, AppSettings::default());
        assert_eq!(backups.len(), 1);

        SettingsStore::new(file.clone()).save(&s).unwrap();
        assert_eq!(fs::read(&backups[0]).unwrap(), truncated, "backup survives the save");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn wrong_type_field_falls_back_alone_and_the_rest_survive() {
        let (file, s, first_run, backups) = load_raw(
            br#"{"version":1,"summarizerEnabled":true,"observerEnabled":true,
                "soundVolume":"loud","attentionHoldMs":-1,"cliEnabled":true,
                "mascotLightsProjects":["/a","/b"],"language":"ko"}"#,
        );
        assert!(!first_run);
        assert!(s.summarizer_enabled);
        assert!(s.observer_enabled);
        assert!(s.cli_enabled);
        assert_eq!(s.mascot_lights_projects, vec!["/a".to_string(), "/b".to_string()]);
        assert_eq!(s.language, "ko");
        assert_eq!(s.sound_volume, AppSettings::default().sound_volume);
        assert_eq!(s.attention_hold_ms, AppSettings::default().attention_hold_ms);
        assert_eq!(backups.len(), 1, "a partially broken file is backed up too");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn wrong_type_nested_object_falls_back_as_a_whole() {
        let (file, s, _, _) = load_raw(
            br#"{"version":1,"diaryEnabled":true,"summaryModels":{"claude":{"light":5}}}"#,
        );
        assert!(s.diary_enabled);
        assert_eq!(s.summary_models, SummaryModels::default());
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn salvage_keeps_legacy_alias_keys() {
        let (file, s, _, _) = load_raw(
            br#"{"version":1,"claudeCliEnabled":true,"claudeHooksEnabled":true,"soundVolume":[]}"#,
        );
        assert!(s.summarizer_enabled);
        assert!(s.observer_enabled);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn valid_file_is_not_backed_up_and_backup_is_made_once_per_process() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        store.save(&AppSettings::default()).unwrap();
        store.load();
        assert!(corrupt_backups(&file).is_empty());

        fs::write(&file, b"{broken").unwrap();
        store.load();
        store.clone().load();
        assert_eq!(corrupt_backups(&file).len(), 1);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_unknown_version_returns_defaults_and_first_run_false() {
        let (file, s, first_run, backups) = load_raw(
            br#"{"version":2,"claudeCliEnabled":true,"claudeHooksEnabled":true}"#,
        );
        assert_eq!(s, AppSettings::default());
        assert!(!first_run);
        assert_eq!(backups.len(), 1);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn legacy_true_fields_map_to_enabled_claude_without_version_migration() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            br#"{"version":1,"claudeCliEnabled":true,"claudeHooksEnabled":true}"#,
        )
        .unwrap();

        let (settings, first_run) = SettingsStore::new(file.clone()).load();
        assert!(!first_run);
        assert!(settings.summarizer_enabled);
        assert_eq!(settings.summary_provider, SummaryProvider::Claude);
        assert!(settings.observer_enabled);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn summary_provider_as_str_matches_serialized_values() {
        for provider in [
            SummaryProvider::Claude,
            SummaryProvider::Codex,
            SummaryProvider::Agy,
            SummaryProvider::Gemini,
            SummaryProvider::Opencode,
            SummaryProvider::Openrouter,
        ] {
            let json = serde_json::to_string(&provider).unwrap();
            assert_eq!(json, format!("\"{}\"", provider.as_str()));
        }
        assert_eq!(SummaryProvider::Claude.as_str(), "claude");
        assert_eq!(SummaryProvider::Codex.as_str(), "codex");
        assert_eq!(SummaryProvider::Agy.as_str(), "agy");
        assert_eq!(SummaryProvider::Gemini.as_str(), "gemini");
        assert_eq!(SummaryProvider::Opencode.as_str(), "opencode");
        assert_eq!(SummaryProvider::Openrouter.as_str(), "openrouter");
    }

    // opencode 슬롯이 없는 구 설정 파일도 그대로 열려야 한다(openrouter를 뒤에
    // 붙였을 때와 같은 계약) — 못 열면 다른 설정까지 기본값으로 잃는다.
    #[test]
    fn load_settings_without_the_opencode_slot_still_reads_the_others() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            br#"{"version":1,"summarizerEnabled":true,"summaryProvider":"gemini",
                "summaryModels":{"claude":{"light":"opus","heavy":""},
                "codex":{"light":"","heavy":""},"agy":{"light":"","heavy":""},
                "gemini":{"light":"","heavy":""},
                "openrouter":{"light":"openai/gpt-5.4-nano","heavy":""}}}"#,
        )
        .unwrap();
        let (s, first_run) = SettingsStore::new(file.clone()).load();
        assert!(!first_run);
        assert!(s.summarizer_enabled, "구 파일의 다른 값이 폴백되면 안 된다");
        assert_eq!(s.summary_provider, SummaryProvider::Gemini);
        assert_eq!(s.summary_models.openrouter.light, "openai/gpt-5.4-nano");
        assert_eq!(s.summary_models.opencode, SummaryModelOverride::default());
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn opencode_provider_round_trips_through_the_settings_file() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        let mut edited = AppSettings::default();
        edited.summary_provider = SummaryProvider::Opencode;
        edited.summary_models.opencode.light = "opencode-go/glm-5.3".to_string();
        store.save(&edited).unwrap();
        assert_eq!(store.load(), (edited, false));
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    // OpenRouter 슬롯이 없는 구 설정 파일도 그대로 열려야 한다 — 못 열면
    // 전체 설정이 기본값으로 폴백해 사용자가 다른 설정까지 잃는다.
    #[test]
    fn load_settings_without_the_openrouter_slot_still_reads_the_others() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            br#"{"version":1,"summarizerEnabled":true,"summaryProvider":"codex",
                "summaryModels":{"claude":{"light":"opus","heavy":""},
                "codex":{"light":"","heavy":""},"agy":{"light":"","heavy":""},
                "gemini":{"light":"","heavy":""}}}"#,
        )
        .unwrap();
        let (s, first_run) = SettingsStore::new(file.clone()).load();
        assert!(!first_run);
        assert!(s.summarizer_enabled, "구 파일의 다른 값이 폴백되면 안 된다");
        assert_eq!(s.summary_provider, SummaryProvider::Codex);
        assert_eq!(s.summary_models.claude.light, "opus");
        assert_eq!(s.summary_models.openrouter, SummaryModelOverride::default());
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn openrouter_provider_round_trips_through_the_settings_file() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        let mut edited = AppSettings::default();
        edited.summary_provider = SummaryProvider::Openrouter;
        edited.summary_models.openrouter.light = "openai/gpt-5.4-nano".to_string();
        store.save(&edited).unwrap();
        assert_eq!(store.load(), (edited, false));
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn legacy_false_fields_map_to_disabled_claude() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            br#"{"version":1,"claudeCliEnabled":false,"claudeHooksEnabled":false}"#,
        )
        .unwrap();

        let (settings, _) = SettingsStore::new(file.clone()).load();
        assert!(!settings.summarizer_enabled);
        assert_eq!(settings.summary_provider, SummaryProvider::Claude);
        assert!(!settings.observer_enabled);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn new_version_one_settings_round_trip_codex_and_neutral_keys() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        let settings = AppSettings {
            version: 1,
            language: "system".to_string(),
            summarizer_enabled: true,
            summary_provider: SummaryProvider::Codex,
            summary_models: SummaryModels::default(),
            diary_enabled: false,
            observer_enabled: true,
            typing_sound_enabled: true,
            notify_sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: ExternalTerminal::Iterm,
            external_editor: ExternalEditor::Vscode,
            attention_hold_ms: 5000,
            git_status_enabled: true,
            workdir_show_ignored: false,
            file_index_backend: FileIndexBackend::Walker,
            cli_enabled: false,
            keep_awake_enabled: false,
            session_log_enabled: true,
            run_recipes_enabled: false,
            work_panels_enabled: false,
            mascot_enabled: false,
            mascot_lights_mode: MascotLightsMode::Off,
            mascot_lights_vertical: false,
            mascot_lights_projects: Vec::new(),
            mascot_lights_face: MascotLightsFace::Sprite,
            mascot_lights_label: MascotLightsLabel::Auto,
            usage_float_enabled: true,
            session_cost_enabled: true,
        };
        store.save(&settings).unwrap();
        let json = fs::read_to_string(&file).unwrap();
        assert!(json.contains("\"summarizerEnabled\""), "{json}");
        assert!(json.contains("\"summaryProvider\": \"codex\""), "{json}");
        assert!(json.contains("\"externalTerminal\": \"iterm\""), "{json}");
        assert!(json.contains("\"externalEditor\": \"vscode\""), "{json}");
        assert!(json.contains("\"attentionHoldMs\": 5000"), "{json}");
        assert!(json.contains("\"observerEnabled\""), "{json}");
        assert!(!json.contains("claudeCliEnabled"), "{json}");
        assert!(!json.contains("claudeHooksEnabled"), "{json}");
        assert_eq!(store.load(), (settings, false));
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn missing_provider_defaults_to_claude_and_unknown_provider_fails_safe() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, br#"{"version":1,"summarizerEnabled":true}"#).unwrap();
        assert_eq!(
            SettingsStore::new(file.clone()).load().0.summary_provider,
            SummaryProvider::Claude
        );

        // 모르는 provider는 그 필드만 기본값이 되지만, 고르지 않은 provider로
        // 크레딧을 쓰지 않도록 요약·일기는 꺼진다. 나머지 필드는 살아남는다.
        fs::write(
            &file,
            br#"{"version":1,"summarizerEnabled":true,"diaryEnabled":true,"summaryProvider":"unknown","observerEnabled":true}"#,
        )
        .unwrap();
        let loaded = SettingsStore::new(file.clone()).load().0;
        assert_eq!(loaded.summary_provider, SummaryProvider::Claude);
        assert!(!loaded.summarizer_enabled);
        assert!(!loaded.diary_enabled);
        assert!(loaded.observer_enabled);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    // 신호등 칸 라벨(§7) — 새 값 `projecttask`가 다른 값들과 같은 lowercase
    // 규약으로 직렬화되고, 설정 파일 왕복도 그대로 통과하는지 확인한다.
    #[test]
    fn mascot_lights_label_project_task_serializes_lowercase_and_round_trips() {
        assert_eq!(
            serde_json::to_string(&MascotLightsLabel::ProjectTask).unwrap(),
            "\"projecttask\""
        );

        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        let mut edited = AppSettings::default();
        edited.mascot_lights_label = MascotLightsLabel::ProjectTask;
        store.save(&edited).unwrap();
        assert_eq!(store.load(), (edited, false));
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn save_is_atomic_and_leaves_no_temp_file() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        store.save(&AppSettings::default()).expect("save succeeds");
        let names: Vec<String> = fs::read_dir(file.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert!(names.iter().any(|n| n == "settings.json"));
        assert!(
            !names.iter().any(|n| n.contains(".tmp")),
            "no temp left: {names:?}"
        );
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_settings_without_language_defaults_to_system() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, br#"{"version":1,"summarizerEnabled":true}"#).unwrap();
        let (s, _) = SettingsStore::new(file.clone()).load();
        assert_eq!(s.language, "system");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    // 카탈로그에 없는 값이 와도 백엔드는 판단하지 않는다 — 저장/로드를 그대로
    // 왕복시키고, 실제로 쓸 언어로 좁히는 일은 렌더러(catalog.matchLanguage)가
    // 한다. 언어가 늘어날 때 Rust를 고치지 않기 위한 계약이다.
    #[test]
    fn unknown_language_round_trips_untouched() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, br#"{"version":1,"language":"ja"}"#).unwrap();
        let (s, _) = SettingsStore::new(file.clone()).load();
        assert_eq!(s.language, "ja");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    // ── 리라이트 모델 enum → 공급자별 문자열 마이그레이션 ───────────────
    //
    // 옛 설정에서 Sonnet을 골라 뒀던 사용자가 업데이트했다고 Haiku로 되돌아가면
    // 안 된다 — 옛 값은 언제나 Anthropic 모델 id였으므로 그쪽으로만 옮긴다.
    // ── 요약기 모델 오버라이드 ────────────────────────────────────────
    #[test]
    fn summary_models_default_to_empty_overrides_and_round_trip() {
        let s = AppSettings::default();
        assert_eq!(s.summary_models, SummaryModels::default());
        assert_eq!(s.summary_models.for_provider(SummaryProvider::Codex).light, "");
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            json.contains(r#""summaryModels":{"claude":{"light":"","heavy":"","command":""}"#),
            "{json}"
        );

        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        let mut edited = AppSettings::default();
        edited.summary_models.codex.light = "gpt-5.4-nano".to_string();
        edited.summary_models.gemini.heavy = "gemini-3-pro".to_string();
        store.save(&edited).unwrap();
        assert_eq!(store.load(), (edited, false));
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_settings_without_summary_models_falls_back_to_empty_overrides() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        // 일부만 적힌 파일도 나머지는 빈 오버라이드로 채워진다.
        fs::write(
            &file,
            br#"{"version":1,"summaryModels":{"claude":{"light":"opus"}}}"#,
        )
        .unwrap();
        let (s, _) = SettingsStore::new(file.clone()).load();
        assert_eq!(s.summary_models.claude.light, "opus");
        assert_eq!(s.summary_models.claude.heavy, "");
        assert_eq!(s.summary_models.agy, SummaryModelOverride::default());
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn serializes_camel_case() {
        let json = serde_json::to_string(&AppSettings::default()).unwrap();
        assert!(json.contains("summarizerEnabled"), "{json}");
        assert!(json.contains("summaryProvider"), "{json}");
        assert!(json.contains("observerEnabled"), "{json}");
    }

    // 하위 호환: 사운드 필드가 없는 기존 settings.json도 기본값(켜짐/0.5)으로
    // 로드된다 — 버전 마이그레이션 없이 serde default로 처리.
    #[test]
    fn load_settings_without_sound_fields_falls_back_to_defaults() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            br#"{"version":1,"claudeCliEnabled":true,"claudeHooksEnabled":false}"#,
        )
        .unwrap();
        let (s, first_run) = SettingsStore::new(file.clone()).load();
        assert!(!first_run);
        assert!(s.summarizer_enabled);
        assert_eq!(s.summary_provider, SummaryProvider::Claude);
        assert!(!s.observer_enabled);
        assert!(s.typing_sound_enabled, "부재 시 기본 켜짐");
        assert!(s.notify_sound_enabled, "부재 시 기본 켜짐");
        assert_eq!(s.sound_volume, 0.5, "부재 시 기본 볼륨 0.5");
        assert_eq!(
            s.external_terminal,
            ExternalTerminal::Terminal,
            "부재 시 기본 Terminal.app"
        );
        assert_eq!(
            s.external_editor,
            ExternalEditor::System,
            "부재 시 기본 시스템 에디터"
        );
        assert_eq!(s.attention_hold_ms, 5000, "부재 시 기본 홀드 5초");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    // ── 사운드 3분할 마이그레이션 ─────────────────────────────────────
    //
    // 옛 설정에는 `soundEnabled` 하나뿐이다. 그 값이 **꺼짐**이었다면 새 키
    // 둘도 꺼짐이어야 한다 — 업데이트했더니 껐던 타건음이 되살아나는 것이
    // 이 마이그레이션이 막아야 할 유일한 사고다.
    #[test]
    fn legacy_sound_enabled_seeds_both_new_switches() {
        for legacy in [true, false] {
            let file = scratch_file();
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(
                &file,
                format!(r#"{{"version":1,"soundEnabled":{legacy}}}"#).as_bytes(),
            )
            .unwrap();
            let (s, _) = SettingsStore::new(file.clone()).load();
            assert_eq!(s.typing_sound_enabled, legacy, "타건음이 옛 값을 따라야 한다");
            assert_eq!(s.notify_sound_enabled, legacy, "알림음이 옛 값을 따라야 한다");
            let _ = fs::remove_dir_all(file.parent().unwrap());
        }
    }

    #[test]
    fn new_switches_win_over_legacy_key_when_both_present() {
        // 새 키를 이미 따로 만져둔 사용자가 있다. 옛 키가 그것을 덮으면 안 된다.
        let mut v: serde_json::Value = serde_json::from_str(
            r#"{"version":1,"soundEnabled":false,"typingSoundEnabled":true}"#,
        )
        .unwrap();
        migrate_sound_keys(&mut v);
        assert_eq!(v["typingSoundEnabled"], serde_json::json!(true), "보존");
        assert_eq!(v["notifySoundEnabled"], serde_json::json!(false), "채움");
    }

    #[test]
    fn migration_is_noop_without_legacy_key() {
        let mut v: serde_json::Value = serde_json::from_str(r#"{"version":1}"#).unwrap();
        migrate_sound_keys(&mut v);
        assert_eq!(v, serde_json::json!({"version":1}), "serde 기본값에 맡긴다");
    }

    // 저장하면 옛 키는 사라지고 새 키 둘만 남는다(claudeCliEnabled 때와 같은 관례).
    #[test]
    fn save_writes_split_sound_keys_and_drops_the_legacy_one() {
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        store
            .save(&AppSettings {
                typing_sound_enabled: false,
                ..AppSettings::default()
            })
            .unwrap();
        let json = fs::read_to_string(&file).unwrap();
        assert!(json.contains("\"typingSoundEnabled\": false"), "{json}");
        assert!(json.contains("\"notifySoundEnabled\": true"), "{json}");
        assert!(!json.contains("\"soundEnabled\""), "{json}");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }
    #[test]
    fn work_panels_default_off_and_opt_in_survives_save() {
        let mut legacy = serde_json::to_value(AppSettings::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("workPanelsEnabled");
        let settings: AppSettings = serde_json::from_value(legacy).unwrap();
        assert!(!settings.work_panels_enabled);
        let file = scratch_file();
        let store = SettingsStore::new(file.clone());
        store.save(&AppSettings { work_panels_enabled: true, ..settings }).unwrap();
        assert!(store.load().0.work_panels_enabled);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

}
