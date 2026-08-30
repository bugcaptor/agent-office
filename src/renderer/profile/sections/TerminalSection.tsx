// src/renderer/profile/sections/TerminalSection.tsx
//
// 터미널 — 시작 폴더·시작 명령어·셸, 그리고 이 캐릭터의 소리(키보드 사운드 팩,
// TTS 목소리). 앱 바깥 환경에 닿는 값만 모여 있다.
import { Trans, useTranslation } from "react-i18next";

import { KEYBOARD_SOUND_PACK_OPTIONS, packLabel } from "../../sound/packs";
import { previewKeyboardSound } from "../../sound/soundManager";
import { VoiceField } from "./VoiceField";
import type { DraftProfile } from "../generate";
import type { AvailableShell } from "@shared/types";
import type { Dispatch, SetStateAction } from "react";
import { IS_WINDOWS } from "../../shared/platform";

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
      <div className="form-field form-check">
        <label>
          <input
            type="checkbox"
            checked={draft.talkReceive !== false}
            onChange={(e) => setDraft({ ...draft, talkReceive: e.target.checked })}
          />
          <span className="form-label-text">{t("terminal.talkReceive")}</span>
        </label>
        {/* 문장 한가운데 <b>가 끼어 있어 키를 쪼개면 어순이 다른 언어에서
            말이 안 된다 — Trans로 태그째 번역한다. */}
        <p className="form-hint">
          <Trans t={t} i18nKey="terminal.talkReceiveHint" components={{ b: <b /> }} />
        </p>
      </div>
      <div className="form-field">
        <span className="form-label-text">{t("bot.section")}</span>
        <p className="form-hint">{t("bot.sectionHint")}</p>
      </div>
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("bot.slug")}</span>
          <input
            value={draft.botSlug ?? ""}
            onChange={(e) => setDraft({ ...draft, botSlug: e.target.value })}
            placeholder={t("bot.slugPlaceholder")}
          />
        </label>
        {/* <code>가 문장 중간이라 talkReceiveHint와 같은 이유로 Trans. */}
        <p className="form-hint">
          <Trans t={t} i18nKey="bot.slugHint" components={{ code: <code /> }} />
        </p>
      </div>
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("bot.whitelist")}</span>
          <input
            value={draft.botWhitelist ?? ""}
            onChange={(e) => setDraft({ ...draft, botWhitelist: e.target.value })}
            placeholder={t("bot.whitelistPlaceholder")}
          />
        </label>
        <p className="form-hint">{t("bot.whitelistHint")}</p>
      </div>
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("bot.poll")}</span>
          <input
            type="number"
            min={30}
            value={draft.botPollIntervalSec ?? ""}
            onChange={(e) => setDraft({ ...draft, botPollIntervalSec: e.target.value })}
            placeholder={t("bot.pollPlaceholder")}
          />
        </label>
      </div>
      {!IS_WINDOWS && (
        <div className="form-field form-check">
          <label>
            <input
              type="checkbox"
              checked={draft.tmuxHost === true}
              onChange={(e) => setDraft({ ...draft, tmuxHost: e.target.checked })}
            />
            <span className="form-label-text">{t("terminal.tmuxHost")}</span>
          </label>
          <p className="form-hint">{t("terminal.tmuxHostHint")}</p>
        </div>
      )}
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
