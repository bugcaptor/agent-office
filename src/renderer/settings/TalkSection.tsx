// src/renderer/settings/TalkSection.tsx
//
// 동료 대화(docs/agent-talk-design.md) 설정 UI. 캐릭터가 **다른 캐릭터의
// 세션에 글자를 밀어 넣는** 기능이라 CLI 제어·웹 원격과 같은 결의 옵트인으로
// 둔다 — 기본 꺼짐이고, 끄면 아직 배달되지 않은 메시지까지 버리는 킬 스위치다.
//
// 이 스위치는 "허용"일 뿐 대화를 시작하지 않는다. 대화는 사용자가 세션에서
// `/agent-office:talk` 스킬을 명시적으로 발동해야 열린다(스킬 설치는 앱이
// 알아서 하므로 여기에 설치 버튼이 없다).

import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";

import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import type { ControlStatus } from "../../shared/types/settings";

/** 한 대화의 왕복 상한 입력 범위. 백엔드도 같은 상한으로 클램프한다. */
const MAX_TURNS_MIN = 1;
const MAX_TURNS_MAX = 50;
/** 주입 전 유휴 대기 입력 범위(ms). 0 = 조용해질 때까지 기다리지 않음. */
const IDLE_QUIET_MIN = 0;
const IDLE_QUIET_MAX = 60000;

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

export function TalkSection() {
  const { t } = useTranslation("settings");
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const enabled = appSettings.talkEnabled;
  // 대화는 에이전트가 `ctl`로 앱에 말을 거는 구조라 CLI 제어가 켜지고 승인까지
  // 돼 있어야 실제로 동작한다 — 안 돼 있으면 스킬만 뜨고 전부 실패하므로
  // 여기서 미리 알려 준다(자동으로 켜 주지는 않는다 — 권한 상승이므로).
  const [control, setControl] = useState<ControlStatus | null>(null);
  const refresh = useCallback(async () => {
    try {
      setControl(await tauriApi.controlStatus());
    } catch {
      setControl(null);
    }
  }, []);
  useEffect(() => {
    void refresh();
  }, [refresh, enabled, appSettings.cliEnabled]);
  const ready = appSettings.cliEnabled && control?.approved === true;

  return (
    <div className="settings-form">
      <label className="settings-item">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => updateAppSettings({ talkEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("talk.title")}</strong>
          <small>
            <Trans t={t} i18nKey="talk.help" components={{ b: <b />, code: <code /> }} />
          </small>
        </span>
      </label>

      <label className="settings-item">
        <span>
          <strong>{t("talk.maxTurnsTitle")}</strong>
          <small>{t("talk.maxTurnsHelp")}</small>
        </span>
        <input
          type="number"
          min={MAX_TURNS_MIN}
          max={MAX_TURNS_MAX}
          disabled={!enabled}
          value={appSettings.talkMaxTurns}
          onChange={(e) =>
            updateAppSettings({
              talkMaxTurns: clamp(
                Math.round(Number(e.target.value) || 0),
                MAX_TURNS_MIN,
                MAX_TURNS_MAX,
              ),
            })
          }
        />
      </label>

      <label className="settings-item">
        <span>
          <strong>{t("talk.idleQuietTitle")}</strong>
          <small>{t("talk.idleQuietHelp")}</small>
        </span>
        <input
          type="number"
          min={IDLE_QUIET_MIN}
          max={IDLE_QUIET_MAX}
          step={500}
          disabled={!enabled}
          value={appSettings.talkIdleQuietMs}
          onChange={(e) =>
            updateAppSettings({
              talkIdleQuietMs: clamp(
                Math.round(Number(e.target.value) || 0),
                IDLE_QUIET_MIN,
                IDLE_QUIET_MAX,
              ),
            })
          }
        />
      </label>

      {enabled && !ready && (
        <p className="settings-note" role="alert">
          <Trans
            t={t}
            i18nKey="talk.controlWarn"
            values={{
              reason: appSettings.cliEnabled
                ? t("talk.reasonUnapproved")
                : t("talk.reasonDisabled"),
            }}
            components={{ b: <b /> }}
          />
        </p>
      )}

      <p className="settings-note">
        <Trans t={t} i18nKey="talk.skillNote" components={{ code: <code /> }} />
      </p>
    </div>
  );
}
