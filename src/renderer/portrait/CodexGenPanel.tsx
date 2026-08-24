// src/renderer/portrait/CodexGenPanel.tsx
//
// 프로필 편집 "외형" 섹션의 [Codex로 생성] 탭 패널(kbm #2fa).
//
// 표시 전용이다 — 생성 오케스트레이션(세션 토큰 가드, 결과를 크롭 에디터로
// 넘기기)은 ProfileDialog가 그대로 들고 있고, 여기서는 codex 설치 탐지와
// 버튼·캡션 렌더링만 한다. ProfileDialog는 상시 마운트(닫힘 = return null)라
// 늦은 응답 무효화를 unmount가 아니라 편집 세션 토큰으로 해야 하기 때문이다.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
// 컴포넌트 밖(순수 함수 `codexGenErrorCaption`)에서 쓰는 번역 함수.
// 컴포넌트 안에서는 훅(useTranslation)을 써야 언어 변경에 리렌더가 걸린다.
import { t as translate } from "@renderer/i18n";
import { tauriApi } from "../ipc/tauriApi";
import { backendErrorText, parseBackendError } from "../shared/backendError";
import type { CodexImageStatus } from "@shared/types";

/** 이 패널 전용 코드 → 번역 키. 공통 매핑(`BACKEND_ERROR_KEY`)에 두지 않는
 *  이유는 안내가 "codex CLI를 설치하라"처럼 이 기능에 매여 있기 때문이다. */
type CodexGenErrorCode = "timeout" | "no_output";

const CODEX_GEN_ERROR_KEY: Record<CodexGenErrorCode, string> = {
  timeout: "profile:codex.errTimeout",
  no_output: "profile:codex.errNoOutput",
};

/** IPC 오류 문자열("{code}: {상세}") → 사용자 캡션. */
export function codexGenErrorCaption(err: unknown): string {
  const { code, detail } = parseBackendError(err);
  // 미설치는 요약기와 같은 `{provider}-not-found` 관례를 따른다(포함 검사).
  if (code.includes("-not-found")) {
    return translate("profile:codex.errNotFound");
  }
  // no_output만 상세를 문장 안에 끼워 넣는 전용 문구가 있다.
  if (code === "no_output") {
    return translate(CODEX_GEN_ERROR_KEY.no_output, { detail });
  }
  const key = CODEX_GEN_ERROR_KEY[code as CodexGenErrorCode];
  if (key) return translate(key);
  return translate("profile:codex.errGeneric", { error: backendErrorText(err) });
}

export type CodexGenKind = "portrait" | "sprite" | "minimi";

export function CodexGenPanel({
  enabled,
  busy,
  note,
  onGenerate,
}: {
  /** false면(=신규 생성 모드) 저장 전이라 대상 캐릭터가 없다. */
  enabled: boolean;
  /** 진행 중인 생성의 종류. null이면 유휴. */
  busy: CodexGenKind | null;
  /** 진행/결과/오류 캡션. */
  note: string | null;
  onGenerate: (kind: CodexGenKind) => void;
}) {
  const { t } = useTranslation("profile");
  const [status, setStatus] = useState<CodexImageStatus | null>(null);

  // 탐지는 패널이 열릴 때 1회. 캐시하지 않는다 — 설치 직후 탭을 다시 열면
  // 바로 반영되는 편이 낫다.
  useEffect(() => {
    let alive = true;
    void (async () => {
      try {
        const s = await tauriApi.codexImageStatus();
        if (alive) setStatus(s);
      } catch {
        if (alive) setStatus({ available: false });
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  const detecting = status === null;
  const available = status?.available === true;
  const canGenerate = enabled && available && busy === null;

  return (
    <div className="codexgen-panel">
      <p className="codexgen-status">
        {detecting
          ? t("codex.detecting")
          : available
            ? status?.version
              ? t("codex.availableVersion", { version: status.version })
              : t("codex.available")
            : t("codex.unavailable")}
      </p>
      {!enabled && (
        <p className="form-hint">{t("codex.saveFirstHint")}</p>
      )}
      <div className="sprite-buttons">
        <button
          className="pixel-btn"
          disabled={!canGenerate}
          onClick={() => onGenerate("portrait")}
        >
          {busy === "portrait" ? t("codex.portraitBusy") : t("codex.portrait")}
        </button>
        <button
          className="pixel-btn"
          disabled={!canGenerate}
          onClick={() => onGenerate("sprite")}
        >
          {busy === "sprite" ? t("codex.spriteBusy") : t("codex.sprite")}
        </button>
        <button
          className="pixel-btn"
          disabled={!canGenerate}
          onClick={() => onGenerate("minimi")}
        >
          {busy === "minimi" ? t("codex.minimiBusy") : t("codex.minimi")}
        </button>
      </div>
      {note && <span className="sprite-custom-badge">{note}</span>}
      <p className="form-hint">{t("codex.hint")}</p>
    </div>
  );
}
