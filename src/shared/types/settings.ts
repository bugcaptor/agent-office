// src/shared/types/settings.ts
//
// Domain slice: app-wide opt-in settings, summarizer provider/purpose,
// and CLI control status. See src/shared/types.ts for the frozen-contract overview.

/** 라벨 요약에 사용할 로컬 CLI provider. Rust `SummaryProvider` 미러. */
export type SummaryProvider = "claude" | "codex" | "agy" | "gemini";

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
}

/** 앱 전역 opt-in 설정 — Rust `persistence::settings_store::AppSettings` 미러. */
export interface AppSettings {
  version: number;
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
}

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
