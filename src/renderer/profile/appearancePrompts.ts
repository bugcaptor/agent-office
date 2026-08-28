// src/renderer/profile/appearancePrompts.ts
//
// 외형 이미지 3종(초상·스프라이트·미니미) × 두 용도(사람이 복사해 갈 프롬프트 /
// codex CLI에 그대로 먹일 프롬프트)의 **디스패치 한 곳**.
//
// 원래는 `ProfileDialog` 안에 `onCopyPrompt`/`onCopySpritePrompt`/
// `onCopyMinimiPrompt` 세 함수가 각자 자기 빌더를 부르는 모양으로 복붙돼 있었고,
// codex 쪽은 같은 분기가 `onGenerateCodex` 안에 삼항 사슬로 한 번 더 있었다.
// 종류가 늘 때 고쳐야 할 자리가 넷이라 여기 하나로 모았다.
//
// 빌더마다 요구하는 필드가 다른 것이 핵심이다:
//   · 초상은 성격(personality)까지 읽는다 — 얼굴은 성격이 드러나야 한다.
//   · 미니미는 자기 의뢰문이 비면 스프라이트 의뢰문을 폴백으로 받는다(빌더가
//     "본체에 어울리는 소환수를 알아서" 문장으로 처리한다).
// 그래서 공통 인자 하나로 뭉개지 않고 종류별로 필요한 것만 넘긴다.
import {
  buildCodexMinimiPrompt,
  buildCodexPortraitPrompt,
  buildCodexSpritePrompt,
  buildMinimiPrompt,
  buildPortraitPrompt,
  buildSpritePrompt,
} from "../portrait/promptBuilder";
import type { CodexGenKind } from "../portrait/CodexGenPanel";
import type { DraftProfile } from "./generate";

/** 이미지 한 종류의 프롬프트를 만든다. `codex`면 codex CLI용 변형을 쓴다. */
export function appearancePrompt(
  kind: CodexGenKind,
  draft: DraftProfile,
  opts: { codex: boolean },
): string {
  const common = {
    name: draft.name,
    role: draft.role,
    seed: draft.seed,
    archetype: draft.archetype,
    colors: draft.colors,
  };
  switch (kind) {
    case "portrait": {
      const args = {
        ...common,
        personality: draft.personalityPrompt ?? "",
        portraitRequest: draft.portraitRequest,
      };
      return opts.codex ? buildCodexPortraitPrompt(args) : buildPortraitPrompt(args);
    }
    case "minimi": {
      const args = {
        ...common,
        minimiRequest: draft.minimiRequest,
        spriteRequest: draft.spriteRequest,
      };
      return opts.codex ? buildCodexMinimiPrompt(args) : buildMinimiPrompt(args);
    }
    case "sprite": {
      const args = { ...common, spriteRequest: draft.spriteRequest };
      return opts.codex ? buildCodexSpritePrompt(args) : buildSpritePrompt(args);
    }
  }
}
