// src/shared/types/profile.ts
//
// Domain slice: persisted character profile / app state and sprite
// generation results. See src/shared/types.ts for the frozen-contract overview.

import type { BotConfig } from './bot';

/**
 * 스프라이트 팔레트에서 사용자가 직접 고를 수 있는 색 슬롯. 아키타입마다
 * 부르는 이름은 달라도(머리/털, 피부/장갑/본체, 옷/포인트) 실제 램프는 이 셋뿐이다.
 */
export type PaletteSlot = 'skin' | 'hair' | 'shirt';

/**
 * 슬롯별 색 오버라이드("#rrggbb"). 시드에서 뽑힌 기본색 대신 이 색을 쓴다 —
 * 시드를 바꾸지 않고(=스프라이트 재생성 없이) 색만 갈아 끼우는 수단이다.
 * 부재/빈 객체 = 전부 시드 기본값. Rust `colors` 미러.
 */
export type ColorOverrides = Partial<Record<PaletteSlot, string>>;

/**
 * Agent profile (single definition). Mirrors Rust `AgentProfile`.
 */
export interface AgentProfile {
  /** nanoid */
  id: string;
  name: string;
  role: string;
  /** Sprite seed. */
  seed: string;
  createdAt: number;
  /** Reference only; actual seating is decided by B's assignDesks (deterministic hash). */
  deskIndex: number;
  /** 사용자가 책상 클릭으로 수동 지정한 책상 인덱스. 부재 = 자동(해시) 배정.
   * 지정된 책상은 자동 배정 풀에서 제외된다(주인 전용). */
  assignedDeskIndex?: number;
  /** Session working directory. Absent/undefined = backend falls back to the home dir. */
  cwd?: string;
  /** 셸 id(예: "pwsh", "git-bash", "wsl", "powershell"). 부재 = 자동/기본 셸. */
  shell?: string;
  /** 새 세션이 뜰 때마다 셸 stdin에 주입할 시작 명령어. 부재/공백 = 미주입.
   * 예: "source ./init.sh", "mysetup.bat". 셸 문법은 사용자 책임. */
  startupCommand?: string;
  /** Claude Code 세션에 추가 시스템 프롬프트로 주입할 캐릭터 성격(멀티라인 가능). */
  personalityPrompt?: string;
  /** 초상화 추가 프롬프트(자유 텍스트). 초상 이미지 프롬프트에만 덧붙는다. */
  portraitRequest?: string;
  /** 초상 존재 표시 + 프론트 캐시 무효화 키(epoch ms). undefined = 초상 없음. */
  portraitUpdatedAt?: number;
  /** 스프라이트 추가 프롬프트(자유 텍스트). 스프라이트 프롬프트에만 덧붙는다
   * — 초상화 쪽으로 폴백하지 않는다(레거시 `appearance` 값은 로드 시 양쪽에 복사). */
  spriteRequest?: string;
  /** 커스텀 스프라이트 존재 표시 + 프론트 캐시 무효화 키(epoch ms). undefined = 절차 생성 사용. */
  spriteUpdatedAt?: number;
  /** 미니미(소환수) 추가 프롬프트(자유 텍스트). 비면 프롬프트가
   * "본체에 어울리는 소환수를 알아서 만들어 달라"는 문장으로 자동 폴백한다. */
  minimiRequest?: string;
  /** 서브에이전트 미니미용 커스텀 픽셀아트 존재 표시 + 프론트 캐시 무효화 키(epoch ms).
   * undefined = 부모 스프라이트 idle0 축소판을 그대로 사용(현행 기본). */
  minimiUpdatedAt?: number;
  /** 캐릭터 아키타입(종족) id. 부재/알 수 없음 = "human" 폴백, "auto" = 시드 추첨(저장 시 확정). */
  archetype?: string;
  /** 팔레트 슬롯별 색 오버라이드. 부재/빈 객체 = 시드에서 뽑힌 기본색 그대로.
   * 스프라이트·초상 프롬프트·분석 대표색이 모두 이 색을 따른다. */
  colors?: ColorOverrides;
  /** 퇴근(clock-out) 상태. true면 오피스/터미널에서 사라지고 소환 목록에만 남는다.
   * 부재/false = 근무 중. 되돌릴 수 있는 상태이며 프로필 자체는 보존된다. */
  clockedOut?: boolean;
  /** 키보드 사운드 팩 id (sound/packs.ts). 부재/무효 = 기본 팩. */
  keyboardSound?: string;
  /** 대사 TTS에 쓸 ElevenLabs voice_id 수동 지정. 부재/빈 값 = archetype 기반
   * 자동 캐스팅. 계정 목록에 없는 id면 백엔드가 자동 배정으로 강등한다. */
  voiceId?: string;
  /** 봇 모드 설정(이슈 #57). 부재 = 기본값. 봇 ON/OFF 자체는 런타임 상태이고
   * 여기엔 지속 설정(slug 별칭·화이트리스트·폴링 주기)만 담는다. Rust `bot` 미러. */
  bot?: BotConfig;
  /** 동료 대화(docs/agent-talk-design.md) 수신 허용. 부재/true = 받는다,
   * false = 이 캐릭터에게는 말을 걸 수 없다(roster에서 reachable=false). */
  talkReceive?: boolean;
  /** 이 캐릭터의 세션을 tmux로 자동 호스팅. true면 새 세션을 tmux 세션으로
   * 띄우고 앱 PTY는 거기 attach만 한다 — 켜면 프로필의 셸 선택은 무의미해진다
   * (pane 셸은 tmux/시스템 기본이 정한다). **유닉스 전용**, Windows에서는 무시.
   * 부재/false = 기존처럼 직접 셸을 띄운다. */
  tmuxHost?: boolean;
}

/**
 * Persisted app state. Mirrors Rust `PersistedState`.
 * Stored by Rust `ProfileStore` (profiles.json in Tauri app data dir);
 * sessionId is never persisted (runtime-only).
 */
export interface PersistedState {
  agents: AgentProfile[];
  version: 1;
  /** 휴가 모드(보스 책상). 부재 = false. */
  vacationMode?: boolean;
}

/** 로컬 codex CLI 설치 탐지 결과 (Rust codex_imagegen::CodexImageStatus 미러). */
export interface CodexImageStatus {
  available: boolean;
  /** `codex --version` 첫 줄. 미설치면 없음. */
  version?: string;
}

/** codex 이미지 생성 결과 (Rust codex_imagegen::GeneratedCodexImage 미러). */
export interface GeneratedCodexImage {
  /** PNG 바이트의 base64(데이터 URL 접두사 없음). */
  pngBase64: string;
}
