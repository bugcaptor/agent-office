// src/renderer/settings/SettingsForm.tsx
//
// 선택적 에이전트 연동 설정 — FirstRunDialog(첫 실행 동의)와
// SettingsDialog(상시 변경)가 공유한다. 폼은 상태를 소유하지 않는다:
// value/onChange 순수 제어 컴포넌트.
import type { AppSettings } from "@shared/types";

export type SettingsFormValue = Pick<
  AppSettings,
  "summarizerEnabled" | "summaryProvider" | "summarizerToolCalls" | "observerEnabled"
>;

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
          checked={value.summarizerEnabled}
          onChange={(e) => onChange({ summarizerEnabled: e.target.checked })}
        />
        <span>
          <strong>작업 라벨 요약</strong>
          <small>
            머리 위 작업 라벨을 선택한 CLI로 요약합니다. 선택한 CLI는 사용자의
            해당 Claude 또는 Codex 계정 사용량을 소모합니다.
          </small>
        </span>
      </label>

      <fieldset aria-label="요약기 선택">
        <legend>요약기 선택</legend>
        <label>
          <input
            type="radio"
            name="summary-provider"
            checked={value.summaryProvider === "claude"}
            onChange={() => onChange({ summaryProvider: "claude" })}
          />
          Claude
        </label>
        <label>
          <input
            type="radio"
            name="summary-provider"
            checked={value.summaryProvider === "codex"}
            onChange={() => onChange({ summaryProvider: "codex" })}
          />
          Codex
        </label>
      </fieldset>

      <label className="settings-item settings-subitem">
        <input
          type="checkbox"
          checked={value.summarizerToolCalls}
          disabled={!value.summarizerEnabled || value.summaryProvider !== "claude"}
          onChange={(e) => onChange({ summarizerToolCalls: e.target.checked })}
        />
        <span>
          <strong>실험: 작업 폴더 훑어보기 (Claude 전용)</strong>
          <small>
            요약기가 읽기 전용 툴(Read/Glob/Grep)로 세션 작업 폴더를 살펴 목표를
            더 정확히 추론합니다. 호출당 사용량을 더 소모하고 느려집니다.
          </small>
        </span>
      </label>

      <label className="settings-item">
        <input
          type="checkbox"
          checked={value.observerEnabled}
          onChange={(e) => onChange({ observerEnabled: e.target.checked })}
        />
        <span>
          <strong>에이전트 관찰 (알림·시간측정)</strong>
          <small>
            Claude, Codex, Pi의 알림과 시간측정은 새로 만든 터미널부터 적용됩니다.
            꺼져 있으면 느낌표 알림과 세션 시간측정이 동작하지 않습니다.
          </small>
        </span>
      </label>
    </div>
  );
}
