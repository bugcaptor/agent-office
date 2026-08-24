// @vitest-environment jsdom
//
// src/renderer/usage/__tests__/UsageDialog.test.tsx
//
// usageView가 문장 대신 키를 돌려주게 바뀐 뒤(i18n phase 4b), **모달이 그 키를
// 실제로 문구로 푸는지**를 언어별로 확인한다. 순수 함수 테스트는 키만 보므로
// 이 파일이 "화면에 뭐가 찍히는가" 쪽을 맡는다. self-gate·닫기도 함께.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterAll, afterEach, beforeEach, describe, expect, it } from "vitest";
import { SOURCE_LANGUAGE, initI18nForTest } from "@renderer/i18n";
import type { UsageSnapshot } from "@shared/types";
import { useAppStore } from "../../store/appStore";
import { UsageDialog } from "../UsageDialog";

const initialState = useAppStore.getState();
const NOW = Date.now();

function snapshot(): UsageSnapshot {
  return {
    claude: {
      provider: "claude",
      fetchedAtMs: NOW,
      planLabel: "Max",
      windows: [
        {
          kind: "session",
          label: null,
          usedPercent: 61.4,
          // +30초 여유: 모달의 1초 tick이 읽는 Date.now()가 NOW보다 몇 ms
          // 뒤라 정확히 12분 경계로 잡으면 11분으로 내려간다.
          resetsAtMs: NOW + (3 * 60 + 12) * 60_000 + 30_000,
          windowMinutes: null,
          isActive: true,
        },
      ],
    },
    codex: null,
    claudeLive: {
      outcome: "network_error",
      tokenSource: null,
      detail: null,
      lastAttemptMs: NOW - 60_000,
      lastSuccessMs: null,
      via: null,
    },
    codexLive: {
      outcome: "ok",
      detail: null,
      lastAttemptMs: null,
      lastSuccessMs: null,
    },
  };
}

beforeEach(async () => {
  // 앞 테스트가 언어를 바꿔 놓았을 수 있다 — 매번 정본으로 되돌린다.
  await initI18nForTest(SOURCE_LANGUAGE);
  useAppStore.setState(initialState, true);
});
afterEach(() => cleanup());
afterAll(async () => {
  await initI18nForTest(SOURCE_LANGUAGE); // 정본 복구(파일 간 언어 상태 누수 방지)
});

describe("UsageDialog", () => {
  it("모달이 usage가 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<UsageDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("정본(ko)에서 창 라벨·카운트다운·신선도·진단을 예전 문구 그대로 그린다", () => {
    useAppStore.setState({ usage: snapshot() });
    useAppStore.getState().openModal({ kind: "usage" });
    const { container } = render(<UsageDialog />);

    expect(screen.getByText("구독 사용량")).toBeTruthy();
    expect(container.querySelector(".usage-window-label")?.textContent).toBe("5시간지금 적용 중");
    expect(container.querySelector(".usage-window-pct")?.textContent).toBe("61%");
    expect(container.querySelector(".usage-countdown")?.textContent).toBe("3시간 12분 후 리셋");
    expect(container.querySelector(".usage-freshness")?.textContent).toBe("방금 기준");
    expect(container.querySelector(".usage-bar")?.getAttribute("aria-label")).toBe("5시간 사용률");

    // 진단 줄: 사유 + 캐시 안내($t 중첩) + 시도 이력(가운뎃점으로 이음).
    const note = container.querySelector(".usage-live-note")!.textContent!;
    expect(note).toContain("실시간 조회 실패: 네트워크 오류.");
    expect(note).toContain("/usage");
    expect(note).toContain("마지막 시도 1분 전 · 성공 이력 없음");

    // 데이터가 없는 provider는 안내 문구를 그린다.
    expect(screen.getByText(/codex login/)).toBeTruthy();
  });

  it("en에서도 같은 자리에 영어 문구가 들어가고 키가 새지 않는다", async () => {
    await initI18nForTest("en");
    useAppStore.setState({ usage: snapshot() });
    useAppStore.getState().openModal({ kind: "usage" });
    const { container } = render(<UsageDialog />);

    expect(screen.getByText("Usage")).toBeTruthy();
    expect(container.querySelector(".usage-window-label")?.textContent).toBe("5hin effect");
    expect(container.querySelector(".usage-countdown")?.textContent).toBe("Resets in 3h 12m");
    expect(container.querySelector(".usage-freshness")?.textContent).toBe("as of just now");
    const note = container.querySelector(".usage-live-note")!.textContent!;
    expect(note).toContain("Live fetch failed: network error.");
    expect(note).toContain("last try 1m ago · never succeeded");
    expect(container.textContent).not.toContain("usage.");
    expect(container.textContent).not.toContain("$t(");
  });

  it("닫기 버튼이 모달을 닫는다", () => {
    useAppStore.getState().openModal({ kind: "usage" });
    render(<UsageDialog />);
    fireEvent.click(screen.getByRole("button", { name: "닫기" }));
    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});
