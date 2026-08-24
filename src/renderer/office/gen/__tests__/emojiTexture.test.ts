// @vitest-environment jsdom
// src/renderer/office/gen/__tests__/emojiTexture.test.ts
//
// 이모지 굽기의 **폴백 계약**을 못 박는다. 실제 래스터 결과(픽셀)는 시스템
// 이모지 폰트에 달려 있어 테스트 대상이 아니다 — 여기서 지켜야 할 것은
// "2d 컨텍스트가 없거나 글리프가 비면 던지지 않고 null을 낸다"와 "같은
// (문자, 크기)는 다시 굽지 않는다" 둘뿐이다. 호출부(TrophyOverlay)가 null일 때
// 절차적 드로잉으로 떨어지므로, 이 계약이 깨지면 트로피가 통째로 사라진다.
import { afterEach, describe, expect, it, vi } from "vitest";
import { bakeEmojiTexture } from "../emojiTexture";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("bakeEmojiTexture", () => {
  it("2d 컨텍스트가 없으면 던지지 않고 null을 낸다(jsdom·헤드리스 경로)", () => {
    const spy = vi
      .spyOn(HTMLCanvasElement.prototype, "getContext")
      .mockReturnValue(null);
    expect(bakeEmojiTexture("🧪", 13, 0x112233)).toBeNull();
    expect(spy).toHaveBeenCalled();
  });

  it("아무것도 안 그려진 글리프(폰트 없음)는 null이다", () => {
    // 알파가 전부 0인 이미지 = 경계 상자 없음.
    const blank = {
      textAlign: "",
      textBaseline: "",
      font: "",
      imageSmoothingEnabled: true,
      fillText: () => {},
      drawImage: () => {},
      getImageData: () => ({ data: new Uint8ClampedArray(256 * 256 * 4) }),
    };
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(
      blank as unknown as ReturnType<HTMLCanvasElement["getContext"]>,
    );
    expect(bakeEmojiTexture("\u{1F9EA}\u{200D}\u{1F9EB}", 13)).toBeNull();
  });

  it("같은 (문자, 크기)는 한 번만 굽는다 — 씬 재구축마다 다시 굽지 않게", () => {
    const spy = vi
      .spyOn(HTMLCanvasElement.prototype, "getContext")
      .mockReturnValue(null);
    const first = bakeEmojiTexture("🧫", 13, 0x112233);
    const callsAfterFirst = spy.mock.calls.length;
    const second = bakeEmojiTexture("🧫", 13, 0x112233);
    expect(second).toBe(first);
    expect(spy.mock.calls.length).toBe(callsAfterFirst); // 두 번째는 캐시 히트
  });
});
