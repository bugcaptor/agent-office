// src/renderer/profile/useAppearanceImages.ts
//
// 외형 이미지 3종(초상·스프라이트·미니미)의 상태와 조작.
//
// 셋은 크기도 쓰임도 다르지만 **다루는 절차가 같다**: 프롬프트를 복사해 사람이
// 밖에서 그려 오거나 codex CLI로 뽑고 → 전용 크롭 편집기로 규격화해 저장하고 →
// 필요하면 지운다. `ProfileDialog` 안에서는 이 절차가 종류마다 한 벌씩, 총 세 벌
// 복붙돼 있었다. 여기서는 종류를 인자로 받는 한 벌로 줄인다 — 종류가 하나 더
// 늘어도 고칠 자리는 `IMAGE_KINDS` 표 하나다.
//
// 저장(업로드) 경로가 여기 없는 이유: 저장은 각 편집기(PortraitEditor /
// SpriteEditor)가 크롭 결과를 직접 백엔드에 쓰고 스토어를 갱신한다. 이 훅은
// 편집기를 **여는 것**(초기 이미지 전달 포함)까지만 책임진다.
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { clearMinimiOverride } from "../office/gen/minimiOverrides";
import { clearSpriteOverride } from "../office/gen/spriteOverrides";
import { codexGenErrorCaption, type CodexGenKind } from "../portrait/CodexGenPanel";
import { appearancePrompt } from "./appearancePrompts";
import type { AgentProfile } from "@shared/types";
import type { DraftProfile } from "./generate";

/** "외형" 섹션 모드. 직접 만들기(프롬프트 복사 + 업로드) / Codex로 생성.
 *  한 번에 하나만 보여 준다 — 두 경로를 나란히 두면 두서없이 보인다. */
export type AppearanceMode = "manual" | "codex";

/** 종류마다 다른 것은 이 표에 담긴 네 조각뿐이다. */
const IMAGE_KINDS = {
  portrait: {
    remove: (id: string) => tauriApi.deletePortrait(id),
    clearOverride: () => {},
    updatedAtKey: "portraitUpdatedAt",
  },
  sprite: {
    remove: (id: string) => tauriApi.deleteSprite(id),
    clearOverride: clearSpriteOverride,
    updatedAtKey: "spriteUpdatedAt",
  },
  minimi: {
    remove: (id: string) => tauriApi.deleteMinimi(id),
    clearOverride: clearMinimiOverride,
    updatedAtKey: "minimiUpdatedAt",
  },
} as const satisfies Record<
  CodexGenKind,
  {
    remove: (id: string) => Promise<unknown>;
    clearOverride: (id: string) => void;
    updatedAtKey: "portraitUpdatedAt" | "spriteUpdatedAt" | "minimiUpdatedAt";
  }
>;

export function useAppearanceImages(deps: {
  draft: DraftProfile;
  editingAgent: AgentProfile | undefined;
  /** 편집 대상 id. 바뀌면 진행 중이던 생성 응답과 이전 세션의 캡션을 버린다. */
  editingAgentId: string | undefined;
}) {
  const { draft, editingAgent, editingAgentId } = deps;
  const { t } = useTranslation("profile");

  const updateAgent = useAppStore((s) => s.updateAgent);
  const removePortrait = useAppStore((s) => s.removePortrait);
  const removeSpritePreview = useAppStore((s) => s.removeSpritePreview);
  const removeMinimiPreview = useAppStore((s) => s.removeMinimiPreview);
  const portraitUrl = useAppStore((s) =>
    editingAgent ? s.portraits[editingAgent.id] : undefined,
  );
  const spritePreviewUrl = useAppStore((s) =>
    editingAgent ? s.spritePreviews[editingAgent.id] : undefined,
  );
  const minimiPreviewUrl = useAppStore((s) =>
    editingAgent ? s.minimiPreviews[editingAgent.id] : undefined,
  );

  const [mode, setMode] = useState<AppearanceMode>("manual");
  const [editorOpen, setEditorOpen] = useState(false);
  const [spriteEditorOpen, setSpriteEditorOpen] = useState(false);
  const [minimiEditorOpen, setMinimiEditorOpen] = useState(false);
  /** Codex 생성 결과 data URL — 각각 PortraitEditor/SpriteEditor initialImage로 전달. */
  const [generatedPortrait, setGeneratedPortrait] = useState<string | null>(null);
  const [generatedImage, setGeneratedImage] = useState<string | null>(null);
  const [generatedMinimi, setGeneratedMinimi] = useState<string | null>(null);
  const [codexBusy, setCodexBusy] = useState<CodexGenKind | null>(null);
  const [codexNote, setCodexNote] = useState<string | null>(null);
  /** 진행 중 생성 요청의 세션 토큰 — 편집 대상이 바뀌거나 다이얼로그가
   *  닫히면 무효화된다 (상시 마운트 컴포넌트라 unmount 가드는 무의미). */
  const codexSeqRef = useRef(0);

  // 편집 세션 경계: 진행 중 생성 응답 무효화 + 이전 세션의 캡션/이미지/busy 정리.
  useEffect(() => {
    codexSeqRef.current++;
    setCodexBusy(null);
    setCodexNote(null);
    setGeneratedImage(null);
    setGeneratedPortrait(null);
    setGeneratedMinimi(null);
  }, [editingAgentId]);

  /** 사람이 밖의 그림 도구에 붙여 넣을 프롬프트를 클립보드에 담는다. */
  const copyPrompt = async (kind: CodexGenKind) => {
    try {
      await navigator.clipboard.writeText(appearancePrompt(kind, draft, { codex: false }));
    } catch (err) {
      console.warn("ProfileDialog: clipboard write failed", err);
    }
  };

  /** 저장된 이미지를 지운다 — 백엔드 파일, 렌더러 오버라이드, 스토어 프리뷰,
   *  프로필의 갱신 시각까지 한 번에. 하나라도 빠지면 화면 어딘가에 옛 그림이 남는다. */
  const removeImage = async (kind: CodexGenKind) => {
    if (!editingAgent) return;
    const id = editingAgent.id;
    const spec = IMAGE_KINDS[kind];
    try {
      await spec.remove(id);
    } catch (err) {
      console.warn(`ProfileDialog: delete ${kind} failed`, err);
    }
    spec.clearOverride(id);
    if (kind === "portrait") removePortrait(id);
    if (kind === "sprite") removeSpritePreview(id);
    if (kind === "minimi") removeMinimiPreview(id);
    updateAgent(id, { [spec.updatedAtKey]: undefined });
  };

  /** codex CLI로 원본 1장을 만든 뒤, 해당 크롭 편집기를 프리로드해 연다.
   *  규격화(240×320 / 4프레임 시트)는 편집기가 담당한다. */
  const generateWithCodex = async (kind: CodexGenKind) => {
    if (codexBusy || !editingAgent) return;
    const seq = ++codexSeqRef.current;
    const targetAgentId = editingAgent.id;
    /** 응답 적용 가능 여부: 토큰 유효 + 같은 에이전트의 편집 모달이 여전히 열려 있음. */
    const stillCurrent = () => {
      const m = useAppStore.getState().modal;
      return (
        codexSeqRef.current === seq && m.kind === "profile-edit" && m.agentId === targetAgentId
      );
    };
    setCodexBusy(kind);
    setCodexNote(null);
    try {
      const res = await tauriApi.generateCodexImage(
        appearancePrompt(kind, draft, { codex: true }),
      );
      if (!stillCurrent()) return;
      const url = `data:image/png;base64,${res.pngBase64}`;
      if (kind === "portrait") {
        setGeneratedPortrait(url);
        setEditorOpen(true);
      } else if (kind === "minimi") {
        setGeneratedMinimi(url);
        setMinimiEditorOpen(true);
      } else {
        setGeneratedImage(url);
        setSpriteEditorOpen(true);
      }
      setCodexNote(t("codex.generated"));
    } catch (err) {
      if (!stillCurrent()) return;
      setCodexNote(codexGenErrorCaption(err));
    } finally {
      if (codexSeqRef.current === seq) setCodexBusy(null);
    }
  };

  return {
    portraitUrl,
    spritePreviewUrl,
    minimiPreviewUrl,
    mode,
    setMode,
    editorOpen,
    setEditorOpen,
    spriteEditorOpen,
    setSpriteEditorOpen,
    minimiEditorOpen,
    setMinimiEditorOpen,
    generatedPortrait,
    setGeneratedPortrait,
    generatedImage,
    setGeneratedImage,
    generatedMinimi,
    setGeneratedMinimi,
    codexBusy,
    codexNote,
    copyPrompt,
    removeImage,
    generateWithCodex,
  };
}
