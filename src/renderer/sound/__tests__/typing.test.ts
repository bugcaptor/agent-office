// TypingScheduler 계약(데드라인 방식):
// - push(letters, now)는 출력량을 타이핑 시간으로 환산(typingDurationMs:
//   MIN_TYPING_MS~MAX_TYPING_MS 클램프)해 deadline = max(기존, now+시간)으로
//   연장한다 — 더 이른 데드라인으로 단축되지 않는다.
// - drain(now)은 [직전 drain, min(now, deadline)] 구간에 대해 초당
//   MIN_CLICKS_PER_SEC~MAX_CLICKS_PER_SEC 사이(주입 rng로 샘플)의 차분한
//   속도로 클릭 수를 산출한다. 몰아서 나오지 않고 시간에 걸쳐 흩어진다.
// meaningfulCount 계약: ANSI 이스케이프 제거 후 글자·숫자만 센다 — TUI
// 리페인트(스피너·테두리)와 실제 본문 스트림을 구분하는 지표.
import { describe, expect, it } from "vitest";
import {
  MAX_CLICKS_PER_SEC,
  MAX_DRAIN_WINDOW_MS,
  MAX_TYPING_MS,
  MIN_CHUNK_LETTERS,
  MIN_CLICKS_PER_SEC,
  MIN_TYPING_MS,
  TypingScheduler,
  meaningfulCount,
  typingDurationMs,
} from "../typing";

describe("meaningfulCount", () => {
  it("일반 텍스트는 글자·숫자 수를 그대로 센다(한글 포함)", () => {
    expect(meaningfulCount("abc 123")).toBe(6); // 공백 제외
    expect(meaningfulCount("한글 텍스트다")).toBe(6);
  });

  it("ANSI CSI/OSC 이스케이프는 세지 않는다", () => {
    // 색상 + 커서 이동 + OSC 타이틀 설정 — 본문 글자는 "ok" 2개뿐
    const chunk = "\x1b[31m\x1b[2;5Hok\x1b[0m\x1b]0;window title\x07";
    expect(meaningfulCount(chunk)).toBe(2);
  });

  it("테두리·스피너 글리프·구두점은 본문이 아니다", () => {
    expect(meaningfulCount("╭──────╮ ✳ · … ()│?")).toBe(0);
  });

  it("스피너 상태줄 리페인트 프레임은 MIN_CHUNK_LETTERS 미만이다", () => {
    // Claude Code 대기 화면 프레임 근사: 커서 이동 + 스피너 줄 + 입력 박스 테두리
    const frame =
      "\x1b[2K\x1b[1G✳ Deliberating… (esc to interrupt · 12s)\n" +
      "\x1b[2K╭──────────────────────────────╮\n" +
      "\x1b[2K│ >                            │\n" +
      "\x1b[2K╰──────────────────────────────╯\n" +
      "\x1b[2K  ? for shortcuts";
    expect(meaningfulCount(frame)).toBeLessThan(MIN_CHUNK_LETTERS);
  });

  it("본문이 콸콸 나오는 리페인트는 MIN_CHUNK_LETTERS를 넘는다", () => {
    const body =
      "\x1b[2K여기는 에이전트가 실제로 생성한 응답 본문이다. " +
      "function reconcileAgents(agentIds: string[]): void { for (const id of agentIds) }";
    expect(meaningfulCount(body)).toBeGreaterThanOrEqual(MIN_CHUNK_LETTERS);
  });
});

describe("typingDurationMs", () => {
  it("임계치 수준의 작은 청크는 최소 시간으로 클램프된다", () => {
    expect(typingDurationMs(MIN_CHUNK_LETTERS)).toBe(MIN_TYPING_MS);
  });

  it("거대한 청크는 최대 시간으로 클램프된다", () => {
    expect(typingDurationMs(100_000)).toBe(MAX_TYPING_MS);
  });

  it("중간 크기는 출력량에 비례하며 클램프 범위 안이다", () => {
    const mid = typingDurationMs(500);
    expect(mid).toBeGreaterThan(MIN_TYPING_MS);
    expect(mid).toBeLessThan(MAX_TYPING_MS);
    expect(typingDurationMs(700)).toBeGreaterThan(mid); // 단조 증가
  });
});

/** 100ms 틱으로 [fromMs, toMs] 구간을 drain하며 클릭 수를 합산한다. */
function drainOver(s: TypingScheduler, fromMs: number, toMs: number): number {
  let total = 0;
  for (let t = fromMs; t <= toMs; t += 100) total += s.drain(t);
  return total;
}

describe("TypingScheduler", () => {
  const midRng = () => 0.5; // 고정 rng — 초당 (MIN+MAX)/2 속도

  it("임계 청크 하나로 최소 시간 동안 차분히 타이핑하고 멈춘다", () => {
    const s = new TypingScheduler(0, midRng);
    s.push(MIN_CHUNK_LETTERS, 0);
    const rate = (MIN_CLICKS_PER_SEC + MAX_CLICKS_PER_SEC) / 2;
    const expected = (MIN_TYPING_MS / 1000) * rate;
    const total = drainOver(s, 100, MIN_TYPING_MS);
    expect(total).toBeGreaterThanOrEqual(Math.floor(expected) - 1); // floor 손실 허용
    expect(total).toBeLessThanOrEqual(Math.ceil(expected));
    // 데드라인 이후엔 조용
    expect(drainOver(s, MIN_TYPING_MS + 100, MIN_TYPING_MS + 2000)).toBe(0);
  });

  it("클릭이 한꺼번에 나오지 않고 구간 전체에 흩어진다", () => {
    const s = new TypingScheduler(0, midRng);
    s.push(100_000, 0); // 최대 시간(10초)짜리 대량 출력
    // 첫 1초 동안은 최대 초당 속도 이하만 나온다 — "부왁" 금지
    expect(drainOver(s, 100, 1000)).toBeLessThanOrEqual(MAX_CLICKS_PER_SEC);
    // 마지막 1초에도 여전히 타이핑 중이다
    expect(drainOver(s, MAX_TYPING_MS - 900, MAX_TYPING_MS)).toBeGreaterThan(0);
    // 데드라인 이후엔 조용
    expect(drainOver(s, MAX_TYPING_MS + 100, MAX_TYPING_MS + 2000)).toBe(0);
  });

  it("타건 속도는 rng 극단에서도 MIN~MAX clicks/sec 범위를 지킨다", () => {
    for (const [rngVal, rate] of [
      [0, MIN_CLICKS_PER_SEC],
      [1, MAX_CLICKS_PER_SEC],
    ] as const) {
      const s = new TypingScheduler(0, () => rngVal);
      s.push(MIN_CHUNK_LETTERS, 0);
      const expected = (MIN_TYPING_MS / 1000) * rate;
      const total = drainOver(s, 100, MIN_TYPING_MS);
      expect(total).toBeGreaterThanOrEqual(Math.floor(expected) - 1); // floor 손실 허용
      expect(total).toBeLessThanOrEqual(Math.ceil(expected));
    }
  });

  it("재생 중 새 출력이 오면 데드라인이 더 늦은 쪽으로 연장된다", () => {
    const s = new TypingScheduler(0, midRng);
    s.push(MIN_CHUNK_LETTERS, 0); // 데드라인 = MIN_TYPING_MS
    drainOver(s, 100, 1500);
    s.push(MIN_CHUNK_LETTERS, 1500); // 데드라인 = 1500 + MIN_TYPING_MS
    // 원래 데드라인을 지나서도 타이핑이 이어진다
    expect(drainOver(s, 1600, 1500 + MIN_TYPING_MS)).toBeGreaterThan(0);
    expect(drainOver(s, 1500 + MIN_TYPING_MS + 100, 1500 + MIN_TYPING_MS + 2000)).toBe(0);
  });

  it("나중 출력이 계산한 데드라인이 더 이르면 기존 데드라인을 단축하지 않는다", () => {
    const s = new TypingScheduler(0, midRng);
    s.push(100_000, 0); // 데드라인 = MAX_TYPING_MS(10초)
    s.push(MIN_CHUNK_LETTERS, 100); // 후보 2.1초 — 무시되어야 한다
    expect(drainOver(s, 200, 3000)).toBeGreaterThan(0);
    // 5초대에도 여전히 타이핑 중
    expect(drainOver(s, 5000, 6000)).toBeGreaterThan(0);
  });

  it("틱이 오래 멈췄다 재개돼도(백그라운드 스로틀) 밀린 클릭을 몰아 내지 않는다", () => {
    const s = new TypingScheduler(0, () => 1); // 최대 속도
    s.push(100_000, 0); // 데드라인 = MAX_TYPING_MS
    // 5초간 drain이 한 번도 안 불리다 재개 — 한 번의 drain은 짧은 창 분량만 낸다
    expect(s.drain(5000)).toBeLessThanOrEqual(
      Math.ceil((MAX_DRAIN_WINDOW_MS / 1000) * MAX_CLICKS_PER_SEC)
    );
  });

  it("시간이 거꾸로 가도(now 역행) 예외 없이 0 이상을 반환한다", () => {
    const s = new TypingScheduler(1000, midRng);
    s.push(MIN_CHUNK_LETTERS, 1000);
    expect(s.drain(500)).toBeGreaterThanOrEqual(0);
  });
});
