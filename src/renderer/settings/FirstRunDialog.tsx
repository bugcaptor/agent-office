// src/renderer/settings/FirstRunDialog.tsx
//
// 첫 실행(설정 파일 부재) 동의 다이얼로그. 에이전트 연동 기능은 opt-in —
// 기본 OFF로 제시하고 유저가 명시적으로 켠 것만 저장한다. 닫기 회피 불가:
// 백드롭 클릭 핸들러를 달지 않는다(선택을 저장해야만 진행).
import { useState } from "react";
import { useAppStore } from "../store/appStore";
import { SettingsForm, type SettingsFormValue } from "./SettingsForm";

export function FirstRunDialog() {
  const firstRun = useAppStore((s) => s.settingsFirstRun);
  const completeFirstRun = useAppStore((s) => s.completeFirstRun);
  const [value, setValue] = useState<SettingsFormValue>({
    summarizerEnabled: false,
    summaryProvider: "claude",
    diaryEnabled: false,
    observerEnabled: false,
  });

  if (!firstRun) return null;

  return (
    <div className="modal-backdrop">
      <div className="pixel-panel first-run-dialog">
        <h2 className="pixel-title">Agent Office 시작하기</h2>
        <p>
          Claude Code / Codex / Pi 연동 기능은 선택 사항입니다. 지금 끄고 시작해도
          언제든 하단 바의 ⚙ 설정에서 켤 수 있습니다.
        </p>
        <SettingsForm value={value} onChange={(p) => setValue((v) => ({ ...v, ...p }))} />
        <div className="dialog-actions">
          <button className="pixel-btn primary" onClick={() => completeFirstRun(value)}>
            시작하기
          </button>
        </div>
      </div>
    </div>
  );
}
