// src/renderer/sound/typing.ts
//
// PTY 출력 청크 → 키 클릭 타이밍 산출. 오디오와 무관한 순수 로직.
//
// 2단 구조:
// 1) meaningfulCount — 청크에서 ANSI 이스케이프를 걷어내고 글자·숫자만
//    센다. Claude Code 같은 TUI는 대기 중에도 스피너·상태줄을 계속 다시
//    그리는데(커서 이동+테두리+짧은 문구), 그 잡음이 소리를 내면 안 된다.
//    호출측은 이 값이 MIN_CHUNK_LETTERS 이상인 "본문다운" 청크만 push한다.
// 2) TypingScheduler — 데드라인 방식. 청크의 유효 글자 수를 타이핑 시간
//    (MIN_TYPING_MS~MAX_TYPING_MS)으로 환산해 deadline = max(기존, now+시간)
//    으로 연장하고, 데드라인까지 사람이 차분히 치는 속도(초당
//    MIN~MAX_CLICKS_PER_SEC 사이를 틱마다 랜덤 샘플)로 클릭을 흘려보낸다.
//    출력이 이어지면 데드라인만 뒤로 밀리므로 실제 타이핑처럼 끊김 없이
//    이어지고, 출력이 멈추면 남은 시간을 치고 조용해진다.

/** 타건 속도 하한(초당 클릭). 틱마다 이 범위에서 랜덤 샘플. */
export const MIN_CLICKS_PER_SEC = 3;
/** 타건 속도 상한(초당 클릭) — 차분한 사람 타속. */
export const MAX_CLICKS_PER_SEC = 8;
/** 임계 통과 청크 하나가 보장하는 최소 타이핑 시간. */
export const MIN_TYPING_MS = 2000;
/** 청크 하나가 늘릴 수 있는 최대 타이핑 시간. */
export const MAX_TYPING_MS = 10_000;
/** 유효 글자 1개 ≈ 이 시간만큼 타이핑. 출력량 → 시간 환산 계수. */
export const TYPING_MS_PER_LETTER = 12;
/**
 * drain 한 번이 정산하는 활성 시간 상한. 백그라운드 스로틀 등으로 틱이
 * 오래 멈췄다 재개돼도 밀린 클릭을 몰아 내지 않는다 — 지나간 타이핑은
 * 그냥 잃어버린 것으로 친다.
 */
export const MAX_DRAIN_WINDOW_MS = 300;
/**
 * 청크 하나가 "본문 출력"으로 인정받기 위한 최소 유효 글자 수.
 * TUI 스피너/상태줄 리페인트는 프레임당 유효 글자가 수십 개 수준이라
 * 여기서 걸러지고, 실제 텍스트가 많이 흐를 때(리페인트가 커질 때)만
 * 소리가 난다. 사용자 키 입력 에코(청크당 1~2글자)도 자연히 제외.
 */
export const MIN_CHUNK_LETTERS = 64;

// CSI(\x1b[...文字) · OSC(\x1b]...BEL/ST) · 기타 2바이트 이스케이프.
// eslint-disable-next-line no-control-regex
const ANSI_RE = /\x1b\[[0-9;?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-_]/g;

/**
 * ANSI 이스케이프를 제거한 뒤 유니코드 글자·숫자만 센다. 테두리(박스
 * 드로잉)·스피너 글리프·공백·구두점은 "본문"이 아니므로 제외 — TUI
 * 리페인트와 실제 텍스트 스트림을 구분하는 지표.
 */
export function meaningfulCount(data: string): number {
  const stripped = data.replace(ANSI_RE, "");
  let n = 0;
  for (const ch of stripped) if (/[\p{L}\p{N}]/u.test(ch)) n++;
  return n;
}

/** 유효 글자 수 → 타이핑 시간(ms). MIN~MAX_TYPING_MS로 클램프. */
export function typingDurationMs(letterCount: number): number {
  return Math.min(Math.max(letterCount * TYPING_MS_PER_LETTER, MIN_TYPING_MS), MAX_TYPING_MS);
}

export class TypingScheduler {
  private deadline = 0; // 이 시각까지 타이핑
  private clickCarry = 0; // 클릭 수의 소수 이월분
  private lastDrainAt: number;

  constructor(
    nowMs: number,
    private rng: () => number = Math.random
  ) {
    this.lastDrainAt = nowMs;
  }

  /** 본문 인정된 청크의 출력량만큼 데드라인을 연장(단축은 없음). */
  push(letterCount: number, nowMs: number): void {
    this.deadline = Math.max(this.deadline, nowMs + typingDurationMs(letterCount));
  }

  /**
   * 직전 drain 이후 재생할 클릭 수. 활성 구간(데드라인 이전)에 대해서만
   * 틱마다 샘플한 속도로 산출 — 호출 간격과 무관하게 속도 범위 유지.
   */
  drain(nowMs: number): number {
    const activeMs = Math.min(
      Math.max(0, Math.min(nowMs, this.deadline) - this.lastDrainAt),
      MAX_DRAIN_WINDOW_MS
    );
    this.lastDrainAt = Math.max(this.lastDrainAt, nowMs);
    if (activeMs === 0) return 0;
    const rate = MIN_CLICKS_PER_SEC + this.rng() * (MAX_CLICKS_PER_SEC - MIN_CLICKS_PER_SEC);
    const clicks = this.clickCarry + (activeMs / 1000) * rate;
    const n = Math.floor(clicks);
    this.clickCarry = clicks - n;
    return n;
  }
}
