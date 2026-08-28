// src/renderer/profile/sections/VoiceField.tsx
//
// 대사 TTS 보이스 선택 + 그 자리 미리듣기.
//
// 기본은 "자동" — 캐릭터 종족(archetype)에 어울리는 성별·연령 라벨로 후보를
// 좁힌 뒤 시드 해시로 고정 배정한다(백엔드 `tts::voice`). 여기서 고르는 것은
// 그 자동 배정을 덮어쓰는 수동 지정이다.
//
// 목록은 백엔드가 ElevenLabs에서 1회 조회해 캐시한 것과 **같은 것**이라, 여기
// 보이는 이름이 실제 발화 목소리와 어긋나지 않는다. 키 값은 오지 않는다.
// TTS가 꺼져 있거나 키가 없으면 고를 것이 없으므로 비활성 + 사유 안내.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { useAppStore } from "../../store/appStore";
import { tauriApi } from "../../ipc/tauriApi";
import { backendErrorText, parseBackendError } from "../../shared/backendError";
import { previewVoice } from "../../sound/soundManager";
import { archetypeOrAuto, resolveArchetype } from "../../office/gen/archetypes";
import type { DraftProfile } from "../generate";
import type { TtsVoiceOption } from "@shared/types";

export function VoiceField({
  draft,
  agentId,
  onChange,
}: {
  draft: DraftProfile;
  agentId?: string;
  onChange: (voiceId: string) => void;
}) {
  const { t } = useTranslation("profile");
  const ttsEnabled = useAppStore((s) => s.appSettings.ttsEnabled);
  const muted = useAppStore((s) => s.muted);
  const [voices, setVoices] = useState<TtsVoiceOption[]>([]);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!ttsEnabled) {
      setVoices([]);
      setNote(null);
      return;
    }
    let alive = true;
    void (async () => {
      try {
        const list = await tauriApi.ttsListVoices();
        if (!alive) return;
        setVoices(list);
        setNote(null);
      } catch (err) {
        if (!alive) return;
        setVoices([]);
        // 키가 없으면 목록도 못 받는다 — 설정으로 안내한다.
        // 키 없음만 전용 안내(설정으로 유도)이고 나머지는 공통 매핑에 맡긴다.
        setNote(
          parseBackendError(err).code === "missing_elevenlabs_key"
            ? t("voice.keyMissing")
            : t("voice.listFailed", { error: backendErrorText(err) })
        );
      }
    })();
    return () => {
      alive = false;
    };
  }, [ttsEnabled]);

  const selected = draft.voiceId ?? "";
  // 다른 PC에서 가져온 프로필 등, 목록에 없는 id도 선택값으로 살려 둔다 —
  // select가 조용히 "자동"으로 되돌아가면 저장 시 지정이 날아간다.
  const missing = selected !== "" && !voices.some((v) => v.voiceId === selected);
  const disabled = !ttsEnabled || voices.length === 0;

  const preview = async () => {
    setBusy(true);
    try {
      const line = await previewVoice({
        agentId: agentId ?? "preview",
        agentName: draft.name,
        // 종족은 보이스 캐스팅용, 대사 말투는 편집 중인 성격 프롬프트가 정한다.
        archetype: resolveArchetype(archetypeOrAuto(draft.archetype), draft.seed),
        ...(draft.personalityPrompt?.trim() ? { personality: draft.personalityPrompt.trim() } : {}),
        seed: draft.seed,
        ...(selected ? { voiceId: selected } : {}),
      });
      setNote(line ? t("voice.spoken", { line }) : t("voice.spokenNone"));
    } catch (err) {
      setNote(t("voice.previewFailed", { error: backendErrorText(err) }));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="form-field">
      <label>
        <span className="form-label-text">{t("voice.label")}</span>
        <div className="form-control-row">
          <select
            value={selected}
            disabled={disabled}
            onChange={(e) => onChange(e.target.value)}
          >
            <option value="">{t("voice.auto")}</option>
            {missing && (
              <option value={selected}>{t("voice.missingOption", { id: selected })}</option>
            )}
            {voices.map((v) => (
              <option key={v.voiceId} value={v.voiceId}>
                {v.labels ? `${v.name} — ${v.labels}` : v.name}
              </option>
            ))}
          </select>
          <button
            type="button"
            className="pixel-btn"
            disabled={!ttsEnabled || busy}
            onClick={preview}
          >
            {t("voice.preview")}
          </button>
        </div>
      </label>
      <p className="form-hint">
        {!ttsEnabled ? t("voice.hintDisabled") : t("voice.hintEnabled")}
        {muted && t("voice.mutedNote")}
      </p>
      {note && <p className="form-hint">{note}</p>}
    </div>
  );
}
