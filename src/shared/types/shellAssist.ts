// src/shared/types/shellAssist.ts
//
// 셸 캐릭터(Rust `shell_assist`)의 LLM 설정과 명령 제안. 설정은 앱 설정과 따로
// `shell-assist.json`(0600)에 저장된다.
import type { SummaryProvider } from './settings';

/** OpenAI 호환 엔드포인트 하나(Ollama·LM Studio·llama.cpp·vLLM·mlx_lm 등). Rust `LocalEndpoint` 미러. */
export interface ShellAssistEndpoint {
  /** 화면 이름(예: "4090 PC"). 비면 주소. */
  label: string;
  /** `http://host:11434`, `…/v1`, `…/v1/chat/completions` 어느 꼴이든 된다. */
  baseUrl: string;
  model: string;
  /** 대부분 비운다. 있으면 Bearer로 보낸다. */
  apiKey: string;
  enabled: boolean;
}

/** Rust `ShellAssistConfig` 미러. */
export interface ShellAssistConfig {
  /** 위에서부터 차례로 시도한다. */
  endpoints: ShellAssistEndpoint[];
  /** 로컬이 모두 실패하면 클라우드로 넘어갈지(명령 출력이 클라우드로 간다). 기본 끔. */
  fallbackEnabled: boolean;
  fallbackProvider: SummaryProvider;
  /** 비면 그 provider의 라벨 요약 기본 모델. */
  fallbackModel: string;
  /** 명령이 끝날 때마다 결과 설명을 붙인다. */
  explain: boolean;
}

/** 말 → 명령 제안. Rust `llm::Suggestion` 미러. */
export interface ShellSuggestion {
  command: string;
  explanation: string;
  interactive: boolean;
  risky: boolean;
  multiline: boolean;
  /** 답한 곳(엔드포인트 이름 또는 `cloud:<provider>`). */
  source: string;
}
