// src/renderer/sound/backend.ts
//
// Web Audio 그래프 조립: 모든 소리 → bus(마스터 게인) → 컴프레서(리미터)
// → 출력. 동시 작업 에이전트가 많아도 컴프레서가 과대음량을 억제한다.
// AudioContext 생성 실패 시 null — 사운드는 장식이므로 앱은 계속 동작.
//
// 타이핑은 녹음 샘플이 주 경로 — samples/<팩id>/*.wav 디렉터리 하나가
// 키보드 사운드 팩 하나다(각 팩의 출처·라이선스는 팩 안의 LICENSE.md).
// 샘플은 비동기 로드되며, 요청 팩이 로드 전/실패면 기본 팩 → 그것도 없으면
// synth.ts의 합성 클릭으로 폴백한다(pickPackSamples). 에이전트별 개성은
// 재생 속도 배율(agentRateMul)로 낸다.
//
// 자동재생 정책 폴백: WKWebView가 사용자 제스처 전 컨텍스트를 suspended로
// 둘 수 있어, 생성 직후 resume을 시도하고 실패 시 첫 입력에서 1회 재시도.
import { agentFreqMul, agentRateMul, playDing, playKeyClick, playSweep } from "./synth";
import { PACK_SAMPLE_URLS, packGain, pickPackSamples } from "./packs";

export interface SoundBackend {
  /** count개의 키 클릭을 직후 ~90ms 창에 지터로 흩어 재생. 음색은 agentId 해시.
   * packId는 키보드 사운드 팩 선택 — 부재/무효면 기본 팩(resolvePackId). */
  playClicks(agentId: string, count: number, packId?: string): void;
  playDing(): void;
  playSessionStart(): void;
  playSessionEnd(): void;
  setVolume(v: number): void;
  dispose(): void;
}

const THOCK_CHANCE = 0.12; // 폴백 합성 클릭용: 약 8타에 1번 저역 타건

/** createWebAudioBackend 의존성 — 기본은 실제 전역. 테스트는 여기로 주입. */
export interface WebAudioDeps {
  /** 기본 new AudioContext(). */
  context?: AudioContext;
  /** resume 재시도 이벤트(pointerdown/keydown/visibilitychange) 부착 대상. 기본 window. */
  resumeTarget?: EventTarget | null;
  /** 출력장치 변경(devicechange) 감지. 기본 navigator.mediaDevices(없으면 null). null이면 비활성. */
  mediaDevices?: EventTarget | null;
}

// pointerdown/keydown = 자동재생 정책 해제(첫 제스처), visibilitychange =
// 포그라운드 복귀. 셋 다 "죽어 있으면 재개"로만 쓰이므로 반복 부착돼도 무해.
const RESUME_EVENTS = ["pointerdown", "keydown", "visibilitychange"] as const;

/** AudioContext 생성 실패 시 null — 호출측은 사운드 전체를 조용히 비활성. */
export function createWebAudioBackend(deps: WebAudioDeps = {}): SoundBackend | null {
  let ctx: AudioContext;
  try {
    ctx = deps.context ?? new AudioContext();
  } catch (err) {
    console.warn("sound: AudioContext 생성 실패 — 사운드 비활성", err);
    return null;
  }
  const resumeTarget =
    deps.resumeTarget !== undefined
      ? deps.resumeTarget
      : typeof window !== "undefined"
        ? window
        : null;
  const mediaDevices =
    deps.mediaDevices !== undefined
      ? deps.mediaDevices
      : typeof navigator !== "undefined"
        ? (navigator.mediaDevices ?? null)
        : null;

  const compressor = ctx.createDynamicsCompressor();
  compressor.threshold.value = -18;
  compressor.knee.value = 20;
  compressor.ratio.value = 8;
  compressor.attack.value = 0.003;
  compressor.release.value = 0.25;
  compressor.connect(ctx.destination);

  const bus = ctx.createGain();
  bus.gain.value = 0.25; // setVolume(0.5)^2 과 동일한 초기값
  bus.connect(compressor);

  // 상시 복구 — WKWebView는 장시간 유휴·OS 오디오 인터럽트·출력장치 변경 시
  // 컨텍스트를 suspended/interrupted로 돌린다. 부팅 1회성 resume만 있으면 한 번
  // 죽은 뒤로 세션 내내 무음이 되므로, 상태 전이·장치 변경·사용자 입력·재생
  // 시점마다 재개를 시도한다. closed는 되살릴 수 없고 running이면 no-op이라
  // 반복 호출은 안전하다(재생 진입점에서도 부르지만 running이면 그냥 통과).
  const ensureRunning = () => {
    if (ctx.state !== "running" && ctx.state !== "closed") void ctx.resume();
  };
  ctx.addEventListener("statechange", ensureRunning);
  for (const ev of RESUME_EVENTS) resumeTarget?.addEventListener(ev, ensureRunning);
  mediaDevices?.addEventListener("devicechange", ensureRunning);
  ensureRunning(); // 자동재생 정책으로 suspended 생성된 경우 즉시 1회 시도

  // 키프레스 샘플 비동기 로드 — 팩별로 독립. 개별 실패는 건너뛰고(그만큼
  // 변주가 줄 뿐), 팩 전체가 실패하면 그 팩은 빈 채로 남아 재생 시
  // pickPackSamples가 기본 팩/합성으로 폴백한다. gain은 팩 간 녹음 레벨
  // 보정(packGain)을 디코드 시점에 엔트리로 박아둔 것.
  type KeySample = { buf: AudioBuffer; gain: number };
  const keySamplesByPack = new Map<string, KeySample[]>();
  for (const [packId, urls] of PACK_SAMPLE_URLS) {
    void Promise.all(
      urls.map(async (url) => {
        try {
          const res = await fetch(url);
          return await ctx.decodeAudioData(await res.arrayBuffer());
        } catch {
          return null;
        }
      })
    ).then((decoded) => {
      const gain = packGain(packId);
      const samples = decoded
        .filter((b): b is AudioBuffer => b !== null)
        .map((buf) => ({ buf, gain }));
      if (samples.length === 0) {
        console.warn(`sound: 키프레스 샘플 로드 실패 (팩 ${packId}) — 폴백 사용`);
        return;
      }
      keySamplesByPack.set(packId, samples);
    });
  }

  return {
    playClicks(agentId, count, packId) {
      ensureRunning();
      const t0 = ctx.currentTime;
      const samples = pickPackSamples(keySamplesByPack, packId);
      for (let i = 0; i < count; i++) {
        const at = t0 + Math.random() * 0.09;
        if (samples) {
          const { buf, gain } = samples[Math.floor(Math.random() * samples.length)];
          const src = ctx.createBufferSource();
          src.buffer = buf;
          // 에이전트 고유 피치 × 타마다 미세 지터 — 기계적 반복감 제거.
          src.playbackRate.value = agentRateMul(agentId) * (0.96 + Math.random() * 0.08);
          const g = ctx.createGain();
          g.gain.value = (0.55 + Math.random() * 0.35) * gain; // 강약 지터 × 팩 레벨 보정
          src.connect(g).connect(bus);
          src.start(at);
        } else {
          playKeyClick(ctx, bus, at, agentFreqMul(agentId), Math.random() < THOCK_CHANCE);
        }
      }
    },
    playDing: () => {
      ensureRunning();
      playDing(ctx, bus, ctx.currentTime);
    },
    playSessionStart: () => {
      ensureRunning();
      playSweep(ctx, bus, ctx.currentTime, 330, 660);
    },
    playSessionEnd: () => {
      ensureRunning();
      playSweep(ctx, bus, ctx.currentTime, 660, 330);
    },
    setVolume(v) {
      bus.gain.setTargetAtTime(v * v, ctx.currentTime, 0.05); // 청감 보정(제곱)
    },
    dispose() {
      ctx.removeEventListener("statechange", ensureRunning);
      for (const ev of RESUME_EVENTS) resumeTarget?.removeEventListener(ev, ensureRunning);
      mediaDevices?.removeEventListener("devicechange", ensureRunning);
      void ctx.close();
    },
  };
}
