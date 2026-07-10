// src/renderer/sound/synth.ts
//
// 소리 재료 합성 — Web Audio 노드 조립. 타이핑은 녹음 샘플(backend.ts)이
// 주 경로고, 여기의 playKeyClick은 샘플 로드 실패/지연 시 폴백이다.
// 각 함수는 (ctx, dest, at)에 일회성 노드를 만들어 재생 후 자동 정리된다
// (BufferSource/Oscillator는 stop 후 GC 대상).

function hashId(agentId: string): number {
  let h = 0;
  for (let i = 0; i < agentId.length; i++) h = (h * 31 + agentId.charCodeAt(i)) >>> 0;
  return h % 1000;
}

/** agentId → 키 클릭 필터 주파수 배율(0.85~1.2). 폴백 합성 클릭의 음색. */
export function agentFreqMul(agentId: string): number {
  return 0.85 + (hashId(agentId) / 1000) * 0.35;
}

/**
 * agentId → 샘플 재생 속도 배율(0.9~1.15). 같은 샘플 팩이라도 에이전트마다
 * 피치가 달라져 자기 키보드 소리처럼 들린다. 범위를 좁게 잡은 이유:
 * 재생 속도 변화는 피치와 길이를 함께 바꿔서, 이보다 넓으면 부자연스럽다.
 */
export function agentRateMul(agentId: string): number {
  return 0.9 + (hashId(agentId) / 1000) * 0.25;
}

/**
 * 기계식 키 클릭 1타: 화이트노이즈 40ms → 밴드패스(2.6~4.4kHz 랜덤 × 배율)
 * → 급감쇠. thock=스페이스바 느낌의 저역(900Hz) 타건.
 */
export function playKeyClick(
  ctx: AudioContext,
  dest: AudioNode,
  at: number,
  freqMul: number,
  thock: boolean
): void {
  const dur = 0.04;
  const buf = ctx.createBuffer(1, Math.ceil(ctx.sampleRate * dur), ctx.sampleRate);
  const ch = buf.getChannelData(0);
  for (let i = 0; i < ch.length; i++) ch[i] = Math.random() * 2 - 1;
  const src = ctx.createBufferSource();
  src.buffer = buf;
  const bp = ctx.createBiquadFilter();
  bp.type = "bandpass";
  bp.frequency.value = (thock ? 900 : 2600 + Math.random() * 1800) * freqMul;
  bp.Q.value = thock ? 2 : 1.2;
  const g = ctx.createGain();
  const peak = (thock ? 0.5 : 0.35) * (0.8 + Math.random() * 0.4); // 타마다 강약 지터
  g.gain.setValueAtTime(peak, at);
  g.gain.exponentialRampToValueAtTime(0.001, at + dur);
  src.connect(bp).connect(g).connect(dest);
  src.start(at);
  src.stop(at + dur);
}

/** 알림 "딩" — 사인 2음 하모닉(880/1320Hz), ~300ms 감쇠. */
export function playDing(ctx: AudioContext, dest: AudioNode, at: number): void {
  for (const [freq, peak] of [
    [880, 0.18],
    [1320, 0.08],
  ] as const) {
    const osc = ctx.createOscillator();
    osc.type = "sine";
    osc.frequency.value = freq;
    const g = ctx.createGain();
    g.gain.setValueAtTime(peak, at);
    g.gain.exponentialRampToValueAtTime(0.001, at + 0.3);
    osc.connect(g).connect(dest);
    osc.start(at);
    osc.stop(at + 0.3);
  }
}

/** 세션 시작(상승 톤 330→660Hz)·종료(하강 톤 660→330Hz) 공용. */
export function playSweep(
  ctx: AudioContext,
  dest: AudioNode,
  at: number,
  fromHz: number,
  toHz: number
): void {
  const dur = 0.18;
  const osc = ctx.createOscillator();
  osc.type = "triangle";
  osc.frequency.setValueAtTime(fromHz, at);
  osc.frequency.exponentialRampToValueAtTime(toHz, at + dur);
  const g = ctx.createGain();
  g.gain.setValueAtTime(0.12, at);
  g.gain.exponentialRampToValueAtTime(0.001, at + dur);
  osc.connect(g).connect(dest);
  osc.start(at);
  osc.stop(at + dur);
}
