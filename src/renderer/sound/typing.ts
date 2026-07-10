// src/renderer/sound/typing.ts
//
// PTY 출력 청크 → 키 클릭 타이밍 산출. 오디오와 무관한 순수 로직.
//
// 2단 구조:
// 1) meaningfulCount — 청크에서 ANSI 이스케이프를 걷어내고 글자·숫자만
//    센다. Claude Code 같은 TUI는 대기 중에도 스피너·상태줄을 계속 다시
//    그리는데(커서 이동+테두리+짧은 문구), 그 잡음이 소리를 내면 안 된다.
//    호출측은 이 값이 MIN_CHUNK_LETTERS 이상인 "본문다운" 청크만 push한다.
// 2) TypingScheduler — 토큰 버킷. 유효 글자 수를 "타이핑 에너지"(클릭 수)로
//    누적하되 캡을 둬서, 출력이 콸콸 나와도 사람 타속(초당
//    MAX_CLICKS_PER_SEC)을 유지하고 끊기면 잔여 에너지를 소진한 뒤 멈춘다.

/** 초당 최대 클릭 수 — 사람이 빠르게 치는 속도 근처. */
export const MAX_CLICKS_PER_SEC = 11;
/** 유효 글자 이 개수 ≈ 1타. 출력량 → 타이핑량 환산 계수. */
export const BYTES_PER_CLICK = 6;
/**
 * 누적 에너지 상한(클릭 수). 폭주 청크가 와도 출력 중단 후 잔여 타이핑이
 * 이 이상 이어지지 않는다(~0.7초).
 */
export const MAX_ENERGY_CLICKS = 8;
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

export class TypingScheduler {
  private energy = 0; // 남은 클릭 수(소수 허용)
  private budgetCarry = 0; // drain 예산의 소수 이월분
  private lastDrainAt: number;

  constructor(nowMs: number) {
    this.lastDrainAt = nowMs;
  }

  /** 본문 인정된 청크의 유효 글자 수를 에너지로 누적(캡 적용). */
  push(letterCount: number): void {
    this.energy = Math.min(this.energy + letterCount / BYTES_PER_CLICK, MAX_ENERGY_CLICKS);
  }

  /** 직전 drain 이후 재생할 클릭 수. 호출 간격과 무관하게 초당 상한 유지. */
  drain(nowMs: number): number {
    const elapsed = Math.max(0, nowMs - this.lastDrainAt);
    this.lastDrainAt = nowMs;
    // 이월 포함 예산은 1초치로 캡 — 오래 idle 후 몰아치기 방지.
    this.budgetCarry = Math.min(
      this.budgetCarry + (elapsed / 1000) * MAX_CLICKS_PER_SEC,
      MAX_CLICKS_PER_SEC
    );
    const n = Math.floor(Math.min(this.energy, this.budgetCarry));
    this.energy -= n;
    this.budgetCarry -= n;
    return n;
  }
}
