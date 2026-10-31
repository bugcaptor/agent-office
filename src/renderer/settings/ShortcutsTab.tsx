// src/renderer/settings/ShortcutsTab.tsx
//
// 설정 다이얼로그 "단축키" 탭 — 알림 에이전트로 가기 키와 에이전트별 터미널
// 열기 키. 칸을 누르고 키를 누르면 기록된다(녹화식). 빈 칸은 단축키 없음이고,
// 추천값은 흐린 글자로만 보이며 '추천값' 버튼을 눌러야 저장된다.
// 동작은 `shortcuts/agentShortcuts.ts`, 형식·검사는 `shortcuts/accelerator.ts`.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { IS_MAC } from "../shared/platform";
import {
  acceleratorFromEvent,
  acceleratorProblem,
  formatAccelerator,
  recommendedAgentShortcut,
  recommendedNotifyShortcut,
} from "../shortcuts/accelerator";

/** 알림 키 칸의 소유자 id. 에이전트 id와 겹치지 않는다. */
const NOTIFY_OWNER = "";

export function ShortcutsTab() {
  const { t } = useTranslation("settings");
  const agentOrder = useAppStore((s) => s.agentOrder);
  const agents = useAppStore((s) => s.agents);
  const agentShortcuts = useAppStore((s) => s.appSettings.agentShortcuts);
  const notifyJump = useAppStore((s) => s.appSettings.notifyJumpShortcut);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const [errors, setErrors] = useState<Record<string, string>>({});

  const ids = agentOrder.filter((id) => agents[id]);
  const valueOf = (owner: string) =>
    owner === NOTIFY_OWNER ? notifyJump : (agentShortcuts[owner] ?? "");
  const nameOf = (owner: string) =>
    owner === NOTIFY_OWNER ? t("shortcuts.notifyTitle") : (agents[owner]?.name ?? owner);

  function save(owner: string, acc: string): void {
    if (acc) {
      const problem = acceleratorProblem(acc, IS_MAC);
      if (problem) {
        setErrors((e) => ({ ...e, [owner]: t(`shortcuts.problem.${problem}`) }));
        return;
      }
      const taken = [NOTIFY_OWNER, ...ids].find((o) => o !== owner && valueOf(o) === acc);
      if (taken !== undefined) {
        setErrors((e) => ({ ...e, [owner]: t("shortcuts.problem.taken", { name: nameOf(taken) }) }));
        return;
      }
    }
    setErrors((e) => ({ ...e, [owner]: "" }));
    if (owner === NOTIFY_OWNER) {
      updateAppSettings({ notifyJumpShortcut: acc });
      return;
    }
    // 지워진 에이전트의 키는 저장할 때 함께 정리한다.
    const next: Record<string, string> = {};
    for (const id of ids) {
      const v = id === owner ? acc : agentShortcuts[id];
      if (v) next[id] = v;
    }
    updateAppSettings({ agentShortcuts: next });
  }

  return (
    <div className="settings-form">
      <p className="settings-note">{t("shortcuts.help")}</p>
      <ShortcutRow
        title={t("shortcuts.notifyTitle")}
        help={t("shortcuts.notifyHelp")}
        value={notifyJump}
        recommended={recommendedNotifyShortcut(IS_MAC)}
        error={errors[NOTIFY_OWNER]}
        onSave={(acc) => save(NOTIFY_OWNER, acc)}
      />
      <strong>{t("shortcuts.agentsTitle")}</strong>
      {ids.length === 0 && <p className="settings-note">{t("shortcuts.noAgents")}</p>}
      {ids.map((id, i) => (
        <ShortcutRow
          key={id}
          title={agents[id].name}
          help={agents[id].clockedOut ? t("shortcuts.clockedOut") : undefined}
          value={agentShortcuts[id] ?? ""}
          recommended={recommendedAgentShortcut(i, IS_MAC)}
          error={errors[id]}
          onSave={(acc) => save(id, acc)}
        />
      ))}
    </div>
  );
}

function ShortcutRow(props: {
  title: string;
  help?: string;
  value: string;
  recommended: string | null;
  error?: string;
  onSave(acc: string): void;
}) {
  const { t } = useTranslation("settings");
  // 설정 창은 모달이라 그동안 단축키 리스너(agentShortcuts)는 동작하지 않는다 —
  // 녹화 중에 이미 지정된 키를 눌러도 터미널이 열리지 않는다.
  const [recording, setRecording] = useState(false);

  const recommendedLabel = props.recommended ? formatAccelerator(props.recommended, IS_MAC) : "";
  const showRecommend = props.recommended !== null && props.value !== props.recommended;

  return (
    <div className="settings-item settings-shortcut-row">
      <span>
        <strong>{props.title}</strong>
        {props.help && <small>{props.help}</small>}
        {props.error && <small className="settings-shortcut-error" role="alert">{props.error}</small>}
      </span>
      <span className="settings-shortcut-controls">
        <button
          type="button"
          className={
            "pixel-btn settings-shortcut-key" +
            (recording ? " recording" : "") +
            (props.value ? "" : " placeholder")
          }
          aria-label={t("shortcuts.recordAria", { name: props.title })}
          onClick={(e) => {
            // WKWebView는 버튼 클릭에 포커스를 주지 않아 keydown이 안 온다.
            e.currentTarget.focus();
            setRecording(true);
          }}
          onBlur={() => setRecording(false)}
          onKeyDown={(e) => {
            if (!recording) return;
            e.preventDefault();
            e.stopPropagation();
            if (e.key === "Escape" && !e.ctrlKey && !e.altKey && !e.metaKey && !e.shiftKey) {
              setRecording(false);
              return;
            }
            const acc = acceleratorFromEvent(e);
            if (!acc) return; // 수식키만 눌림 — 나머지 키를 기다린다.
            setRecording(false);
            props.onSave(acc);
          }}
        >
          {recording
            ? t("shortcuts.pressKey")
            : props.value
              ? formatAccelerator(props.value, IS_MAC)
              : recommendedLabel || t("shortcuts.none")}
        </button>
        {showRecommend && (
          <button
            type="button"
            className="pixel-btn"
            title={t("shortcuts.useRecommendedTitle", { key: recommendedLabel })}
            onClick={() => props.onSave(props.recommended!)}
          >
            {t("shortcuts.useRecommended")}
          </button>
        )}
        {props.value && (
          <button
            type="button"
            className="pixel-btn"
            aria-label={t("shortcuts.clearAria", { name: props.title })}
            onClick={() => props.onSave("")}
          >
            ×
          </button>
        )}
      </span>
    </div>
  );
}
