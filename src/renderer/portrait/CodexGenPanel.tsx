// src/renderer/portrait/CodexGenPanel.tsx
//
// 프로필 편집 "외형" 섹션의 [Codex로 생성] 탭 패널(kbm #2fa).
//
// 표시 전용이다 — 생성 오케스트레이션(세션 토큰 가드, 결과를 크롭 에디터로
// 넘기기)은 ProfileDialog가 그대로 들고 있고, 여기서는 codex 설치 탐지와
// 버튼·캡션 렌더링만 한다. ProfileDialog는 상시 마운트(닫힘 = return null)라
// 늦은 응답 무효화를 unmount가 아니라 편집 세션 토큰으로 해야 하기 때문이다.
import { useEffect, useState } from "react";
import { tauriApi } from "../ipc/tauriApi";
import type { CodexImageStatus } from "@shared/types";

/** IPC 오류 문자열("{code}: {상세}") → 사용자 캡션. */
export function codexGenErrorCaption(err: unknown): string {
  const raw = String(err);
  const code = raw.split(":")[0]?.trim() ?? "";
  // 미설치는 요약기와 같은 `-not-found` 관례를 따른다(포함 검사).
  if (code.includes("-not-found")) {
    return "codex CLI를 찾을 수 없습니다. 설치한 뒤 다시 시도하세요.";
  }
  switch (code) {
    case "timeout":
      return "생성이 시간 안에 끝나지 않았습니다. 다시 시도하세요.";
    case "no_output":
      return `codex가 이미지를 저장하지 않았습니다. 다시 시도하세요: ${raw.slice(raw.indexOf(":") + 1).trim()}`;
    default:
      return `생성에 실패했습니다: ${raw}`;
  }
}

export type CodexGenKind = "portrait" | "sprite";

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
          ? "codex CLI 확인 중…"
          : available
            ? `codex CLI 사용 가능${status?.version ? ` — ${status.version}` : ""}`
            : "codex CLI를 찾을 수 없습니다. 설치하고 로그인한 뒤 다시 열어 주세요."}
      </p>
      {!enabled && (
        <p className="form-hint">저장한 뒤 편집에서 생성할 수 있습니다.</p>
      )}
      <div className="sprite-buttons">
        <button
          className="pixel-btn"
          disabled={!canGenerate}
          onClick={() => onGenerate("portrait")}
        >
          {busy === "portrait" ? "초상 생성 중…" : "초상 생성"}
        </button>
        <button
          className="pixel-btn"
          disabled={!canGenerate}
          onClick={() => onGenerate("sprite")}
        >
          {busy === "sprite" ? "스프라이트 생성 중…" : "스프라이트 생성"}
        </button>
      </div>
      {note && <span className="sprite-custom-badge">{note}</span>}
      <p className="form-hint">
        로컬 codex CLI의 이미지 생성 기능을 그대로 씁니다. 한 장에 보통 1~3분이
        걸리고, 여러분의 Codex 사용량이 차감됩니다. 결과는 크롭 편집기로 열려
        확인한 뒤 저장합니다.
      </p>
    </div>
  );
}
