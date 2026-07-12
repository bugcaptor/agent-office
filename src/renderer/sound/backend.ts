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
// 유휴 후 무음 문제(WKWebView): 장시간 유휴·OS 오디오 인터럽트·디스플레이
// 슬립·출력장치 변경 시 컨텍스트가 suspended/interrupted로 죽는다. WebKit은
// "사용자 제스처 없이" 부른 resume()을 신뢰성 있게 이행하지 않는데, 이 앱의
// 타이핑 소리는 에이전트 출력(=비제스처)에 재생되므로 resume만으로는 못
// 살아난다. 그래서 2단 방어:
//  1) keep-alive — 무음 루프를 상시 재생해 오디오 세션이 유휴로 정리되는
//     것을 예방(가장 흔한 유휴 케이스 차단).
//  2) 재생성(rebuild) — 재생 시점에 resume을 시도하되(타임아웃 포함) 그래도
//     running이 안 되면 컨텍스트를 통째로 다시 만든다. AudioBuffer는 컨텍스트에
//     묶이지 않아 재로드 없이 재사용하므로 비용이 사실상 없다. 볼륨은 상태로
//     보존하고, 재생성은 쿨다운으로 디바운스해 폭주를 막는다.
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
  /** AudioContext 팩토리 — 기본 () => new AudioContext(). 재생성(rebuild)이
   * 매번 이 팩토리로 새 컨텍스트를 얻으므로, 테스트는 여기로 컨텍스트 열을 준다. */
  createContext?: () => AudioContext;
  /** resume 재시도 이벤트(pointerdown/keydown/visibilitychange) 부착 대상. 기본 window. */
  resumeTarget?: EventTarget | null;
  /** 출력장치 변경(devicechange) 감지. 기본 navigator.mediaDevices(없으면 null). null이면 비활성. */
  mediaDevices?: EventTarget | null;
  /** 재생성 쿨다운 판정용 시계. 기본 () => Date.now(). */
  now?: () => number;
  /** resume 타임아웃 구현. 기본 setTimeout 기반. 테스트에서 즉시 만료로 주입. */
  delay?: (ms: number) => Promise<void>;
}

// pointerdown/keydown = 자동재생 정책 해제(첫 제스처), visibilitychange =
// 포그라운드 복귀. 셋 다 "죽어 있으면 재개"로만 쓰이므로 반복 부착돼도 무해.
const RESUME_EVENTS = ["pointerdown", "keydown", "visibilitychange"] as const;

/** interrupted 상태의 resume()은 WebKit에서 영원히 pending일 수 있어, 이 시간
 * 뒤엔 "못 살아났다"로 보고 재생성 판정으로 넘어간다. */
const RESUME_TIMEOUT_MS = 400;
/** 재생성 최소 간격 — 자동재생으로 새 컨텍스트도 suspended로 시작할 수 있어,
 * 연속 재생 호출이 재생성을 폭주시키지 않도록 디바운스한다. */
const REBUILD_COOLDOWN_MS = 4000;

/** AudioContext 생성 실패 시 null — 호출측은 사운드 전체를 조용히 비활성. */
export function createWebAudioBackend(deps: WebAudioDeps = {}): SoundBackend | null {
  const createContext = deps.createContext ?? (() => new AudioContext());
  let ctx: AudioContext;
  try {
    ctx = createContext();
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
  const now = deps.now ?? (() => Date.now());
  const delay = deps.delay ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));

  // 키프레스 샘플 비동기 로드 — 팩별로 독립. 개별 실패는 건너뛰고(그만큼
  // 변주가 줄 뿐), 팩 전체가 실패하면 그 팩은 빈 채로 남아 재생 시
  // pickPackSamples가 기본 팩/합성으로 폴백한다. gain은 팩 간 녹음 레벨
  // 보정(packGain)을 디코드 시점에 엔트리로 박아둔 것. AudioBuffer는 컨텍스트에
  // 묶이지 않는 순수 데이터라, 재생성 후에도 이 맵을 그대로 재사용한다(재로드 X).
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

  // 재생성(rebuild) 간에도 유지되는 상태.
  let bus: GainNode; // 마스터 게인 — 재생성마다 새 컨텍스트에서 다시 만든다.
  let currentGain = 0.25; // setVolume(0.5)^2 초기값. 재생성 시 새 bus에 재적용.
  let disposed = false;
  let rebuilding = false;
  let lastRebuildAt = Number.NEGATIVE_INFINITY;

  const onRevive = () => void reviveIfNeeded();

  /** 현재 ctx에 오디오 그래프·keep-alive·statechange 리스너를 조립한다.
   * 최초 생성과 재생성이 공유 — 보존한 볼륨(currentGain)을 새 bus에 적용한다. */
  function buildGraph(): void {
    const compressor = ctx.createDynamicsCompressor();
    compressor.threshold.value = -18;
    compressor.knee.value = 20;
    compressor.ratio.value = 8;
    compressor.attack.value = 0.003;
    compressor.release.value = 0.25;
    compressor.connect(ctx.destination);

    bus = ctx.createGain();
    bus.gain.value = currentGain;
    bus.connect(compressor);

    startKeepAlive();
    ctx.addEventListener("statechange", onRevive);
  }

  /** 무음 루프 소스 — 오디오 세션을 활성으로 붙들어 유휴 정리를 예방한다(가장
   * 흔한 유휴 무음 케이스 차단). bus/컴프레서를 우회해 destination에 직결(게인 0).
   * 최선노력이라 실패해도 재생성 복구가 남으니 치명적이지 않다. */
  function startKeepAlive(): void {
    try {
      const frames = Math.max(1, Math.floor(ctx.sampleRate));
      const buf = ctx.createBuffer(1, frames, ctx.sampleRate); // 무음(0으로 초기화)
      const src = ctx.createBufferSource();
      src.buffer = buf;
      src.loop = true;
      const g = ctx.createGain();
      g.gain.value = 0;
      src.connect(g).connect(ctx.destination);
      src.start();
    } catch {
      // keep-alive 실패는 무시 — 재생성 복구가 백스톱.
    }
  }

  /** 죽어 있으면(running/closed 아님) resume을 시도하고, 타임아웃 뒤에도 running이
   * 아니면 컨텍스트를 재생성한다. running/closed면 즉시 통과(핫패스). WebKit은
   * 비제스처 resume을 무시하므로, resume 실패를 재생성으로 확실히 복구한다. */
  async function reviveIfNeeded(): Promise<void> {
    if (disposed) return;
    const state = ctx.state as string;
    if (state === "running" || state === "closed") return;
    // resume은 WebKit interrupted에서 영원히 pending일 수 있어 타임아웃과 경주.
    try {
      await Promise.race([Promise.resolve(ctx.resume()).catch(() => {}), delay(RESUME_TIMEOUT_MS)]);
    } catch {
      // resume 거부 — 재생성 판정으로 진행.
    }
    if (disposed) return;
    if ((ctx.state as string) === "running") return;
    // resume으로 못 살렸다 → 죽은 것으로 보고 재생성(디바운스로 폭주 방지).
    if (rebuilding) return;
    if (now() - lastRebuildAt < REBUILD_COOLDOWN_MS) return;
    rebuild();
  }

  /** 죽은 컨텍스트를 버리고 새로 조립한다. 샘플은 재사용, 볼륨은 보존.
   * statechange 리스너를 close 전에 떼어 재귀 재생성을 막는다. */
  function rebuild(): void {
    if (disposed || rebuilding) return;
    rebuilding = true;
    lastRebuildAt = now();
    const old = ctx;
    old.removeEventListener("statechange", onRevive); // close 전에 — statechange 재귀 차단
    try {
      void old.close();
    } catch {
      // 이미 닫혔거나 close 실패 — 무시하고 새 컨텍스트로 넘어간다.
    }
    try {
      ctx = createContext();
    } catch (err) {
      console.warn("sound: AudioContext 재생성 실패 — 다음 기회에 재시도", err);
      rebuilding = false;
      return;
    }
    buildGraph();
    rebuilding = false;
    // 새 컨텍스트도 자동재생 정책으로 suspended면 1회 시도(제스처 리스너가 백스톱).
    if ((ctx.state as string) !== "running") void Promise.resolve(ctx.resume()).catch(() => {});
    if (disposed) teardown(); // 재생성 중 dispose가 끼어들었으면 새 컨텍스트도 정리
  }

  /** 현재 ctx의 statechange 리스너 해제 + close. dispose·재생성 레이스 공용. */
  function teardown(): void {
    ctx.removeEventListener("statechange", onRevive);
    void ctx.close();
  }

  buildGraph();
  // window/mediaDevices 리스너는 onRevive가 현재 ctx를 클로저로 참조하므로 1회만
  // 부착하면 재생성 후에도 유효하다(statechange만 컨텍스트별로 buildGraph에서 부착).
  for (const ev of RESUME_EVENTS) resumeTarget?.addEventListener(ev, onRevive);
  mediaDevices?.addEventListener("devicechange", onRevive);
  if ((ctx.state as string) !== "running") void Promise.resolve(ctx.resume()).catch(() => {}); // 부팅 자동재생 대비

  return {
    playClicks(agentId, count, packId) {
      // 죽어 있으면 복구를 걸되(async), 이번 버스트는 현재 ctx로 그냥 진행한다.
      // 아직 못 살아났으면 그 타는 무음 — 사운드는 장식이라 다음 호출부터 정상.
      void reviveIfNeeded();
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
      void reviveIfNeeded();
      playDing(ctx, bus, ctx.currentTime);
    },
    playSessionStart: () => {
      void reviveIfNeeded();
      playSweep(ctx, bus, ctx.currentTime, 330, 660);
    },
    playSessionEnd: () => {
      void reviveIfNeeded();
      playSweep(ctx, bus, ctx.currentTime, 660, 330);
    },
    setVolume(v) {
      currentGain = v * v; // 청감 보정(제곱) — 재생성 시 새 bus에 재적용할 값으로 보존.
      bus.gain.setTargetAtTime(currentGain, ctx.currentTime, 0.05);
    },
    dispose() {
      disposed = true;
      for (const ev of RESUME_EVENTS) resumeTarget?.removeEventListener(ev, onRevive);
      mediaDevices?.removeEventListener("devicechange", onRevive);
      teardown();
    },
  };
}
