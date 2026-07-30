// src/renderer/sound/voiceQueue.ts
//
// 확인 요청 대사 TTS의 직렬 큐. 여러 에이전트가 동시에 물어보면 목소리가
// 겹치지 않게 순차 재생한다.
//
// 정책 세 가지:
//  1) **한 번에 하나.** 합성(`tts_speak`)과 재생(`playVoice`)을 한 슬롯으로 묶어
//     직렬화한다. 합성만 병렬로 돌리면 재생 순서가 응답 시간에 따라 뒤집히고
//     ElevenLabs 동시요청 제한에도 걸린다.
//  2) **에이전트당 대기 1건, 최신 우선.** 같은 캐릭터가 대기 중 또 물어보면
//     오래된 문구를 버리고 최신 것만 남긴다 — 이미 지나간 확인 요청을 읽어주는
//     것은 소음이다.
//  3) **큐 상한.** 캐릭터가 아무리 많아도 대기열이 무한히 자라지 않게 오래된
//     항목부터 버린다.
//
// 실패는 전부 조용하다(장식 기능). 큐가 막히지 않도록 어떤 예외도 슬롯을
// 소비하고 다음으로 넘어간다.

import type { AgentOfficeApi, TtsSpeakRequest } from "@shared/types";

/** 동시에 대기시킬 최대 캐릭터 수. 넘으면 가장 오래된 것부터 버린다. */
export const MAX_PENDING = 6;

export interface VoiceQueueDeps {
  /** `tauriApi.ttsSpeak` — 합성. 실패는 삼킨다. */
  speak: AgentOfficeApi["ttsSpeak"];
  /** 디코드+재생. 재생 완료 시 resolve해야 직렬이 성립한다. */
  play: (mp3: ArrayBuffer) => Promise<void>;
  /** 매 항목을 꺼내는 시점에 아직 발화해도 되는지(muted/설정 OFF 등). */
  shouldSpeak?: () => boolean;
}

export interface VoiceQueue {
  /** 발화 요청. 같은 agentId의 대기 항목은 이것으로 대체된다. */
  enqueue(request: TtsSpeakRequest): void;
  /** 대기 중 항목 수(테스트/디버그용). 재생 중인 것은 포함하지 않는다. */
  pendingCount(): number;
  /** 대기열을 비운다. 재생 중인 항목은 끝까지 간다. */
  clear(): void;
}

/** base64 → ArrayBuffer. atob는 latin1 문자열을 주므로 바이트로 펼친다. */
export function base64ToBytes(b64: string): ArrayBuffer {
  const bin = atob(b64);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out.buffer;
}

export function createVoiceQueue(deps: VoiceQueueDeps): VoiceQueue {
  // Map은 삽입 순서를 보존하므로 그 자체가 FIFO다. agentId 키 재대입은 순서를
  // 유지한 채 값만 교체하므로 "최신 것만" 정책과 자연스럽게 맞물린다.
  const pending = new Map<string, TtsSpeakRequest>();
  let running = false;

  async function drain(): Promise<void> {
    if (running) return;
    running = true;
    try {
      for (;;) {
        const next = pending.entries().next();
        if (next.done) return;
        const [agentId, request] = next.value;
        pending.delete(agentId);
        if (deps.shouldSpeak && !deps.shouldSpeak()) continue;
        try {
          const result = await deps.speak(request);
          if (deps.shouldSpeak && !deps.shouldSpeak()) continue;
          await deps.play(base64ToBytes(result.audioBase64));
        } catch (err) {
          // 키 미설정·쿼터 초과·네트워크 실패 등. 경고만 남기고 다음으로.
          console.warn("tts: 대사 발화 실패", err);
        }
      }
    } finally {
      running = false;
    }
  }

  return {
    enqueue(request) {
      pending.set(request.agentId, request);
      // 상한 초과분은 가장 오래된 것부터(삽입 순서 앞부터) 버린다.
      while (pending.size > MAX_PENDING) {
        const oldest = pending.keys().next();
        if (oldest.done) break;
        pending.delete(oldest.value);
      }
      void drain();
    },
    pendingCount: () => pending.size,
    clear: () => pending.clear(),
  };
}
