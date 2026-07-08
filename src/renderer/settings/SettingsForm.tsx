// src/renderer/settings/SettingsForm.tsx
//
// Claude Code 연동 opt-in 토글 2개 — FirstRunDialog(첫 실행 동의)와
// SettingsDialog(상시 변경)가 공유한다. 폼은 상태를 소유하지 않는다:
// value/onChange 순수 제어 컴포넌트.
export interface SettingsFormValue {
  claudeCliEnabled: boolean;
  claudeHooksEnabled: boolean;
}

export function SettingsForm({
  value,
  onChange,
}: {
  value: SettingsFormValue;
  onChange: (patch: Partial<SettingsFormValue>) => void;
}) {
  return (
    <div className="settings-form">
      <label className="settings-item">
        <input
          type="checkbox"
          checked={value.claudeCliEnabled}
          onChange={(e) => onChange({ claudeCliEnabled: e.target.checked })}
        />
        <span>
          <strong>작업 라벨 요약 (claude CLI)</strong>
          <small>
            머리 위 작업 라벨 요약에 로컬 claude CLI를 호출합니다. 호출마다
            Claude 구독 크레딧을 소모합니다.
          </small>
        </span>
      </label>
      <label className="settings-item">
        <input
          type="checkbox"
          checked={value.claudeHooksEnabled}
          onChange={(e) => onChange({ claudeHooksEnabled: e.target.checked })}
        />
        <span>
          <strong>알림·시간측정 (Claude Code 훅)</strong>
          <small>
            세션 안의 claude에 훅 설정을 주입하고 127.0.0.1 로컬 서버로 알림을
            받습니다. 꺼져 있으면 느낌표 알림과 세션 시간측정이 동작하지
            않습니다. 변경은 새 세션부터 적용됩니다.
          </small>
        </span>
      </label>
    </div>
  );
}
