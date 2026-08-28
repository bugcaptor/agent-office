// src/renderer/profile/sections/AppearanceSection.tsx
//
// 외형 — 프리뷰 카드 3장(초상·스프라이트·미니미), 키 컬러 칩, 그리고 그림을
// 얻는 두 경로(직접 만들기 / Codex 생성)와 종류별 추가 의뢰문.
//
// 만드는 방법은 한 번에 하나만 보여 준다 — 두 경로를 나란히 두면 두서없이 보인다.
// 이미지 조작 자체는 `useAppearanceImages`가 종류를 인자로 받아 처리하고, 여기는
// 그 결과를 그린다.
import { useTranslation } from "react-i18next";

import { hexColor } from "../../office/gen/archetypes";
import { CodexGenPanel } from "../../portrait/CodexGenPanel";
import type { KeyColor } from "../../office/gen/archetypes";
import type { useAppearanceImages } from "../useAppearanceImages";
import type { DraftProfile } from "../generate";
import type { AgentProfile, PaletteSlot } from "@shared/types";
import type { Dispatch, SetStateAction } from "react";

export function AppearanceSection({
  draft,
  setDraft,
  editing,
  editingAgent,
  spriteUrl,
  keyColors,
  onPickSlot,
  regenSeed,
  images,
}: {
  draft: DraftProfile;
  setDraft: Dispatch<SetStateAction<DraftProfile>>;
  editing: boolean;
  editingAgent: AgentProfile | undefined;
  spriteUrl: string;
  keyColors: KeyColor[];
  onPickSlot: (slot: PaletteSlot) => void;
  regenSeed: () => void;
  images: ReturnType<typeof useAppearanceImages>;
}) {
  const { t } = useTranslation("profile");
  // 이미지 3종 상태를 JSX가 읽던 이름 그대로 푼다.
  const {
    portraitUrl,
    spritePreviewUrl,
    minimiPreviewUrl,
    mode: appearanceMode,
    setMode: setAppearanceMode,
    setEditorOpen,
    setSpriteEditorOpen,
    setMinimiEditorOpen,
    codexBusy,
    codexNote,
  } = images;
  const setPickingSlot = onPickSlot;
  const onGenerateCodex = images.generateWithCodex;
  const onCopyPrompt = () => void images.copyPrompt("portrait");
  const onCopySpritePrompt = () => void images.copyPrompt("sprite");
  const onCopyMinimiPrompt = () => void images.copyPrompt("minimi");
  const onRemovePortrait = () => void images.removeImage("portrait");
  const onRemoveSprite = () => void images.removeImage("sprite");
  const onRemoveMinimi = () => void images.removeImage("minimi");
  return (
    <>
    {/* ── 외형: 프리뷰 카드 + 키 컬러 + 초상화/스프라이트/미니미 추가 프롬프트 ── */}
    <section className="form-section">
      <h3 className="form-section-title">{t("appearance.section")}</h3>
      <div className="profile-previews">
        <div className="portrait-section">
          <span className="preview-card-title">{t("portrait.label")}</span>
          <div className="portrait-current">
            <img
              // 호버 카드와 동일한 폴백 체인(초상 > 커스텀 스프라이트 프리뷰 >
              // 프로시저럴) — spritePreviewUrl 누락 시 스프라이트 생성 후에도
              // 생성 전 프로시저럴 이미지가 잔존하는 버그.
              src={portraitUrl ?? spritePreviewUrl ?? spriteUrl}
              alt="portrait"
              width={90}
              height={120}
              // 초상은 부드럽게 축소(240×320 → 90×120), 스프라이트 폴백은 nearest 확대.
              style={{
                objectFit: "cover",
                objectPosition: "top center",
                imageRendering: portraitUrl ? "auto" : "pixelated",
              }}
            />
          </div>
          <div className="portrait-buttons">
            {editing && editingAgent && portraitUrl && (
              <button className="pixel-btn" onClick={onRemovePortrait}>
                {t("portrait.remove")}
              </button>
            )}
          </div>
        </div>
        <div className="sprite-preview">
          <span className="preview-card-title">{t("sprite.label")}</span>
          <img
            src={spritePreviewUrl ?? spriteUrl}
            alt="sprite"
            width={96}
            height={96}
          />
          <div className="sprite-buttons">
            <button className="pixel-btn" onClick={regenSeed}>
              {t("sprite.regen")}
            </button>
            {spritePreviewUrl && (
              <span className="sprite-custom-badge">{t("sprite.customBadge")}</span>
            )}
            {editing && editingAgent && spritePreviewUrl && (
              <button className="pixel-btn" onClick={onRemoveSprite}>
                {t("sprite.removeCustom")}
              </button>
            )}
          </div>
          {/* 서브에이전트 미니미 — 머리 옆에 뜨는 작은 분신. 지정이 없으면
              스프라이트를 그대로 축소해 쓴다. */}
          <div className="minimi-subsection">
            <span className="preview-card-title">{t("minimi.label")}</span>
            <img
              src={minimiPreviewUrl ?? spritePreviewUrl ?? spriteUrl}
              alt="minimi"
              width={48}
              height={48}
              style={{ imageRendering: "pixelated" }}
            />
            <span className="sprite-custom-badge">
              {minimiPreviewUrl ? t("minimi.customBadge") : t("minimi.emptyBadge")}
            </span>
            {editing && editingAgent && (
              <div className="sprite-buttons">
                <button className="pixel-btn" onClick={() => setMinimiEditorOpen(true)}>
                  {minimiPreviewUrl ? t("minimi.change") : t("minimi.upload")}
                </button>
                {minimiPreviewUrl && (
                  <button className="pixel-btn" onClick={onRemoveMinimi}>
                    {t("minimi.remove")}
                  </button>
                )}
              </div>
            )}
          </div>
        </div>
      </div>

      {/* 키 컬러: 프롬프트에 그대로 실리는 색. 기본값은 시드(+아키타입)가
          정하지만, 칩을 누르면 그 색만 따로 골라 덮어쓸 수 있다(kbm #2fj) —
          색 하나 때문에 시드를 통째로 다시 뽑지 않아도 된다. */}
      <div className="key-colors">
        <span className="form-label-text">{t("keyColor.label")}</span>
        <ul className="key-color-list">
          {keyColors.map((c) => {
            const custom = Boolean(draft.colors?.[c.slot]);
            return (
              <li key={c.en}>
                <button
                  type="button"
                  className={custom ? "key-color key-color-custom" : "key-color"}
                  title={t("keyColor.chipTitle", { name: c.en, hex: hexColor(c.rgb) })}
                  onClick={() => setPickingSlot(c.slot)}
                >
                  <span
                    className="key-color-chip"
                    style={{ background: hexColor(c.rgb) }}
                    aria-hidden
                  />
                  <span>{t(c.labelKey)}</span>
                  <code>{hexColor(c.rgb)}</code>
                  {custom && (
                    <span className="key-color-mark" title={t("keyColor.customMark")}>
                      ●
                    </span>
                  )}
                </button>
              </li>
            );
          })}
        </ul>
        <p className="form-hint">{t("keyColor.hint")}</p>
      </div>

      {/* 만드는 방법은 한 번에 하나만 — 직접 만들기와 Codex 생성을 나란히
          늘어놓으면 무엇을 눌러야 할지 알 수 없다. SettingsDialog와 같은
          tablist 관례를 작은 크기로 재사용한다. */}
      <div
        className="appearance-tabs"
        role="tablist"
        aria-label={t("appearance.tablistLabel")}
      >
        {/* 라벨은 키로 들고 렌더 시점에 번역한다 — 상수 배열에서 t()를 미리
            부르면 언어를 바꿔도 문구가 그대로 남는다. */}
        {([
          { id: "manual", labelKey: "appearance.tabManual" },
          { id: "codex", labelKey: "appearance.tabCodex" },
        ] as const).map((tab) => (
          <button
            key={tab.id}
            type="button"
            role="tab"
            id={`appearance-tab-${tab.id}`}
            aria-selected={appearanceMode === tab.id}
            aria-controls={`appearance-tabpanel-${tab.id}`}
            className={
              appearanceMode === tab.id
                ? "appearance-tab appearance-tab-active"
                : "appearance-tab"
            }
            onClick={() => setAppearanceMode(tab.id)}
          >
            {t(tab.labelKey)}
          </button>
        ))}
      </div>
      <div
        className="appearance-tabpanel"
        role="tabpanel"
        id={`appearance-tabpanel-${appearanceMode}`}
        aria-labelledby={`appearance-tab-${appearanceMode}`}
      >
        {appearanceMode === "manual" ? (
          <>
            <p className="form-hint">{t("appearance.manualHint")}</p>
            <div className="appearance-manual-row">
              <span className="form-label-text">{t("portrait.label")}</span>
              <div className="portrait-buttons">
                <button className="pixel-btn" onClick={onCopyPrompt}>
                  {t("portrait.copyPrompt")}
                </button>
                {editing && editingAgent && (
                  <button className="pixel-btn" onClick={() => setEditorOpen(true)}>
                    {portraitUrl ? t("portrait.change") : t("portrait.upload")}
                  </button>
                )}
              </div>
            </div>
            <div className="appearance-manual-row">
              <span className="form-label-text">{t("sprite.label")}</span>
              <div className="sprite-buttons">
                <button className="pixel-btn" onClick={onCopySpritePrompt}>
                  {t("sprite.copyPrompt")}
                </button>
                {editing && editingAgent && (
                  <button className="pixel-btn" onClick={() => setSpriteEditorOpen(true)}>
                    {spritePreviewUrl ? t("sprite.change") : t("sprite.upload")}
                  </button>
                )}
              </div>
            </div>
            <div className="appearance-manual-row">
              <span className="form-label-text">{t("minimi.label")}</span>
              <div className="sprite-buttons">
                <button className="pixel-btn" onClick={onCopyMinimiPrompt}>
                  {t("minimi.copyPrompt")}
                </button>
                {editing && editingAgent && (
                  <button className="pixel-btn" onClick={() => setMinimiEditorOpen(true)}>
                    {minimiPreviewUrl ? t("minimi.change") : t("minimi.upload")}
                  </button>
                )}
              </div>
            </div>
            {!(editing && editingAgent) && (
              <p className="form-hint">{t("appearance.saveFirstHint")}</p>
            )}
          </>
        ) : (
          <CodexGenPanel
            enabled={Boolean(editing && editingAgent)}
            busy={codexBusy}
            note={codexNote}
            onGenerate={onGenerateCodex}
          />
        )}
      </div>
      {/* 세 프롬프트 칸은 각자 자기 그림에만 덧붙는다 — 예전처럼 한 칸이
          다른 그림의 폴백이 되지 않는다(칸 사이 관계를 없애 헷갈림 제거). */}
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("portrait.requestLabel")}</span>
          <input
            value={draft.portraitRequest ?? ""}
            onChange={(e) => setDraft({ ...draft, portraitRequest: e.target.value })}
            placeholder={t("portrait.requestPlaceholder")}
          />
        </label>
        <p className="form-hint">{t("portrait.requestHint")}</p>
      </div>
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("sprite.requestLabel")}</span>
          <input
            value={draft.spriteRequest ?? ""}
            onChange={(e) => setDraft({ ...draft, spriteRequest: e.target.value })}
            placeholder={t("sprite.requestPlaceholder")}
          />
        </label>
        <p className="form-hint">{t("sprite.requestHint")}</p>
      </div>
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("minimi.requestLabel")}</span>
          <input
            value={draft.minimiRequest ?? ""}
            onChange={(e) => setDraft({ ...draft, minimiRequest: e.target.value })}
            placeholder={t("minimi.requestPlaceholder")}
          />
        </label>
        <p className="form-hint">{t("minimi.requestHint")}</p>
      </div>
    </section>
    </>
  );
}
