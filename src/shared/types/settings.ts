// src/shared/types/settings.ts
//
// Domain slice: app-wide opt-in settings, summarizer provider/purpose,
// and CLI control status. See src/shared/types.ts for the frozen-contract overview.

/**
 * 요약(라벨·일기·학습자료)에 사용할 provider. Rust `SummaryProvider` 미러.
 * 앞의 다섯은 로컬 CLI를 부르고, `openrouter`만 HTTP 경로다(키는 TTS와 같은
 * 백엔드 키 스토어 또는 `OPENROUTER_API_KEY` 환경변수를 쓴다).
 */
export type SummaryProvider =
  | "claude"
  | "codex"
  | "agy"
  | "gemini"
  | "opencode"
  | "openrouter";

/**
 * 설정 화면의 "모델 고르기"가 목록을 물어볼 수 있는 서비스. 요약기 provider
 * 여섯에 TTS 리라이트 전용인 `anthropic`(Anthropic API 직결)을 더한 것이다.
 * Rust `list_provider_models` 커맨드가 받는 문자열과 같아야 한다.
 */
export type ModelCatalogProvider = SummaryProvider | "anthropic";

/**
 * 요약 호출의 목적. 목적별로 백엔드 타임아웃이 다르다(#66) — 라벨(인터랙티브)은
 * 20초, 일기(백그라운드 배치)는 120초. Rust `SummaryPurpose` 미러.
 */
export type SummaryPurpose = "label" | "diary";

/** "OS 터미널로 열기"가 사용할 외부 터미널 앱 — Rust `ExternalTerminal` 미러.
 * macOS에서만 의미가 있다(다른 OS는 무시). */
export type ExternalTerminalApp = "terminal" | "iterm";

/** 셸 출력 내보내기(.txt)를 열 외부 에디터 — Rust `ExternalEditor` 미러.
 * 기본은 OS 기본 연결(system). */
export type ExternalEditorApp = "system" | "vscode";

/** 파일 목록 스캔 백엔드 — Rust `FileIndexBackend` 미러. 기본 walker. */
export type FileIndexBackend = "walker" | "everything";

/** 마스코트 신호등(docs/mascot-lights-design.md) 대상 선택 모드 —
 * Rust `MascotLightsMode` 미러. 기본 off. */
export type MascotLightsMode = "off" | "agents" | "projects";

import type { TtsRewriteProvider } from './tts';

/**
 * 요약기 provider 하나의 모델 오버라이드 — Rust `SummaryModelOverride` 미러.
 * **빈 문자열 = 오버라이드 없음**(백엔드의 목적별 기본 모델을 그대로 쓴다).
 */
export interface SummaryModelOverride {
  /** 라벨·일기(짧은 변환)에 쓸 모델 id. */
  light: string;
  /** 학습자료(긴 전사 구조화)에 쓸 모델 id. */
  heavy: string;
}

/** 요약기 provider별 모델 오버라이드 — Rust `SummaryModels` 미러. */
export interface SummaryModels {
  claude: SummaryModelOverride;
  codex: SummaryModelOverride;
  agy: SummaryModelOverride;
  gemini: SummaryModelOverride;
  /** opencode 모델 id(`provider/model` 표기 — `opencode models` 출력 형식). */
  opencode: SummaryModelOverride;
  /** OpenRouter 모델 id(`벤더/모델` 표기). */
  openrouter: SummaryModelOverride;
}

/** 앱 전역 opt-in 설정 — Rust `persistence::settings_store::AppSettings` 미러. */
export interface AppSettings {
  version: number;
  /**
   * UI 언어 — `"system"`(OS 로케일 따름, 기본) 또는 BCP47 언어 코드(`"ko"`, `"en"`).
   * 유니언이 아니라 **자유 문자열**인 게 의도다: 카탈로그 폴더만 추가하면 언어가
   * 늘어나는 구조(src/shared/i18n/catalog.ts)라, 언어를 추가할 때 이 타입 계약과
   * Rust 미러를 함께 고쳐야 하는 상황을 만들지 않는다. 카탈로그에 없는 값은
   * 런타임이 조용히 폴백한다. Rust `AppSettings::language` 미러.
   */
  language: string;
  /** 머리 위 라벨 요약용 로컬 CLI 호출 허용. */
  summarizerEnabled: boolean;
  /** 라벨 요약에 사용할 로컬 CLI provider. */
  summaryProvider: SummaryProvider;
  /** 요약기 provider별 모델 오버라이드. 빈 문자열이면 백엔드 기본 모델. */
  summaryModels: SummaryModels;
  /** 캐릭터 일기(#56) 자동 생성 허용. 요약기와 같은 provider·CLI를 쓰므로
   * 크레딧을 소모한다 → opt-in. 기본 false. */
  diaryEnabled: boolean;
  /** 세션 observer 주입 + 로컬 observer 서버 기동(알림·시간측정). */
  observerEnabled: boolean;
  /** 키보드 타건음(캐릭터가 출력을 뿜을 때). 기본 켜짐.
   * 레거시 `soundEnabled` 하나가 담당하던 것을 셋으로 쪼갠 결과 —
   * 새 키가 없는 설정 파일은 백엔드가 옛 값으로 초기화한다. */
  typingSoundEnabled: boolean;
  /** 알림 딩 + 세션 시작/종료 효과음. 기본 켜짐. */
  notifySoundEnabled: boolean;
  /** 마스터 볼륨 0.0~1.0. 기본 0.5. */
  soundVolume: number;
  /** "OS 터미널로 열기"가 사용할 터미널 앱. 기본 Terminal.app(macOS 전용). */
  externalTerminal: ExternalTerminalApp;
  /** 셸 출력 내보내기(.txt)를 열 에디터. 기본 OS 기본 연결. */
  externalEditor: ExternalEditorApp;
  /** 질문(Hook) 알림을 방출 전 보류하는 시간(ms). 그 사이 세션이 계속
   * 일하면(오토모드 자동 승인 등) 알림을 조용히 폐기한다. 0이면 즉시 알림. 기본 5000. */
  attentionHoldMs: number;
  /** "작업 폴더 보기"(이슈 #11)에서 파일별 git 상태 뱃지를 조회할지. 거대
   * 저장소에서 무거울 수 있어 끌 수 있다. 기본 true. */
  gitStatusEnabled: boolean;
  /** "작업 폴더 보기"에서 `.gitignore`로 무시된 파일·숨김(dot) 파일까지 목록에
   * 담을지. 켜면 빌드 산출물이 통째로 들어와 5000개 상한을 금방 채울 수 있어
   * 기본 false. `.git/` 내부는 켜도 항상 제외한다. */
  workdirShowIgnored: boolean;
  /** 파일 목록 백엔드. everything은 es.exe(Windows) 필요, 문서(md) 팔레트에만
   * 적용, 실패 시 자동 폴백. 기본 walker. */
  fileIndexBackend: FileIndexBackend;
  /** 로컬 CLI 제어 서버(이슈 #55) 기동 여부. 켜도 앱에서 명시적 승인이 있어야
   * 명령이 실행된다(2단계 옵트인). 보안 표면이므로 기본 false. */
  cliEnabled: boolean;
  /** 캐릭터가 작업 중일 때 시스템 유휴 잠자기를 막을지(이슈 #68). 디스플레이
   * 잠자기는 막지 않는다(화면은 꺼져도 에이전트는 계속 돈다). 기본 false. */
  keepAwakeEnabled: boolean;
  /** 터미널 세션 로그를 파일로 상시 기록할지(docs/session-log-design.md).
   * 나중에 회고하려면 그때 이미 켜져 있었어야 하므로 기본 true(opt-out).
   * 보존은 30일·2GB로 스스로 제한한다. */
  sessionLogEnabled: boolean;
  /** 데스크톱 마스코트 창(이슈 #72) — 활동 중인 캐릭터 1명을 앱 창과 별개의
   * 투명·최상단 창으로 띄운다. 화면을 상시 점유하므로 기본 false. */
  mascotEnabled: boolean;
  /** 마스코트 신호등 대상 선택 모드. `mascotEnabled`가 상위 게이트이고, 이
   * 값이 `off`가 아니면 활동이 없어도 신호등 때문에 마스코트 창이 떠 있을 수
   * 있다. 기본 "off". */
  mascotLightsMode: MascotLightsMode;
  /** 신호등 칸을 세로로 배열할지. 기본 false(가로). */
  mascotLightsVertical: boolean;
  /** 신호등 프로젝트 모드에서 칸을 받을 저장소 폴더 목록(표시 순서 = 등록
   * 순서). `mascotLightsMode==="projects"`일 때만 쓰인다. 기본 빈 배열. */
  mascotLightsProjects: string[];
  /** 알림 대사 TTS(질문·완료 알림을 캐릭터 목소리로 발화). 외부 유료 API 두 곳을
   * 호출하므로 opt-in — 기본 false. API 키는 이 구조체에 없다(백엔드 0600 파일). */
  ttsEnabled: boolean;
  /** 대사 리라이트에 쓸 Anthropic 모델 id(자유 입력). Anthropic API 경로와
   * claude CLI 경로가 함께 쓴다. 기본 "claude-haiku-4-5". */
  ttsRewriteModelAnthropic: string;
  /** OpenRouter 경로의 모델 id(`<vendor>/<model>`). 기본 "openai/gpt-5.4-mini". */
  ttsRewriteModelOpenrouter: string;
  /** 대사 리라이트 공급자. 기본 "auto"(API 키 → env → claude CLI → 생략). */
  ttsRewriteProvider: TtsRewriteProvider;
  /** 어떤 원격 주소를 받아 줄지. 기본 "tailnet"(Tailscale 대역 + 루프백). */
  webRemoteBind: WebRemoteBindPolicy;
  /** 수신 포트. 기본 47800(점유 시 백엔드가 +1씩 스캔한 실제 포트를 알려준다). */
  webRemotePort: number;
  /** 웹 원격(docs/web-remote-design.md) — 브라우저로 접속해 상태 확인·터미널
   * 조작을 하게 한다. 켜져 있을 때만 리스너가 뜨고, 페어링 승인은 여전히
   * 필요하다. 네트워크 표면이므로 기본 false. */
  webRemoteEnabled: boolean;
  /** 동료 대화(docs/agent-talk-design.md) — 캐릭터끼리 앱을 거쳐 메시지를
   * 주고받게 한다. 남의 세션에 글자를 밀어 넣는 기능이라 기본 false이고,
   * 끄면 대기 중 메시지까지 버려지는 킬 스위치다. */
  talkEnabled: boolean;
  /** 한 대화의 왕복 상한(무한 핑퐁 방지). 기본 6. */
  talkMaxTurns: number;
  /** 수신자가 이만큼 조용해야 메시지를 주입한다. 기본 3000ms. */
  talkIdleQuietMs: number;
}

/** 웹 원격 수신 서버가 받아 줄 원격 주소 범위. */
export type WebRemoteBindPolicy = "tailnet" | "all" | "loopback";

/** `get_app_settings` 응답. firstRun = settings.json 부재(첫 실행). */
export interface GetAppSettingsResult {
  settings: AppSettings;
  firstRun: boolean;
}

/** `control_status` 응답(이슈 #55) — CLI 제어의 2단계 승인 상태. */
export interface ControlStatus {
  /** 설정 cliEnabled(서버 기동 대상 여부). */
  enabled: boolean;
  /** control 서버가 실제로 떠 있는지. */
  running: boolean;
  /** 승인됨(토큰 발급됨) 여부. */
  approved: boolean;
  /** 현재 바인딩된 포트(서버가 떠 있을 때만). */
  port: number | null;
  /** 연결 안내에 쓰는 app_data 경로. */
  appDataDir: string;
}
