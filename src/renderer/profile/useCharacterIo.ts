// src/renderer/profile/useCharacterIo.ts
//
// 캐릭터 내보내기·가져오기(이슈 #77).
//
// 내보내기: 현재 편집 draft(진행 중 편집 반영) + 백엔드에 저장된 초상/스프라이트/
// 미니미를 모아 자기완결형 번들 파일로 쓴다. 로컬 환경 필드(cwd/셸/시작명령/봇)는
// 제외한다 — 남의 기계에서 의미가 없고, 남의 경로를 실어 보낼 이유도 없다.
//
// 가져오기: **편집 중인 캐릭터에 적용**한다(새 캐릭터를 만들지 않는다). 이미지는
// replace 시맨틱이다 — 번들에 있으면 저장·표시, 없으면 기존 이미지를 제거해 소스와
// 똑같은 외형이 되게 한다. 텍스트 필드는 draft에 실어 '저장'에서 확정한다.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { pngBase64ToDataUrl } from "../portrait/portraitCache";
import { loadSpritesFor } from "../sprite/spriteCache";
import { loadMinimisFor } from "../sprite/minimiCache";
import { clearSpriteOverride } from "../office/gen/spriteOverrides";
import { clearMinimiOverride } from "../office/gen/minimiOverrides";
import {
  applyBundleToDraft,
  buildExportFileName,
  portableFromDraft,
  serializeBundle,
} from "./characterIo";
import { parseCharacterBundle, type CharacterBundleError } from "@shared/types";
import type { AgentProfile } from "@shared/types";
import type { DraftProfile } from "./generate";
import type { Dispatch, SetStateAction } from "react";

/** `parseCharacterBundle`이 돌려주는 실패 코드 → `common` 카탈로그 키.
 *  파서는 `src/shared`에 있어 renderer의 `t()`를 부를 수 없으므로(그리고 문구는
 *  그리는 쪽 언어를 따라야 하므로) 번역은 여기서 한다 — SummarySection의
 *  `SUMMARY_TEST_ERROR_KEY`와 같은 관례. `Record<CharacterBundleError, …>`라
 *  코드가 늘면 타입 검사에서 걸린다. */
const BUNDLE_ERROR_KEY: Record<CharacterBundleError, string> = {
  "bundle-not-json": "common:errors.bundleNotJson",
  "bundle-not-character-file": "common:errors.bundleNotCharacterFile",
  "bundle-schema-version-missing": "common:errors.bundleSchemaVersionMissing",
  "bundle-schema-version-newer": "common:errors.bundleSchemaVersionNewer",
  "bundle-schema-version-unsupported": "common:errors.bundleSchemaVersionUnsupported",
  "bundle-profile-missing": "common:errors.bundleProfileMissing",
  "bundle-profile-name-missing": "common:errors.bundleProfileNameMissing",
  "bundle-portrait-invalid": "common:errors.bundlePortraitInvalid",
  "bundle-portrait-too-large": "common:errors.bundlePortraitTooLarge",
  "bundle-sprite-invalid": "common:errors.bundleSpriteInvalid",
  "bundle-sprite-too-large": "common:errors.bundleSpriteTooLarge",
  "bundle-minimi-invalid": "common:errors.bundleMinimiInvalid",
  "bundle-minimi-too-large": "common:errors.bundleMinimiTooLarge",
};

export function useCharacterIo(deps: {
  draft: DraftProfile;
  setDraft: Dispatch<SetStateAction<DraftProfile>>;
  editingAgent: AgentProfile | undefined;
  editingAgentId: string | undefined;
}) {
  const { draft, setDraft, editingAgent, editingAgentId } = deps;
  const { t } = useTranslation("profile");

  const updateAgent = useAppStore((s) => s.updateAgent);
  const setPortrait = useAppStore((s) => s.setPortrait);
  const removePortrait = useAppStore((s) => s.removePortrait);
  const removeSpritePreview = useAppStore((s) => s.removeSpritePreview);
  const removeMinimiPreview = useAppStore((s) => s.removeMinimiPreview);

  /** 진행 표시 + 결과/오류 캡션. */
  const [ioBusy, setIoBusy] = useState(false);
  const [ioNote, setIoNote] = useState<string | null>(null);

  // 편집 세션이 바뀌면 이전 세션의 캡션·busy를 끌고 가지 않는다.
  useEffect(() => {
    setIoBusy(false);
    setIoNote(null);
  }, [editingAgentId]);

  const onExportCharacter = async () => {
    if (ioBusy || !editingAgent) return;
    setIoBusy(true);
    setIoNote(null);
    try {
      const profile = portableFromDraft(draft);
      const [portraitB64, spriteB64, minimiB64] = await Promise.all([
        tauriApi.loadPortrait(editingAgent.id),
        tauriApi.loadSprite(editingAgent.id),
        tauriApi.loadMinimi(editingAgent.id),
      ]);
      const json = serializeBundle(
        profile,
        portraitB64 ?? undefined,
        spriteB64 ?? undefined,
        minimiB64 ?? undefined,
      );
      const saved = await tauriApi.exportCharacterFile(buildExportFileName(profile.name), json);
      setIoNote(saved ? t("io.exported") : null); // null=사용자가 취소
    } catch (err) {
      console.warn("ProfileDialog: exportCharacter failed", err);
      setIoNote(t("io.exportFailed"));
    } finally {
      setIoBusy(false);
    }
  };

  const onImportCharacter = async () => {
    if (ioBusy || !editingAgent) return;
    setIoBusy(true);
    setIoNote(null);
    const id = editingAgent.id;
    try {
      const text = await tauriApi.importCharacterFile();
      if (text == null) {
        setIoBusy(false);
        return; // 사용자가 취소
      }
      const res = parseCharacterBundle(text);
      if (!res.ok) {
        setIoNote(t(BUNDLE_ERROR_KEY[res.error]));
        setIoBusy(false);
        return;
      }
      const b = res.bundle;

      if (b.portraitPngBase64) {
        await tauriApi.savePortrait(id, b.portraitPngBase64);
        setPortrait(id, pngBase64ToDataUrl(b.portraitPngBase64));
        updateAgent(id, { portraitUpdatedAt: Date.now() });
      } else {
        await tauriApi.deletePortrait(id);
        removePortrait(id);
        updateAgent(id, { portraitUpdatedAt: undefined });
      }

      if (b.spritePngBase64) {
        await tauriApi.saveSprite(id, b.spritePngBase64);
        updateAgent(id, { spriteUpdatedAt: Date.now() });
        await loadSpritesFor([id]); // 백엔드에서 디코드 → 오버라이드 + 프리뷰 갱신
      } else {
        await tauriApi.deleteSprite(id);
        clearSpriteOverride(id);
        removeSpritePreview(id);
        updateAgent(id, { spriteUpdatedAt: undefined });
      }

      // 미니미도 스프라이트와 같은 replace 시맨틱(번들에 없으면 제거).
      if (b.minimiPngBase64) {
        await tauriApi.saveMinimi(id, b.minimiPngBase64);
        updateAgent(id, { minimiUpdatedAt: Date.now() });
        await loadMinimisFor([id]); // 백엔드에서 디코드 → 오버라이드 + 프리뷰 갱신
      } else {
        await tauriApi.deleteMinimi(id);
        clearMinimiOverride(id);
        removeMinimiPreview(id);
        updateAgent(id, { minimiUpdatedAt: undefined });
      }

      setDraft((d) => applyBundleToDraft(d, b.profile));
      setIoNote(t("io.imported"));
    } catch (err) {
      console.warn("ProfileDialog: importCharacter failed", err);
      setIoNote(t("io.importFailed"));
    } finally {
      setIoBusy(false);
    }
  };

  return { ioBusy, ioNote, onExportCharacter, onImportCharacter };
}
