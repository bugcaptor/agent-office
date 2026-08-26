// src-tauri/src/persistence/settings_store.rs
//
// 앱 전역 설정(`settings.json`, Tauri app data dir) 영속화. 파일 부재 =
// 첫 실행(first_run=true) — 렌더러가 첫 실행 동의 다이얼로그를 띄우는
// 신호다. 파손/버전 불일치는 기본값(Claude 기능 OFF, 사운드 ON)으로 폴백하되 first_run은
// false(파일이 존재했다는 것 자체가 온보딩 완료의 증거 — 유저를 온보딩으로
// 다시 괴롭히지 않는다). 쓰기는 ProfileStore와 같은 temp+rename 원자 쓰기.
//
// 사운드 설정은 한 덩어리(`soundEnabled`)에서 세 갈래(타건음/알림음/TTS)로
// 쪼개졌다. 파일에 새 키가 없으면 옛 `soundEnabled` 값으로 둘 다 초기화한다
// (`migrate_sound_keys`) — 버전 올림 없이 로드 시점에 한 번 접어 넣는 방식이라
// 옛 앱으로 되돌아가도 파일이 깨지지 않는다.
//
// TTS 리라이트 모델도 같은 방식으로 접었다: 옛 `ttsRewriteModel`(Anthropic
// 모델 3종 enum)이 공급자별 자유 문자열 둘(`ttsRewriteModelAnthropic` /
// `ttsRewriteModelOpenrouter`)로 갈렸다 — `migrate_tts_rewrite_model`.

use std::fs;
use std::path::PathBuf;

fn default_true() -> bool {
    true
}
fn default_sound_volume() -> f32 {
    0.5
}
fn default_attention_hold_ms() -> u64 {
    5000
}
/// 짧은 한 줄 리라이트라 Anthropic 기본은 가장 빠르고 싼 Haiku. 설정 필드의
/// serde 기본값이자, 사용자가 입력을 **비웠을 때**의 폴백이다
/// (`tts::RewriteConfig::from_settings` — 빈 모델 id를 그대로 실으면 API가 400).
pub const DEFAULT_TTS_REWRITE_MODEL_ANTHROPIC: &str = "claude-haiku-4-5";
/// OpenRouter 기본. 모델 id 표기는 OpenRouter 규약(`<vendor>/<model>`)을 따른다.
pub const DEFAULT_TTS_REWRITE_MODEL_OPENROUTER: &str = "openai/gpt-5.4-mini";

fn default_tts_rewrite_model_anthropic() -> String {
    DEFAULT_TTS_REWRITE_MODEL_ANTHROPIC.to_string()
}
fn default_tts_rewrite_model_openrouter() -> String {
    DEFAULT_TTS_REWRITE_MODEL_OPENROUTER.to_string()
}

/// 요약(라벨·일기·학습자료)에 사용할 제공자. 기존 설정과의 호환을 위해
/// 기본은 Claude.
///
/// 앞의 다섯은 로컬 CLI를 서브프로세스로 부르고, `Openrouter`만 HTTP(OpenAI 호환
/// chat/completions) 경로다 — CLI 설치 없이 한 키로 여러 벤더 모델을 쓰려는
/// 사용자를 위한 갈래이고, 키는 TTS와 같은 0600 키 스토어를 공유한다.
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

/// 대사 리라이트를 누가 수행할지. 기본 `Auto` — 사용 가능한 경로를
/// 저렴/빠른 순으로 자동 선택한다(`tts::resolve_rewrite_route` 참고).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TtsRewriteProvider {
    /// 저장 API 키 → ANTHROPIC_API_KEY env → claude CLI → 리라이트 생략.
    #[default]
    #[serde(rename = "auto")]
    Auto,
    /// Anthropic Messages API만. 키가 없으면 리라이트를 건너뛴다.
    #[serde(rename = "api")]
    Api,
    /// OpenRouter(OpenAI 호환 chat/completions)만. **명시 선택 시에만** 쓰인다 —
    /// `Auto` 체인은 기존 순서를 유지한다(키를 넣어 뒀다고 조용히 과금 경로가
    /// 바뀌면 안 된다).
    #[serde(rename = "openrouter")]
    OpenRouter,
    /// `claude -p` 헤드리스 서브프로세스만. 구독 사용량을 소모한다.
    #[serde(rename = "claude-cli")]
    ClaudeCli,
    /// 리라이트 없음 — 원문 문구를 그대로 읽는다(합성만 사용).
    #[serde(rename = "none")]
    None,
}

impl TtsRewriteProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Api => "api",
            Self::OpenRouter => "openrouter",
            Self::ClaudeCli => "claude-cli",
            Self::None => "none",
        }
    }
}

/// 요약기 provider 하나의 모델 오버라이드. **빈 문자열 = 오버라이드 없음**
/// (`summarizer::SummaryPurpose`의 하드코딩 기본값을 그대로 쓴다) — Option 대신
/// 빈 문자열을 쓰는 이유는 tts 키 스토어와 같다: 렌더러에서 "지우기"가
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
    /// 레거시 `soundEnabled` 하나가 담당하던 것을 셋(타건/알림/TTS)으로 쪼갠
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
    /// 데스크톱 마스코트 창(이슈 #72, docs/mascot-window-design.md)을 띄울지.
    /// 활동 중인 캐릭터 1명을 앱 창과 별개의 투명·최상단 창으로 보여준다.
    /// 화면을 상시 점유하는 시스템 표면이라 opt-in — 기본 꺼짐.
    #[serde(default)]
    pub mascot_enabled: bool,
    /// 확인 요청 대사 TTS — 질문(Hook) 알림 문구를 캐릭터 말투 대사로 리라이트한 뒤
    /// ElevenLabs로 합성해 캐릭터 목소리로 재생한다. 외부 API 두 곳을 호출해
    /// 유료 크레딧을 소모하므로 opt-in — 기본 꺼짐. API 키는 이 구조체가 아니라
    /// `tts::keys::TtsKeyStore`(0600 별도 파일)에 있다 — 설정은 렌더러로 통째로
    /// 왕복하므로 키를 여기 두면 웹뷰에 노출된다.
    #[serde(default)]
    pub tts_enabled: bool,
    /// 대사 리라이트에 쓸 Anthropic 모델 id. 기본 `claude-haiku-4-5`.
    /// API 경로에서는 Messages API의 `model`, claude CLI 경로에서는
    /// `claude -p --model`의 값으로 같이 쓰인다. 자유 입력이라 오타는
    /// 호출 실패 → 원문 발화로 강등된다(장식 기능의 강등 규칙).
    #[serde(default = "default_tts_rewrite_model_anthropic")]
    pub tts_rewrite_model_anthropic: String,
    /// OpenRouter 경로에서 쓸 모델 id(`<vendor>/<model>`). 기본
    /// `openai/gpt-5.4-mini`.
    #[serde(default = "default_tts_rewrite_model_openrouter")]
    pub tts_rewrite_model_openrouter: String,
    /// 대사 리라이트 공급자. 기본 `auto`(API 키 → env → claude CLI → 생략).
    #[serde(default)]
    pub tts_rewrite_provider: TtsRewriteProvider,
    /// 어떤 원격 주소를 받아 줄지. 기본 `tailnet`(Tailscale 대역 + 루프백).
    #[serde(default)]
    pub web_remote_bind: WebRemoteBind,
    /// 수신 포트. 수동 `host:port` 입력이 곧 디스커버리라 고정값이 기본이고,
    /// 점유 시에는 +1씩 스캔한 실제 포트를 설정 UI에 표시한다.
    #[serde(default = "default_web_remote_port")]
    pub web_remote_port: u16,
    /// 웹 원격(docs/web-remote-design.md) — 브라우저로 접속해 상태 확인·터미널
    /// 조작을 하게 한다. 켜져 있을 때만 리스너가 뜨고 정적 자산·웹 RPC가
    /// 응답한다. 페어링 승인은 여전히 필요하다. 네트워크 표면이라 기본 꺼짐.
    #[serde(default)]
    pub web_remote_enabled: bool,
    /// 동료 대화(docs/agent-talk-design.md) — 캐릭터끼리 앱을 거쳐 메시지를
    /// 주고받게 한다. 남의 세션 stdin에 글자를 밀어 넣는 기능이라 opt-in이고,
    /// 끄는 순간 대기 중인 메시지까지 버려지는 킬 스위치이기도 하다. 기본 꺼짐.
    #[serde(default)]
    pub talk_enabled: bool,
    /// 한 대화의 왕복 상한(무한 핑퐁 방지). 0은 허용하지 않는다.
    #[serde(default = "default_talk_max_turns")]
    pub talk_max_turns: u32,
    /// 수신자가 이만큼 조용해야 메시지를 주입한다(작업 중 방해 방지).
    #[serde(default = "default_talk_idle_quiet_ms")]
    pub talk_idle_quiet_ms: u64,
}

fn default_language() -> String {
    "system".to_string()
}

fn default_talk_max_turns() -> u32 {
    crate::talk::DEFAULT_MAX_TURNS
}

fn default_talk_idle_quiet_ms() -> u64 {
    crate::talk::DEFAULT_IDLE_QUIET_MS
}

fn default_web_remote_port() -> u16 {
    crate::webremote::protocol::DEFAULT_WEB_REMOTE_PORT
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
            mascot_enabled: false,
            tts_enabled: false,
            tts_rewrite_model_anthropic: default_tts_rewrite_model_anthropic(),
            tts_rewrite_model_openrouter: default_tts_rewrite_model_openrouter(),
            tts_rewrite_provider: TtsRewriteProvider::Auto,
            web_remote_bind: WebRemoteBind::Tailnet,
            web_remote_port: crate::webremote::protocol::DEFAULT_WEB_REMOTE_PORT,
            web_remote_enabled: false,
            talk_enabled: false,
            talk_max_turns: default_talk_max_turns(),
            talk_idle_quiet_ms: default_talk_idle_quiet_ms(),
        }
    }
}

/// 웹 원격 서버가 받아 줄 원격 주소 범위. 인터페이스 열거 크레이트를
/// 새로 들이지 않고 **원격 주소 허용목록**으로 정책을 강제한다 — tailnet 밖
/// 클라이언트는 페어링조차 시작하지 못하므로 "기본 구성에서 평문이 LAN에
/// 흐르지 않는다"는 성질은 그대로다.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WebRemoteBind {
    /// Tailscale 대역(100.64.0.0/10, fd7a:115c:a1e0::/48) + 루프백만. 기본값.
    #[default]
    Tailnet,
    /// 아무 주소나(순수 LAN 사용 — 평문 전송 경고 동반).
    All,
    /// 루프백만(사실상 비활성 — 같은 머신 테스트용).
    Loopback,
}

impl WebRemoteBind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tailnet => "tailnet",
            Self::All => "all",
            Self::Loopback => "loopback",
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

/// 레거시 TTS 리라이트 모델 키 접기. 옛 `ttsRewriteModel`(Anthropic 모델 3종
/// enum, 값이 곧 모델 id)이 공급자별 자유 문자열 둘로 갈렸다.
///
/// 규칙은 `migrate_sound_keys`와 같다: **새 키가 없을 때만** 옛 값을 옮긴다.
/// 옛 값은 언제나 Anthropic 모델 id였으므로 `ttsRewriteModelAnthropic`에만
/// 실린다(OpenRouter 쪽은 손대지 않는다 — 그 경로는 이 마이그레이션 이전에
/// 존재하지도 않았다). `ttsRewriteModel`은 AppSettings에 더 이상 없으므로
/// serde가 무시하고, 다음 저장에서 파일에서 사라진다.
///
/// 순수 — JSON 값만 만진다.
pub fn migrate_tts_rewrite_model(value: &mut serde_json::Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    let Some(legacy) = obj
        .get("ttsRewriteModel")
        .and_then(|v| v.as_str())
        .map(str::to_string)
    else {
        return;
    };
    if legacy.trim().is_empty() || obj.contains_key("ttsRewriteModelAnthropic") {
        return;
    }
    obj.insert(
        "ttsRewriteModelAnthropic".to_string(),
        serde_json::Value::String(legacy),
    );
}

#[derive(Clone)]
pub struct SettingsStore {
    file: PathBuf,
}

impl SettingsStore {
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    /// (설정, first_run). first_run은 "파일이 아예 없다"일 때만 true.
    ///
    /// 구조체로 바로 파싱하지 않고 `Value`를 한 번 거치는 이유는 레거시 키
    /// 접기(`migrate_sound_keys`, `migrate_tts_rewrite_model`) 때문이다 —
    /// serde의 필드 기본값은 **다른 필드 값을 볼 수 없으므로** 파생 매크로만으로는
    /// 표현할 수 없다.
    pub fn load(&self) -> (AppSettings, bool) {
        match fs::read(&self.file) {
            Ok(bytes) => {
                let parsed = serde_json::from_slice::<serde_json::Value>(&bytes).map(|mut v| {
                    migrate_sound_keys(&mut v);
                    migrate_tts_rewrite_model(&mut v);
                    v
                });
                match parsed.and_then(serde_json::from_value::<AppSettings>) {
                    Ok(s) if s.version == 1 => (s, false),
                    _ => (AppSettings::default(), false),
                }
            }
            Err(_) => (AppSettings::default(), true),
        }
    }

    pub fn save(&self, settings: &AppSettings) -> std::io::Result<()> {
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(settings)?;
        let tmp = self
            .file
            .with_file_name(format!("settings.json.tmp-{}", uuid::Uuid::new_v4()));
        fs::write(&tmp, &bytes)?;
        if let Err(e) = fs::rename(&tmp, &self.file) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
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
            mascot_enabled: false,
            tts_enabled: false,
            tts_rewrite_model_anthropic: default_tts_rewrite_model_anthropic(),
            tts_rewrite_model_openrouter: default_tts_rewrite_model_openrouter(),
            tts_rewrite_provider: TtsRewriteProvider::Auto,
            web_remote_bind: Default::default(),
            web_remote_port: crate::webremote::protocol::DEFAULT_WEB_REMOTE_PORT,
            web_remote_enabled: false,
            talk_enabled: false,
            talk_max_turns: crate::talk::DEFAULT_MAX_TURNS,
            talk_idle_quiet_ms: crate::talk::DEFAULT_IDLE_QUIET_MS,
        };
        store.save(&s).expect("save succeeds");
        let (loaded, first_run) = store.load();
        assert_eq!(loaded, s);
        assert!(!first_run);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_corrupt_file_returns_defaults_and_first_run_false() {
        // 파손 파일은 기본값(전부 OFF)으로 안전 폴백하되, 온보딩을 다시
        // 띄우지 않는다(first_run=false) — 파일이 존재했다는 사실 자체가
        // 온보딩 완료의 증거.
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, b"not json").unwrap();
        let store = SettingsStore::new(file.clone());
        let (s, first_run) = store.load();
        assert_eq!(s, AppSettings::default());
        assert!(!first_run);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn load_unknown_version_returns_defaults_and_first_run_false() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            br#"{"version":2,"claudeCliEnabled":true,"claudeHooksEnabled":true}"#,
        )
        .unwrap();
        let store = SettingsStore::new(file.clone());
        let (s, _) = store.load();
        assert_eq!(s, AppSettings::default());
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
            mascot_enabled: false,
            tts_enabled: false,
            tts_rewrite_model_anthropic: default_tts_rewrite_model_anthropic(),
            tts_rewrite_model_openrouter: default_tts_rewrite_model_openrouter(),
            tts_rewrite_provider: TtsRewriteProvider::Auto,
            web_remote_bind: Default::default(),
            web_remote_port: crate::webremote::protocol::DEFAULT_WEB_REMOTE_PORT,
            web_remote_enabled: false,
            talk_enabled: false,
            talk_max_turns: crate::talk::DEFAULT_MAX_TURNS,
            talk_idle_quiet_ms: crate::talk::DEFAULT_IDLE_QUIET_MS,
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

        fs::write(
            &file,
            br#"{"version":1,"summarizerEnabled":true,"summaryProvider":"unknown","observerEnabled":true}"#,
        )
        .unwrap();
        assert_eq!(
            SettingsStore::new(file.clone()).load().0,
            AppSettings::default()
        );
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

    // TTS(확인 요청 대사)는 외부 유료 API를 부르므로 기본 꺼짐이어야 하고,
    // 리라이트 모델은 와이어에 모델 id 문자열 그대로 실려야 한다(렌더러
    // 입력값과 Messages API `model`이 같은 문자열을 공유한다).
    #[test]
    fn tts_defaults_off_and_model_serializes_as_model_id() {
        let s = AppSettings::default();
        assert!(!s.tts_enabled);
        assert_eq!(s.tts_rewrite_model_anthropic, "claude-haiku-4-5");
        assert_eq!(s.tts_rewrite_model_openrouter, "openai/gpt-5.4-mini");
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"ttsEnabled\":false"), "{json}");
        assert!(
            json.contains("\"ttsRewriteModelAnthropic\":\"claude-haiku-4-5\""),
            "{json}"
        );
        assert!(
            json.contains("\"ttsRewriteModelOpenrouter\":\"openai/gpt-5.4-mini\""),
            "{json}"
        );
        assert_eq!(s.tts_rewrite_provider, TtsRewriteProvider::Auto);
        assert!(json.contains("\"ttsRewriteProvider\":\"auto\""), "{json}");
        for p in [
            TtsRewriteProvider::Auto,
            TtsRewriteProvider::Api,
            TtsRewriteProvider::OpenRouter,
            TtsRewriteProvider::ClaudeCli,
            TtsRewriteProvider::None,
        ] {
            assert_eq!(
                serde_json::to_value(p).unwrap(),
                serde_json::Value::String(p.as_str().to_string())
            );
        }
    }

    // 하위 호환: TTS 키가 없는 기존 settings.json도 기본값(꺼짐/Haiku)으로 로드된다.
    #[test]
    fn load_settings_without_tts_fields_falls_back_to_defaults() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, br#"{"version":1,"soundEnabled":true}"#).unwrap();
        let (s, _) = SettingsStore::new(file.clone()).load();
        assert!(!s.tts_enabled);
        assert_eq!(s.tts_rewrite_model_anthropic, "claude-haiku-4-5");
        assert_eq!(s.tts_rewrite_model_openrouter, "openai/gpt-5.4-mini");
        assert_eq!(s.tts_rewrite_provider, TtsRewriteProvider::Auto);
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    // ── UI 언어 ────────────────────────────────────────────────────────
    //
    // 언어 키가 없던 시절의 설정 파일은 `"system"`(OS 로케일 따름)이 되어야
    // 한다 — 기존 사용자가 업데이트했다고 갑자기 영어로 뜨면 안 된다.
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
    #[test]
    fn legacy_tts_rewrite_model_moves_to_the_anthropic_field() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(
            &file,
            br#"{"version":1,"ttsRewriteModel":"claude-sonnet-5"}"#,
        )
        .unwrap();
        let (s, _) = SettingsStore::new(file.clone()).load();
        assert_eq!(s.tts_rewrite_model_anthropic, "claude-sonnet-5");
        assert_eq!(
            s.tts_rewrite_model_openrouter, "openai/gpt-5.4-mini",
            "OpenRouter 쪽은 옛 값의 영향을 받지 않는다"
        );
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    #[test]
    fn new_tts_model_field_wins_over_the_legacy_key() {
        let mut v: serde_json::Value = serde_json::from_str(
            r#"{"version":1,"ttsRewriteModel":"claude-sonnet-5","ttsRewriteModelAnthropic":"claude-opus-5"}"#,
        )
        .unwrap();
        migrate_tts_rewrite_model(&mut v);
        assert_eq!(v["ttsRewriteModelAnthropic"], serde_json::json!("claude-opus-5"));
    }

    #[test]
    fn tts_model_migration_is_noop_without_or_with_blank_legacy_key() {
        let mut none: serde_json::Value = serde_json::from_str(r#"{"version":1}"#).unwrap();
        migrate_tts_rewrite_model(&mut none);
        assert_eq!(none, serde_json::json!({"version":1}), "serde 기본값에 맡긴다");

        let mut blank: serde_json::Value =
            serde_json::from_str(r#"{"version":1,"ttsRewriteModel":"  "}"#).unwrap();
        migrate_tts_rewrite_model(&mut blank);
        assert!(
            blank.get("ttsRewriteModelAnthropic").is_none(),
            "빈 모델 id를 옮기면 API가 400을 낸다"
        );
    }

    // 저장하면 옛 키는 사라지고 새 키 둘만 남는다(사운드 키와 같은 관례).
    #[test]
    fn save_drops_the_legacy_tts_model_key() {
        let file = scratch_file();
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, br#"{"version":1,"ttsRewriteModel":"claude-opus-5"}"#).unwrap();
        let store = SettingsStore::new(file.clone());
        let (s, _) = store.load();
        store.save(&s).unwrap();
        let json = fs::read_to_string(&file).unwrap();
        assert!(!json.contains("\"ttsRewriteModel\":"), "{json}");
        assert!(json.contains("\"ttsRewriteModelAnthropic\""), "{json}");
        let _ = fs::remove_dir_all(file.parent().unwrap());
    }

    // ── 요약기 모델 오버라이드 ────────────────────────────────────────
    #[test]
    fn summary_models_default_to_empty_overrides_and_round_trip() {
        let s = AppSettings::default();
        assert_eq!(s.summary_models, SummaryModels::default());
        assert_eq!(s.summary_models.for_provider(SummaryProvider::Codex).light, "");
        let json = serde_json::to_string(&s).unwrap();
        assert!(
            json.contains(r#""summaryModels":{"claude":{"light":"","heavy":""}"#),
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
}
