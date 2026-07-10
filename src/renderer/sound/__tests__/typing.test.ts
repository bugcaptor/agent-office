// TypingScheduler 계약:
// - push(letters)로 "타이핑 에너지"(클릭 수 환산)를 누적하되 MAX_ENERGY_CLICKS로 캡
//   → 폭주 청크가 와도 출력 중단 후 잔여 타이핑이 짧게 끝난다.
// - drain(now)은 직전 drain 이후 경과 시간만큼의 예산(MAX_CLICKS_PER_SEC)과
//   에너지 중 작은 쪽을 정수로 반환. 예산의 소수 잔여분은 이월(carry).
// meaningfulCount 계약: ANSI 이스케이프 제거 후 글자·숫자만 센다 — TUI
// 리페인트(스피너·테두리)와 실제 본문 스트림을 구분하는 지표.
import { describe, expect, it } from "vitest";
import {
  BYTES_PER_CLICK,
  MAX_CLICKS_PER_SEC,
  MAX_ENERGY_CLICKS,
  MIN_CHUNK_LETTERS,
  TypingScheduler,
  meaningfulCount,
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

describe("TypingScheduler", () => {
  it("대량 출력이 계속 들어와도 초당 클릭 상한을 넘지 않는다", () => {
    const s = new TypingScheduler(0);
    let total = 0;
    // 1초 동안 100ms마다: 대형 청크 push + drain
    for (let t = 100; t <= 1000; t += 100) {
      s.push(10_000);
      total += s.drain(t);
    }
    expect(total).toBeLessThanOrEqual(MAX_CLICKS_PER_SEC);
    expect(total).toBeGreaterThanOrEqual(MAX_CLICKS_PER_SEC - 1); // floor 손실 1 이내
  });

  it("출력이 멈추면 잔여 에너지(캡 이하)만 소진하고 0으로 수렴한다", () => {
    const s = new TypingScheduler(0);
    s.push(100_000); // 에너지는 MAX_ENERGY_CLICKS로 캡
    let total = 0;
    for (let t = 100; t <= 3000; t += 100) total += s.drain(t);
    expect(total).toBe(MAX_ENERGY_CLICKS);
    expect(s.drain(3100)).toBe(0); // 이후엔 조용
  });

  it("작은 청크는 바이트 수에 비례한 클릭을 낸다", () => {
    const s = new TypingScheduler(0);
    s.push(BYTES_PER_CLICK * 3); // 정확히 3타 분량
    // 충분한 시간(1초) 뒤 drain — 예산은 넉넉, 에너지가 바닥
    expect(s.drain(1000)).toBe(3);
    expect(s.drain(2000)).toBe(0);
  });

  it("drain 예산 이월은 1초치로 캡된다(오래 idle 후 몰아치기 방지)", () => {
    const s = new TypingScheduler(0);
    // 10초 idle 후 대량 push — 첫 drain이 10초치 예산을 쓰면 안 된다
    s.push(100_000);
    expect(s.drain(10_000)).toBeLessThanOrEqual(MAX_ENERGY_CLICKS);
    // 캡(8) < 1초 예산(11)이라 에너지가 바닥나는 게 정상
    expect(s.drain(10_100)).toBe(0);
  });

  it("시간이 거꾸로 가도(now 역행) 예외 없이 0 이상을 반환한다", () => {
    const s = new TypingScheduler(1000);
    s.push(60);
    expect(s.drain(500)).toBeGreaterThanOrEqual(0);
  });
});
