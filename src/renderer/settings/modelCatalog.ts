// src/renderer/settings/modelCatalog.ts
//
// 설정 화면의 "모델 고르기"가 쓰는 서비스별 모델 목록(kbm #2fc).
//
// 목록은 두 층이다:
//  - 정적 프리셋: 서비스마다 자주 쓰는 것 몇 개. 실시간 조회가 실패하거나
//    (오프라인·키 없음·CLI 미설치) 애초에 라이브 소스가 없는 서비스에서도
//    최소한 이만큼은 보여야 하므로 폴백을 겸한다. 항상 목록 맨 앞이다 —
//    뒤에 두면 자주 쓰는 것이 수백 개 카탈로그 아래로 밀린다.
//  - 실시간 카탈로그: `list_provider_models`. provider마다 소스가 다르다
//    (OpenRouter 공개 카탈로그 / Anthropic `/v1/models` / `opencode models`).
//
// 어느 층이든 **강제가 아니라 힌트**다. 모델 id는 그대로 CLI의 `--model`이나
// API 본문으로 실리므로, 목록에 없는 새 모델도 그냥 적어 넣으면 된다 —
// 새 모델이 나올 때마다 앱을 고쳐야 하는 쪽이 훨씬 나쁘다.
//
// 조회는 **provider당 세션 1회**만 한다(모듈 수준 캐시). 설정 다이얼로그는
// 열고 닫기를 반복하는 창이고 카탈로그는 그 사이에 바뀌지 않는다. 새로고침을
// 누르면 그 provider의 캐시만 버린다.
import { useCallback, useEffect, useRef, useState } from "react";
import type { ModelCatalogProvider } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";

/**
 * 서비스별 "자주 쓰는 것" 상단 고정 목록 겸 라이브 조회 실패 시의 폴백.
 * 요약기 기본 모델(SettingsDialog의 SUMMARY_DEFAULT_MODELS)을 포함해야
 * 한다 — 기본값이 목록에 없으면 "지금 쓰이는 모델"을 고를 수가 없다.
 */
export const MODEL_PRESETS: Record<ModelCatalogProvider, string[]> = {
  claude: ["haiku", "sonnet", "opus", "claude-haiku-4-5", "claude-sonnet-5", "claude-opus-5"],
  anthropic: ["claude-haiku-4-5", "claude-sonnet-5", "claude-opus-5"],
  codex: ["gpt-5.4-mini", "gpt-5.4"],
  agy: ["gemini-3.6-flash-low", "gemini-3.1-pro-low"],
  gemini: ["gemini-2.5-flash", "gemini-2.5-pro"],
  opencode: ["opencode-go/deepseek-v4-flash", "opencode-go/deepseek-v4-pro"],
  openrouter: [
    "openai/gpt-5.4-mini",
    "openai/gpt-5.4",
    "anthropic/claude-haiku-4.5",
    "google/gemini-2.5-flash",
    "meta-llama/llama-4-maverick",
    "deepseek/deepseek-v4-flash",
    "deepseek/deepseek-v4-pro",
    "deepseek/deepseek-chat-v3.1",
  ],
};

/** 라이브 조회의 결과 — 성공(빈 목록 포함)과 실패를 구분해 안내 문구가
 *  "이 서비스는 목록을 못 준다"와 "조회에 실패했다"를 갈라 말하게 한다. */
type Fetched = { models: string[]; failed: boolean };

/** provider당 1회 조회를 보장하는 캐시. 실패도 캐시한다 — 다이얼로그를 열
 *  때마다 죽은 네트워크나 없는 CLI를 다시 두드릴 이유가 없다. */
const cache = new Map<ModelCatalogProvider, Promise<Fetched>>();

function fetchOnce(provider: ModelCatalogProvider): Promise<Fetched> {
  let hit = cache.get(provider);
  if (!hit) {
    hit = (async (): Promise<Fetched> => {
      try {
        return { models: await tauriApi.listProviderModels(provider), failed: false };
      } catch {
        // 조용히 정적 프리셋만 쓴다 — 모델 추천은 있으면 좋은 것이지,
        // 없다고 설정을 못 하는 것이 아니다.
        return { models: [], failed: true };
      }
    })();
    cache.set(provider, hit);
  }
  return hit;
}

/** 테스트 전용 — 모듈 캐시를 비운다(각 케이스가 독립적으로 조회를 관찰하도록). */
export function resetModelCatalogCache(): void {
  cache.clear();
}

export interface ModelCatalog {
  /** 프리셋 + 실시간(중복 제거). 프리셋이 항상 앞이다. */
  models: string[];
  /** `models` 앞쪽 몇 개가 프리셋인지 — 목록에 구분선을 넣는 데 쓴다. */
  presetCount: number;
  /** 조회가 아직 끝나지 않았는지. */
  loading: boolean;
  /** 조회가 실패했는지(라이브 소스가 없어 빈 목록인 것과 구분). */
  failed: boolean;
  /** 캐시를 버리고 다시 조회한다. */
  reload: () => void;
}

/**
 * 서비스별 모델 목록. `enabled`가 false면 조회를 아예 하지 않는다 —
 * 목록을 펼치지 않은 사용자는 네트워크도 CLI도 건드리지 않아야 한다.
 */
export function useModelCatalog(
  provider: ModelCatalogProvider,
  enabled: boolean,
): ModelCatalog {
  const presets = MODEL_PRESETS[provider] ?? [];
  const [live, setLive] = useState<Fetched | null>(null);
  // 새로고침은 캐시를 버린 뒤 이 값을 올려 effect를 다시 돌린다.
  const [nonce, setNonce] = useState(0);
  // provider가 바뀌면 이전 서비스의 목록을 그대로 보여주면 안 된다.
  const shownFor = useRef<ModelCatalogProvider | null>(null);
  if (shownFor.current !== provider) {
    shownFor.current = provider;
    if (live !== null) setLive(null);
  }

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    void fetchOnce(provider).then((got) => {
      if (alive) setLive(got);
    });
    return () => {
      alive = false;
    };
  }, [provider, enabled, nonce]);

  const reload = useCallback(() => {
    cache.delete(provider);
    setLive(null);
    setNonce((n) => n + 1);
  }, [provider]);

  const seen = new Set(presets);
  const models = [...presets, ...(live?.models ?? []).filter((m) => !seen.has(m))];
  return {
    models,
    presetCount: presets.length,
    loading: enabled && live === null,
    failed: live?.failed ?? false,
    reload,
  };
}
