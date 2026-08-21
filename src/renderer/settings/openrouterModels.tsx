// src/renderer/settings/openrouterModels.tsx
//
// 설정 화면 두 곳(요약 모델·TTS 리라이트 모델)이 공유하는 OpenRouter 모델
// 추천 목록. 모델 입력은 자유 텍스트라 이 목록은 강제가 아니라 힌트다 —
// 목록에 없는 모델도 그냥 적어 넣으면 된다.
//
// 목록은 두 층이다:
//  - 정적 프리셋: 자주 쓰는 것 몇 개. 실시간 조회가 실패해도(오프라인·프록시)
//    최소한 이만큼은 보여야 하므로 폴백을 겸한다.
//  - 실시간 카탈로그: `openrouter_list_models`(키 불필요 공개 GET). 수백 개라
//    프리셋 뒤에 붙인다 — 앞에 두면 자주 쓰는 것이 스크롤 저 아래로 밀린다.
//
// 조회는 **세션당 한 번**만 한다(모듈 수준 캐시). 설정 다이얼로그는 열고 닫기를
// 반복하는 창이고, 카탈로그는 그 사이에 바뀌지 않는다.
import { useEffect, useMemo, useState } from "react";
import { tauriApi } from "../ipc/tauriApi";

/** 실시간 조회 실패 시의 폴백 겸 "자주 쓰는 것" 상단 고정 목록. */
export const OPENROUTER_MODEL_PRESETS = [
  "openai/gpt-5.4-mini",
  "openai/gpt-5.4",
  "anthropic/claude-haiku-4.5",
  "google/gemini-2.5-flash",
  "meta-llama/llama-4-maverick",
  "deepseek/deepseek-v4-flash",
  "deepseek/deepseek-v4-pro",
  "deepseek/deepseek-chat-v3.1",
];

/** 세션당 1회 조회를 보장하는 캐시. 실패도 "빈 목록"으로 캐시한다 —
 *  다이얼로그를 열 때마다 죽은 네트워크를 다시 두드릴 이유가 없다. */
let cached: Promise<string[]> | null = null;

function fetchModelsOnce(): Promise<string[]> {
  if (!cached) {
    cached = (async () => {
      try {
        return await tauriApi.openrouterListModels();
      } catch {
        // 조용히 정적 프리셋만 쓴다 — 모델 추천은 있으면 좋은 것이지
        // 없다고 설정을 못 하는 것이 아니다.
        return [];
      }
    })();
  }
  return cached;
}

/** 테스트 전용 — 모듈 캐시를 비운다(각 케이스가 독립적으로 조회를 관찰하도록). */
export function resetOpenrouterModelsCache(): void {
  cached = null;
}

/** 프리셋 + 실시간 목록(중복 제거). 조회 전/실패 시에는 프리셋만. */
export function useOpenrouterModels(): string[] {
  const [live, setLive] = useState<string[]>([]);

  useEffect(() => {
    let alive = true;
    void fetchModelsOnce().then((list) => {
      if (alive) setLive(list);
    });
    return () => {
      alive = false;
    };
  }, []);

  return useMemo(() => {
    const seen = new Set(OPENROUTER_MODEL_PRESETS);
    return [...OPENROUTER_MODEL_PRESETS, ...live.filter((m) => !seen.has(m))];
  }, [live]);
}

/**
 * `<input list=...>`가 가리킬 datalist. 훅을 이 컴포넌트 안에 가둬 두면
 * **datalist가 실제로 렌더될 때만** 조회가 돈다 — OpenRouter를 고르지 않은
 * 사용자는 네트워크를 전혀 건드리지 않는다.
 */
export function OpenrouterModelDatalist({ id }: { id: string }) {
  const models = useOpenrouterModels();
  return (
    <datalist id={id}>
      {models.map((m) => (
        <option key={m} value={m} />
      ))}
    </datalist>
  );
}
