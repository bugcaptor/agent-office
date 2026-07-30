// 확인 요청 대사 TTS 직렬 큐 검증 — 겹침 방지, 에이전트당 최신 1건, 상한,
// 실패 격리, 게이트 재확인.
import { describe, expect, it, vi } from "vitest";
import { MAX_PENDING, base64ToBytes, createVoiceQueue } from "../voiceQueue";
import type { TtsSpeakRequest, TtsSpeakResult } from "@shared/types";

function result(line: string): TtsSpeakResult {
  return {
    audioBase64: "AAAA",
    mimeType: "audio/mpeg",
    line,
    voiceId: "v1",
    modelId: "eleven_v3",
    cached: false,
    rewritten: true,
    rewriteVia: "api",
  };
}

function req(agentId: string, message = "m"): TtsSpeakRequest {
  return { agentId, agentName: agentId, seed: `seed-${agentId}`, message };
}

/** 수동으로 resolve할 수 있는 프라미스 — 재생 겹침을 관찰하기 위한 도구. */
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((r) => (resolve = r));
  return { promise, resolve };
}

describe("createVoiceQueue", () => {
  it("재생이 끝나기 전에 다음 발화를 시작하지 않는다 (직렬)", async () => {
    const gates = [deferred(), deferred()];
    let playing = 0;
    let maxConcurrent = 0;
    let idx = 0;
    const q = createVoiceQueue({
      speak: async (r) => result(r.message),
      play: async () => {
        playing++;
        maxConcurrent = Math.max(maxConcurrent, playing);
        await gates[idx++].promise;
        playing--;
      },
    });

    q.enqueue(req("a"));
    q.enqueue(req("b"));
    await vi.waitFor(() => expect(playing).toBe(1));
    expect(maxConcurrent).toBe(1);
    gates[0].resolve();
    await vi.waitFor(() => expect(idx).toBe(2));
    gates[1].resolve();
    await vi.waitFor(() => expect(playing).toBe(0));
    expect(maxConcurrent).toBe(1);
  });

  it("대기 중 같은 에이전트가 또 물으면 최신 문구만 발화한다", async () => {
    const gate = deferred();
    const spoken: string[] = [];
    const q = createVoiceQueue({
      speak: async (r) => {
        spoken.push(`${r.agentId}:${r.message}`);
        return result(r.message);
      },
      play: async () => {
        // 첫 항목 재생 중에 대기열이 쌓이도록 붙잡는다.
        if (spoken.length === 1) await gate.promise;
      },
    });

    q.enqueue(req("a", "first"));
    await vi.waitFor(() => expect(spoken).toHaveLength(1));
    q.enqueue(req("b", "old-b"));
    q.enqueue(req("b", "new-b"));
    expect(q.pendingCount()).toBe(1);
    gate.resolve();
    await vi.waitFor(() => expect(spoken).toHaveLength(2));
    expect(spoken[1]).toBe("b:new-b");
  });

  it("대기열이 상한을 넘으면 오래된 것부터 버린다", async () => {
    const gate = deferred();
    let started = 0;
    const q = createVoiceQueue({
      speak: async (r) => result(r.message),
      play: async () => {
        started++;
        if (started === 1) await gate.promise;
      },
    });
    q.enqueue(req("hold"));
    await vi.waitFor(() => expect(started).toBe(1));
    for (let i = 0; i < MAX_PENDING + 5; i++) q.enqueue(req(`a${i}`));
    expect(q.pendingCount()).toBe(MAX_PENDING);
    gate.resolve();
  });

  it("합성/재생 실패가 큐를 막지 않는다", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const spoken: string[] = [];
    const q = createVoiceQueue({
      speak: async (r) => {
        if (r.agentId === "boom") throw new Error("missing_elevenlabs_key: …");
        return result(r.message);
      },
      play: async (mp3) => {
        spoken.push(String(mp3.byteLength));
      },
    });
    q.enqueue(req("boom"));
    q.enqueue(req("ok"));
    await vi.waitFor(() => expect(spoken).toHaveLength(1));
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it("shouldSpeak가 false면 대기 항목을 발화하지 않고 버린다", async () => {
    let allowed = true;
    const spoken: string[] = [];
    const gate = deferred();
    let started = 0;
    const q = createVoiceQueue({
      speak: async (r) => {
        started++;
        if (started === 1) await gate.promise;
        return result(r.message);
      },
      play: async () => {
        spoken.push("play");
      },
      shouldSpeak: () => allowed,
    });
    q.enqueue(req("a"));
    await vi.waitFor(() => expect(started).toBe(1));
    q.enqueue(req("b"));
    // 합성 왕복 중 사용자가 무음/TTS OFF로 바꾼 상황.
    allowed = false;
    gate.resolve();
    await vi.waitFor(() => expect(q.pendingCount()).toBe(0));
    expect(spoken).toHaveLength(0);
    expect(started).toBe(1);
  });

  it("clear는 대기열만 비운다", () => {
    const q = createVoiceQueue({
      speak: async (r) => result(r.message),
      play: async () => new Promise<void>(() => {}), // 영원히 재생 중
    });
    q.enqueue(req("a"));
    q.enqueue(req("b"));
    q.clear();
    expect(q.pendingCount()).toBe(0);
  });
});

describe("base64ToBytes", () => {
  it("base64를 바이트 그대로 복원한다", () => {
    // "ID3" (mp3 태그 매직)
    const bytes = new Uint8Array(base64ToBytes(btoa("ID3")));
    expect(Array.from(bytes)).toEqual([0x49, 0x44, 0x33]);
  });

  it("0x80 이상 바이트도 손실 없이 복원한다 (latin1 경계)", () => {
    const raw = new Uint8Array([0xff, 0xfb, 0x90, 0x00]);
    const b64 = btoa(String.fromCharCode(...raw));
    expect(Array.from(new Uint8Array(base64ToBytes(b64)))).toEqual(Array.from(raw));
  });
});
