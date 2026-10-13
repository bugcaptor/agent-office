// src/renderer/profile/sections/TerminalSection.tsx
//
// 터미널 — 시작 폴더·시작 명령어·셸, 그리고 이 캐릭터의 소리(키보드 사운드 팩,
// TTS 목소리). 앱 바깥 환경에 닿는 값만 모여 있다.
import { useTranslation } from "react-i18next";

import { KEYBOARD_SOUND_PACK_OPTIONS, packLabel } from "../../sound/packs";
import { previewKeyboardSound } from "../../sound/soundManager";
import { WorkAgentFields } from "../../workpanels/WorkAgentFields";
import { VoiceField } from "./VoiceField";
import type { DraftProfile } from "../generate";
import type { AvailableShell } from "@shared/types";
import type { Dispatch, SetStateAction } from "react";

export function TerminalSection({
  draft,
  setDraft,
  shells,
  onBrowseCwd,
  editingAgentId,
}: {
  draft: DraftProfile;
  setDraft: Dispatch<SetStateAction<DraftProfile>>;
  shells: AvailableShell[];
  onBrowseCwd: () => void;
  editingAgentId: string | undefined;
}) {
  const { t } = useTranslation("profile");
  return (
    <>
    {/* ── 터미널: 시작 폴더 · 시작 명령어 · 셸 ─────────────── */}
    <section className="form-section">
      <h3 className="form-section-title">{t("terminal.section")}</h3>
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("terminal.cwd")}</span>
          <div className="form-control-row">
            <input
              value={draft.cwd ?? ""}
              onChange={(e) => setDraft({ ...draft, cwd: e.target.value })}
              placeholder={t("terminal.cwdPlaceholder")}
            />
            <button type="button" className="pixel-btn" onClick={onBrowseCwd}>
              {t("terminal.browse")}
            </button>
          </div>
        </label>
      </div>
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("terminal.startupCommand")}</span>
          <input
            value={draft.startupCommand ?? ""}
            onChange={(e) => setDraft({ ...draft, startupCommand: e.target.value })}
            placeholder={t("terminal.startupCommandPlaceholder")}
          />
        </label>
        <p className="form-hint">{t("terminal.startupCommandHint")}</p>
      </div>
      <div className="form-field"><WorkAgentFields profile value={draft.defaultAgent} onChange={(defaultAgent) => setDraft((d) => ({ ...d, defaultAgent }))} /></div>
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("terminal.keyboardSound")}</span>
          <select
            value={draft.keyboardSound ?? ""}
            onChange={(e) => {
              setDraft({ ...draft, keyboardSound: e.target.value });
              previewKeyboardSound(e.target.value || undefined, editingAgentId);
            }}
          >
            <option value="">{t("terminal.keyboardSoundDefault")}</option>
            {KEYBOARD_SOUND_PACK_OPTIONS.map((p) => (
              <option key={p.id} value={p.id}>{packLabel(p, t)}</option>
            ))}
          </select>
        </label>
        <p className="form-hint">{t("terminal.keyboardSoundHint")}</p>
      </div>
      <VoiceField
        draft={draft}
        agentId={editingAgentId}
        onChange={(voiceId) => setDraft((d) => ({ ...d, voiceId }))}
      />
      {shells.length > 0 && (
        <div className="form-field">
          <label>
            <span className="form-label-text">{t("terminal.shell")}</span>
            <select
              value={draft.shell ?? ""}
              onChange={(e) => setDraft({ ...draft, shell: e.target.value })}
            >
              <option value="">{t("terminal.shellAuto")}</option>
              {shells.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.hooksSupported
                    ? s.label
                    : t("terminal.shellNoHooks", { label: s.label })}
                </option>
              ))}
            </select>
          </label>
        </div>
      )}
    </section>
    </>
  );
}
