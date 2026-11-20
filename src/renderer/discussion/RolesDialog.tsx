// src/renderer/discussion/RolesDialog.tsx
//
// 토의 역할 목록 편집(추가·수정·삭제·복제·프리셋 추가). `modal.kind === "roles"`로
// self-gate한다. 편집은 사본에서 하고 저장을 누를 때 목록 전체를 쓴다.
//
// 지운 역할을 기본 역할로 쓰던 프로필은 그대로 둔다 — 없는 id는 "역할 없음"으로 읽는다.
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { DISCUSSION_STANCES, type DiscussionRole } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import { WorkAgentFields } from "../workpanels/WorkAgentFields";
import { normalizeWorkAgent } from "../profile/generate";
import { ROLE_PRESETS, newRoleId, normalizeRoles, presetRole, presetRoleId, useRoleStore } from "./roleStore";
import "./discussion.css";

export function RolesDialog() {
  const open = useAppStore((s) => s.modal.kind === "roles");
  return open ? <RolesEditor /> : null;
}

function RolesEditor() {
  const { t } = useTranslation("discussion");
  const closeModal = useAppStore((s) => s.closeModal);
  const agents = useAppStore((s) => s.agents);
  const status = useRoleStore((s) => s.status);
  const stored = useRoleStore((s) => s.roles);
  const [draft, setDraft] = useState<DiscussionRole[] | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState(false);
  useEscapeToClose(true, closeModal);

  useEffect(() => { void useRoleStore.getState().ensureLoaded(); }, []);
  // 창이 열린 동안 역할 수신함(CLI 변경)을 미룬다. 닫은 뒤 저장된 목록 위에 적용된다.
  useEffect(() => {
    useRoleStore.setState({ editing: true });
    return () => useRoleStore.setState({ editing: false });
  }, []);
  // 처음 불러온 목록으로 한 번만 사본을 만든다(이후 저장소 변화가 편집을 덮지 않게).
  useEffect(() => {
    if (status === "ready" && draft === null) {
      setDraft(stored);
      setSelectedId(stored[0]?.id ?? null);
    }
  }, [status, stored, draft]);

  const roles = draft ?? [];
  const selected = roles.find((r) => r.id === selectedId);
  const users = useMemo(
    () => Object.values(agents).filter((a) => selected && a.defaultRoleId === selected.id).map((a) => a.name),
    [agents, selected],
  );
  const normalized = draft ? normalizeRoles(draft) : null;

  const patch = (changes: Partial<DiscussionRole>) =>
    setDraft((list) => list && list.map((r) => (r.id === selectedId ? { ...r, ...changes } : r)));
  const insert = (role: DiscussionRole) => {
    setDraft((list) => {
      const at = list ? list.findIndex((r) => r.id === selectedId) : -1;
      const next = [...(list ?? [])];
      next.splice(at < 0 ? next.length : at + 1, 0, role);
      return next;
    });
    setSelectedId(role.id);
  };
  const addBlank = () => insert({ id: newRoleId(), name: t("roles.newName"), perspective: "", stance: "free", criteria: "" });
  const duplicate = () => selected && insert({ ...structuredClone(selected), id: newRoleId(), name: t("roles.copyName", { name: selected.name.trim() }) });
  const addPreset = (key: string) => {
    // 이미 있는 프리셋이면 새 id로 하나 더 만든다.
    const id = roles.some((r) => r.id === presetRoleId(key)) ? newRoleId() : presetRoleId(key);
    insert(presetRole(key, id));
  };
  const remove = () => {
    if (!selected) return;
    const index = roles.indexOf(selected);
    const next = roles.filter((r) => r.id !== selected.id);
    setDraft(next);
    setSelectedId(next[Math.min(index, next.length - 1)]?.id ?? null);
  };
  const save = async () => {
    if (!normalized) return;
    setSaving(true); setSaveError(false);
    try {
      await useRoleStore.getState().save(normalized.map((r) => (r.agent ? { ...r, agent: normalizeWorkAgent(r.agent) } : r)));
      closeModal();
    } catch {
      setSaveError(true);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="modal-backdrop" onMouseDown={(e) => { if (e.button === 0 && e.target === e.currentTarget) closeModal(); }}>
      <div className="pixel-panel profile-dialog roles-dialog" role="dialog" aria-labelledby="roles-dialog-title">
        <header className="profile-dialog-header">
          <h2 className="pixel-title" id="roles-dialog-title">{t("roles.title")}</h2>
          <p className="profile-dialog-sub">{t("roles.sub")}</p>
        </header>
        {status === "error" && (
          <div role="alert">
            {t("roles.loadError")}{" "}
            <button className="pixel-btn" onClick={() => void useRoleStore.getState().load()}>{t("roles.retry")}</button>
          </div>
        )}
        {(status === "loading" || status === "idle") && <p className="form-hint">{t("roles.loading")}</p>}
        {draft && (
          <div className="roles-body">
            <div className="roles-list-col">
              <ul className="roles-list" role="listbox" aria-label={t("roles.listLabel")}>
                {roles.map((r) => (
                  <li key={r.id} role="option" aria-selected={r.id === selectedId}>
                    <button type="button" className={`roles-list-item${r.id === selectedId ? " selected" : ""}`} onClick={() => setSelectedId(r.id)}>
                      <span className="roles-list-name">{r.name.trim() || t("roles.unnamed")}</span>
                      <small>{t(`stances.${r.stance}`)}</small>
                    </button>
                  </li>
                ))}
                {!roles.length && <li className="form-hint">{t("roles.empty")}</li>}
              </ul>
              <div className="roles-list-actions">
                <button type="button" className="pixel-btn" onClick={addBlank}>{t("roles.add")}</button>
                <button type="button" className="pixel-btn" disabled={!selected} onClick={duplicate}>{t("roles.duplicate")}</button>
                <button type="button" className="pixel-btn danger" disabled={!selected} onClick={remove}>{t("roles.delete")}</button>
                <select aria-label={t("roles.addPreset")} value="" onChange={(e) => { if (e.target.value) addPreset(e.target.value); }}>
                  <option value="">{t("roles.addPreset")}</option>
                  {ROLE_PRESETS.map((p) => <option key={p.key} value={p.key}>{t(`presets.${p.key}.name`)}</option>)}
                </select>
              </div>
            </div>
            {selected ? (
              <section className="form-section roles-form" key={selected.id}>
                <div className="form-field">
                  <label>
                    <span className="form-label-text">{t("roles.name")}</span>
                    <input value={selected.name} onChange={(e) => patch({ name: e.target.value })} />
                  </label>
                  {!selected.name.trim() && <p className="form-hint roles-warn">{t("roles.nameRequired")}</p>}
                </div>
                <div className="form-field">
                  <label>
                    <span className="form-label-text">{t("roles.stance")}</span>
                    <select value={selected.stance} onChange={(e) => patch({ stance: e.target.value as DiscussionRole["stance"] })}>
                      {DISCUSSION_STANCES.map((s) => <option key={s} value={s}>{t(`stances.${s}`)}</option>)}
                    </select>
                  </label>
                  <p className="form-hint">{t(`stanceHints.${selected.stance}`)}</p>
                </div>
                <div className="form-field">
                  <label>
                    <span className="form-label-text">{t("roles.perspective")}</span>
                    <textarea rows={3} value={selected.perspective} placeholder={t("roles.perspectivePlaceholder")} onChange={(e) => patch({ perspective: e.target.value })} />
                  </label>
                </div>
                <div className="form-field">
                  <label>
                    <span className="form-label-text">{t("roles.criteria")}</span>
                    <textarea rows={3} value={selected.criteria} placeholder={t("roles.criteriaPlaceholder")} onChange={(e) => patch({ criteria: e.target.value })} />
                  </label>
                </div>
                <div className="form-field">
                  <WorkAgentFields
                    value={selected.agent}
                    labels={{ field: t("roles.agent"), empty: t("roles.agentFromProfile") }}
                    onChange={(agent) => patch({ agent })}
                  />
                  <p className="form-hint">{t("roles.agentHint")}</p>
                </div>
                <p className="form-hint">
                  {users.length ? t("roles.usedBy", { names: users.join(", ") }) : t("roles.unused")}
                </p>
              </section>
            ) : (
              <p className="form-hint roles-form">{t("roles.noSelection")}</p>
            )}
          </div>
        )}
        <div className="dialog-actions">
          {saveError && <span className="roles-warn" role="alert">{t("roles.saveError")}</span>}
          <button className="pixel-btn primary" disabled={!normalized || saving} onClick={() => void save()}>{t("roles.save")}</button>
          <button className="pixel-btn" onClick={closeModal}>{t("roles.cancel")}</button>
        </div>
      </div>
    </div>
  );
}
