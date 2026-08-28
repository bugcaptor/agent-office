// src/renderer/profile/sections/IdentitySection.tsx
//
// 정체성 — 이름·역할·성격 프롬프트·아키타입. 다이얼로그에서 유일하게 "이 캐릭터가
// 누구인가"만 다루는 칸이라, 외형(색·그림)과 터미널(환경)은 여기 없다.
import { useTranslation } from "react-i18next";

import { useAwardsStore } from "../../awards/awardsStore";
import { ArchetypePicker } from "../ArchetypePicker";
import type { DraftProfile } from "../generate";
import type { AgentProfile } from "@shared/types";
import type { Dispatch, SetStateAction } from "react";

export function IdentitySection({
  draft,
  setDraft,
  editing,
  editingAgent,
}: {
  draft: DraftProfile;
  setDraft: Dispatch<SetStateAction<DraftProfile>>;
  editing: boolean;
  editingAgent: AgentProfile | undefined;
}) {
  const { t } = useTranslation("profile");
  // "이 달의 우수사원" 통산 수상 횟수 뱃지(docs/employee-of-the-month-design.md
  // §6) — 스토어에서 count만 읽는 소규모 표시. 0회면 뱃지를 그리지 않는다.
  const awardCountFor = useAwardsStore((s) => s.awardCountFor);
  return (
    <>
    {/* ── 정체성: 이름 · 역할 · 성격 프롬프트 · 아키타입 ────── */}
    <section className="form-section">
      <h3 className="form-section-title">{t("identity.section")}</h3>
      <div className="form-row-2">
        <div className="form-field">
          <label>
            <span className="form-label-text">{t("identity.name")}</span>
            <input
              value={draft.name}
              onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            />
          </label>
        </div>
        <div className="form-field">
          <label>
            <span className="form-label-text">{t("identity.role")}</span>
            <input
              value={draft.role}
              onChange={(e) => setDraft({ ...draft, role: e.target.value })}
            />
          </label>
        </div>
      </div>
      {editing && editingAgent && awardCountFor(editingAgent.id) > 0 && (
        <p className="profile-award-badge">
          {t("identity.awardBadge", { n: awardCountFor(editingAgent.id) })}
        </p>
      )}
      {/* 예전의 '메모'와 '성격 프롬프트'를 하나로 통합했다 — 둘의 차이가
          헷갈렸고 실제로 같은 것(캐릭터가 어떤 존재인가)을 적는 칸이었다.
          기존 메모는 편집기를 열 때 이 칸에 합쳐진다. */}
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("identity.personality")}</span>
          <textarea
            value={draft.personalityPrompt ?? ""}
            onChange={(e) => setDraft({ ...draft, personalityPrompt: e.target.value })}
            rows={4}
          />
        </label>
        <p className="form-hint">{t("identity.personalityHint")}</p>
      </div>
      <div className="form-field">
        <label>
          <span className="form-label-text">{t("identity.archetype")}</span>
          <ArchetypePicker
            value={draft.archetype ?? "auto"}
            onChange={(v) => setDraft({ ...draft, archetype: v })}
          />
        </label>
        <p className="form-hint">{t("identity.archetypeHint")}</p>
      </div>
    </section>
    </>
  );
}
