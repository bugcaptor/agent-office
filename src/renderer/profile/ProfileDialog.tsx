// src/renderer/profile/ProfileDialog.tsx
//
// 캐릭터 생성/편집 다이얼로그의 **뼈대**. 초안(draft)을 세 훅에 맡기고, 화면은
// 세 섹션 컴포넌트에 맡긴 뒤, 이 파일은 그 사이를 잇고 저장을 확정한다.
//
//   useProfileDraft      — 초안 상태 + 편집 모드 진입 시 기존 값 로드 + 파생 색
//   useAppearanceImages  — 초상·스프라이트·미니미 3종의 생성·삭제·프롬프트
//   useCharacterIo       — 캐릭터 번들 내보내기/가져오기(이슈 #77)
//   sections/            — 정체성 / 외형 / 터미널 세 칸의 JSX
//
// 저장(onSave): 초안을 정규화해 `addAgent`(스토어, 세션을 `starting`으로 씨앗)
// → `tauriApi.createSession`(PTY 시작) → 닫기. 편집 모드는 기존 프로필을 제자리에서
// 갱신하고 새 세션을 시작하지 않는다.
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { sessionOptsFor } from "../ipc/sessionOpts";
import { hexColor, pickArchetype } from "../office/gen/archetypes";
import type { KeyColor } from "../office/gen/archetypes";
import { buildBotConfig, draftToProfile, normalizeColors } from "./generate";
import { useProfileDraft } from "./useProfileDraft";
import { useAppearanceImages } from "./useAppearanceImages";
import { useCharacterIo } from "./useCharacterIo";
import { ColorPickerDialog } from "./ColorPickerDialog";
import { AppearanceSection } from "./sections/AppearanceSection";
import { IdentitySection } from "./sections/IdentitySection";
import { TerminalSection } from "./sections/TerminalSection";
import { PortraitEditor } from "../portrait/PortraitEditor";
import { SpriteEditor } from "../sprite/SpriteEditor";
import type { PaletteSlot } from "@shared/types";
import "../portrait/portrait.css";

export function ProfileDialog() {
  const { t } = useTranslation("profile");
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const addAgent = useAppStore((s) => s.addAgent);
  const updateAgent = useAppStore((s) => s.updateAgent);
  const setSessionState = useAppStore((s) => s.setSessionState);
  const agentOrder = useAppStore((s) => s.agentOrder);

  const editing = modal.kind === "profile-edit";
  const editingAgentId = modal.kind === "profile-edit" ? modal.agentId : undefined;
  const editingAgent = useAppStore((s) =>
    editingAgentId ? s.agents[editingAgentId] : undefined
  );
  /** 컬러 피커를 연 슬롯. null이면 닫힘. 슬롯이 곧 다이얼로그의 `key`다. */
  const [pickingSlot, setPickingSlot] = useState<PaletteSlot | null>(null);

  const {
    draft,
    setDraft,
    spriteUrl,
    shells,
    keyColors,
    basePalette,
    setSlotColor,
    regenSeed,
    regenAll,
    onBrowseCwd,
  } = useProfileDraft(editingAgentId);

  // 외형 이미지 3종은 다루는 절차가 같아 한 훅이 종류를 인자로 받아 처리한다.
  // 결과는 AppearanceSection이 통째로 받아 자기 안에서 풀어 쓴다.
  const images = useAppearanceImages({ draft, editingAgent, editingAgentId });

  const { ioBusy, ioNote, onExportCharacter, onImportCharacter } = useCharacterIo({
    draft,
    setDraft,
    editingAgent,
    editingAgentId,
  });

  /** 색 칩·피커 제목에 쓸 슬롯 이름. 키 컬러 데이터는 번역 키만 들고 있으므로
   *  (아키타입마다 "머리"/"털"/"본체"로 이름이 다르다) 여기서 번역한다. */
  const keyColorLabel = (c: KeyColor | undefined): string =>
    c ? t(c.labelKey) : t("keyColor.fallbackLabel");

  /** 피커가 떠 있는 슬롯의 정보(라벨·현재색·기본색). 닫혀 있으면 null. */
  const picking = pickingSlot
    ? {
        slot: pickingSlot,
        label: keyColorLabel(keyColors.find((c) => c.slot === pickingSlot)),
        value: hexColor(
          keyColors.find((c) => c.slot === pickingSlot)?.rgb ?? basePalette[pickingSlot].base,
        ),
        defaultValue: hexColor(basePalette[pickingSlot].base),
        overridden: Boolean(draft.colors?.[pickingSlot]),
      }
    : null;


  const onSave = async () => {
    if (editing && editingAgent) {
      const trimmedCwd = (draft.cwd ?? "").trim();
      const trimmedShell = (draft.shell ?? "").trim();
      const trimmedStartupCommand = (draft.startupCommand ?? "").trim();
      const trimmedPersonalityPrompt = (draft.personalityPrompt ?? "").trim();
      const trimmedPortraitRequest = (draft.portraitRequest ?? "").trim();
      const trimmedSpriteRequest = (draft.spriteRequest ?? "").trim();
      const trimmedMinimiRequest = (draft.minimiRequest ?? "").trim();
      const trimmedKeyboardSound = (draft.keyboardSound ?? "").trim();
      const trimmedVoiceId = (draft.voiceId ?? "").trim();
      // 목록에 없는 자유 입력도 그대로 저장한다(공백만 다듬는다) — 스프라이트는
      // human으로 폴백하고 그림 프롬프트에는 적은 문구가 들어간다.
      const trimmedArchetype = (draft.archetype ?? "").trim();
      const chosenArchetype =
        trimmedArchetype && trimmedArchetype !== "auto"
          ? trimmedArchetype
          : pickArchetype(draft.seed);
      updateAgent(editingAgent.id, {
        name: draft.name,
        role: draft.role,
        seed: draft.seed,
        archetype: chosenArchetype,
        colors: normalizeColors(draft.colors),
        cwd: trimmedCwd || undefined,
        shell: trimmedShell || undefined,
        startupCommand: trimmedStartupCommand || undefined,
        personalityPrompt: trimmedPersonalityPrompt || undefined,
        portraitRequest: trimmedPortraitRequest || undefined,
        spriteRequest: trimmedSpriteRequest || undefined,
        minimiRequest: trimmedMinimiRequest || undefined,
        keyboardSound: trimmedKeyboardSound || undefined,
        voiceId: trimmedVoiceId || undefined,
        bot: buildBotConfig(draft),
        // 기본(수신 허용)은 필드를 지운다 — 끈 경우만 false로 남긴다.
        talkReceive: draft.talkReceive === false ? false : undefined,
      });
    } else {
      const profile = draftToProfile(draft, agentOrder.length);
      addAgent(profile); // status: 'starting'
      // 캐릭터 등장은 profiles prop 변화 → B의 syncAgents가 처리 (정합화)
      try {
        await tauriApi.createSession(profile.id, sessionOptsFor(profile)); // PTY 시작
      } catch (err) {
        // The profile is already saved; mark the session exited so clicking the
        // character later retries via the bridge's ensureSession.
        setSessionState({ agentId: profile.id, status: "exited" });
        console.warn(`ProfileDialog: createSession failed for ${profile.id}`, err);
      }
    }
    closeModal();
  };

  if (modal.kind !== "profile-create" && modal.kind !== "profile-edit") return null;
  return (
    <div
      className="modal-backdrop"
      // mousedown + target guard (not onClick), mirroring TerminalOverlay's
      // backdrop close (commit 7986f3d): PortraitEditor renders nested
      // inside this backdrop, and a plain onClick={closeModal} here would
      // catch every bubbled synthetic click from the nested editor (its
      // 저장/취소 buttons, 레트로 필터 checkbox, file input), closing this
      // dialog underneath it. mousedown fires at the press point and the
      // target === currentTarget check only matches an actual press on the
      // backdrop itself, never a bubbled event from a descendant — so
      // nested-editor interactions (and the retargeted click that follows
      // the editor's own backdrop-mousedown unmount) never reach this
      // handler at all.
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel profile-dialog">
        {/* ── 헤더 ─────────────────────────────────────────────── */}
        <header className="profile-dialog-header">
          <h2 className="pixel-title">
            {editing ? t("dialog.titleEdit") : t("dialog.titleCreate")}
          </h2>
          <p className="profile-dialog-sub">
            {editing ? t("dialog.subEdit") : t("dialog.subCreate")}
          </p>
        </header>

        <IdentitySection
          draft={draft}
          setDraft={setDraft}
          editing={editing}
          editingAgent={editingAgent}
        />

        <AppearanceSection
          draft={draft}
          setDraft={setDraft}
          editing={editing}
          editingAgent={editingAgent}
          spriteUrl={spriteUrl}
          keyColors={keyColors}
          onPickSlot={setPickingSlot}
          regenSeed={regenSeed}
          images={images}
        />

        <TerminalSection
          draft={draft}
          setDraft={setDraft}
          shells={shells}
          onBrowseCwd={onBrowseCwd}
          editingAgentId={editingAgentId}
        />

        {/* ── 액션 ─────────────────────────────────────────────── */}
        <div className="dialog-actions">
          {!editing && (
            <button className="pixel-btn dialog-action-aux" onClick={regenAll}>
              {t("dialog.randomizeAll")}
            </button>
          )}
          {editing && editingAgent && (
            <div className="dialog-io-group">
              <button
                className="pixel-btn"
                onClick={onExportCharacter}
                disabled={ioBusy}
              >
                {t("io.export")}
              </button>
              <button
                className="pixel-btn"
                onClick={onImportCharacter}
                disabled={ioBusy}
              >
                {t("io.import")}
              </button>
              {ioNote && <span className="profile-io-note">{ioNote}</span>}
            </div>
          )}
          <button className="pixel-btn primary" onClick={onSave}>
            {t("dialog.save")}
          </button>
          <button className="pixel-btn" onClick={closeModal}>
            {t("dialog.cancel")}
          </button>
        </div>
      </div>
      {images.editorOpen && editingAgent && (
        <PortraitEditor
          agentId={editingAgent.id}
          initialImage={images.generatedPortrait ?? undefined}
          onClose={() => {
            images.setEditorOpen(false);
            images.setGeneratedPortrait(null);
          }}
        />
      )}
      {images.spriteEditorOpen && editingAgent && (
        <SpriteEditor
          agentId={editingAgent.id}
          initialImage={images.generatedImage ?? undefined}
          onClose={() => {
            images.setSpriteEditorOpen(false);
            images.setGeneratedImage(null);
          }}
        />
      )}
      {images.minimiEditorOpen && editingAgent && (
        <SpriteEditor
          agentId={editingAgent.id}
          target="minimi"
          initialImage={images.generatedMinimi ?? undefined}
          onClose={() => {
            images.setMinimiEditorOpen(false);
            images.setGeneratedMinimi(null);
          }}
        />
      )}
      {picking && (
        <ColorPickerDialog
          // 슬롯이 곧 정체성 — 다른 칩을 열면 새로 마운트돼 초기 색이 다시 잡힌다.
          key={picking.slot}
          label={picking.label}
          value={picking.value}
          defaultValue={picking.defaultValue}
          overridden={picking.overridden}
          onApply={(hex) => setSlotColor(picking.slot, hex)}
          onReset={() => setSlotColor(picking.slot, null)}
          onClose={() => setPickingSlot(null)}
        />
      )}
    </div>
  );
}
