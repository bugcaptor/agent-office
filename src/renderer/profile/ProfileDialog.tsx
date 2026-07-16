// src/renderer/profile/ProfileDialog.tsx
//
// Profile creation/editing dialog. Renders a
// random draft (or the existing profile's values, in edit mode), a live
// sprite preview driven directly by B's pure `generateSpritePreview` (no
// scene call), and on save: normalize the draft -> `addAgent`
// (store, seeds session as `starting`) -> `tauriApi.createSession` (PTY
// start) -> close. Editing updates the existing profile in place and never
// starts a new session.
import { useEffect, useRef, useState } from "react";
import { nanoid } from "nanoid";
import { useAppStore } from "../store/appStore";
import { generateDraft, draftToProfile, type DraftProfile } from "./generate";
import { generateSpritePreview } from "../office/gen/characterFactory";
import { ARCHETYPE_SELECT_OPTIONS, resolveArchetype, pickArchetype } from "../office/gen/archetypes";
import { tauriApi } from "../ipc/tauriApi";
import { sessionOptsFor } from "../ipc/sessionOpts";
import { buildPortraitPrompt, buildSpritePrompt, buildPixelLabSpriteDescription } from "../portrait/promptBuilder";
import { PortraitEditor } from "../portrait/PortraitEditor";
import { SpriteEditor } from "../sprite/SpriteEditor";
import { clearSpriteOverride } from "../office/gen/spriteOverrides";
import { KEYBOARD_SOUND_PACK_OPTIONS } from "../sound/packs";
import { previewKeyboardSound } from "../sound/soundManager";
import type { AvailableShell } from "@shared/types";
import "../portrait/portrait.css";

/** IPC 오류 문자열("{code}: {상세}") → 사용자 캡션. */
export function pixellabErrorCaption(err: unknown): string {
  const raw = String(err);
  const code = raw.split(":")[0]?.trim();
  switch (code) {
    case "missing_api_key":
      return "PIXELLAB_API_KEY 환경변수를 설정한 뒤 앱을 재시작하세요.";
    case "invalid_api_key":
      return "PixelLab API 키가 유효하지 않습니다.";
    case "insufficient_credits":
      return "PixelLab 크레딧이 부족합니다.";
    case "rate_limited":
      return "요청이 몰려 있습니다. 잠시 후 다시 시도하세요.";
    default:
      return `생성에 실패했습니다: ${raw}`;
  }
}

export function ProfileDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const addAgent = useAppStore((s) => s.addAgent);
  const updateAgent = useAppStore((s) => s.updateAgent);
  const setSessionState = useAppStore((s) => s.setSessionState);
  const agentOrder = useAppStore((s) => s.agentOrder);

  const editing = modal.kind === "profile-edit";
  const editingAgentId = modal.kind === "profile-edit" ? modal.agentId : undefined;
  const editingAgent = useAppStore((s) =>
    editingAgentId ? s.agents[editingAgentId] : undefined
  );
  const removePortrait = useAppStore((s) => s.removePortrait);
  const portraitUrl = useAppStore((s) =>
    editingAgent ? s.portraits[editingAgent.id] : undefined
  );
  const [editorOpen, setEditorOpen] = useState(false);
  const removeSpritePreview = useAppStore((s) => s.removeSpritePreview);
  const spritePreviewUrl = useAppStore((s) =>
    editingAgent ? s.spritePreviews[editingAgent.id] : undefined
  );
  const [spriteEditorOpen, setSpriteEditorOpen] = useState(false);
  const [pixellabBusy, setPixellabBusy] = useState(false);
  const [pixellabNote, setPixellabNote] = useState<string | null>(null);
  /** PixelLab 생성 결과 data URL — SpriteEditor initialImage로 전달. */
  const [generatedImage, setGeneratedImage] = useState<string | null>(null);
  /** 진행 중 생성 요청의 세션 토큰 — 편집 대상이 바뀌거나 다이얼로그가
   * 닫히면 무효화된다 (상시 마운트 컴포넌트라 unmount 가드는 무의미). */
  const pixellabSeqRef = useRef(0);

  const [draft, setDraft] = useState<DraftProfile>(() => generateDraft());
  const [spriteUrl, setSpriteUrl] = useState<string>("");
  const [shells, setShells] = useState<AvailableShell[]>([]);

  // 마운트 시 사용 가능한 셸 목록 조회 (Windows 외에는 빈 배열 → 셀렉터 미노출).
  useEffect(() => {
    tauriApi.listAvailableShells().then(setShells).catch(() => setShells([]));
  }, []);

  // 편집 모드 진입 시 기존 값 로드.
  //
  // Depend on the agent's IDENTITY (id), not the `editingAgent` object
  // itself: PortraitEditor's onSave and the 제거 button both call
  // `updateAgent` while this dialog stays open (setting/clearing
  // `portraitUpdatedAt`), which produces a new `editingAgent` object on
  // every such update. Depending on the object would re-fire this effect
  // and silently revert any typed-but-unsaved name/role/note/appearance
  // edits back to the store's values. Reading the agent via `getState()`
  // (rather than closing over the reactive `editingAgent`) keeps this
  // effect's deps honest for exhaustive-deps without an eslint-disable.
  useEffect(() => {
    // 편집 세션 경계: 진행 중 생성 응답 무효화 + 이전 세션의 캡션/이미지/busy 정리.
    pixellabSeqRef.current++;
    setPixellabBusy(false);
    setPixellabNote(null);
    setGeneratedImage(null);
    if (!editingAgentId) return;
    const agent = useAppStore.getState().agents[editingAgentId];
    if (!agent) return;
    setDraft({
      name: agent.name,
      role: agent.role,
      note: agent.note,
      seed: agent.seed,
      cwd: agent.cwd ?? "",
      shell: agent.shell ?? "",
      startupCommand: agent.startupCommand ?? "",
      personalityPrompt: agent.personalityPrompt ?? "",
      appearance: agent.appearance ?? "",
      spriteRequest: agent.spriteRequest ?? "",
      archetype: agent.archetype ?? "auto",
      keyboardSound: agent.keyboardSound ?? "",
    });
  }, [editingAgentId]);

  // seed 또는 archetype 변경 시 라이브 스프라이트 프리뷰 (B의 순수 함수 — 동기, 아키타입 반영)
  useEffect(() => {
    const eff = resolveArchetype(draft.archetype, draft.seed);
    setSpriteUrl(generateSpritePreview(draft.seed, 6, undefined, undefined, eff));
  }, [draft.seed, draft.archetype]);

  const regenSeed = () => setDraft((d) => ({ ...d, seed: nanoid(8) }));
  const regenAll = () => setDraft(generateDraft());

  const onCopyPrompt = async () => {
    const prompt = buildPortraitPrompt({
      name: draft.name,
      role: draft.role,
      note: draft.note,
      appearance: draft.appearance,
      seed: draft.seed,
      archetype: draft.archetype,
    });
    try {
      await navigator.clipboard.writeText(prompt);
    } catch (err) {
      console.warn("ProfileDialog: clipboard write failed", err);
    }
  };

  const onRemovePortrait = async () => {
    if (!editingAgent) return;
    try {
      await tauriApi.deletePortrait(editingAgent.id);
    } catch (err) {
      console.warn("ProfileDialog: deletePortrait failed", err);
    }
    removePortrait(editingAgent.id);
    updateAgent(editingAgent.id, { portraitUpdatedAt: undefined });
  };

  const onCopySpritePrompt = async () => {
    const prompt = buildSpritePrompt({
      name: draft.name,
      role: draft.role,
      spriteRequest: draft.spriteRequest,
      appearance: draft.appearance,
      seed: draft.seed,
      archetype: draft.archetype,
    });
    try {
      await navigator.clipboard.writeText(prompt);
    } catch (err) {
      console.warn("ProfileDialog: clipboard write failed", err);
    }
  };

  const onGeneratePixellab = async () => {
    if (pixellabBusy || !editingAgent) return;
    const seq = ++pixellabSeqRef.current;
    const targetAgentId = editingAgent.id;
    /** 응답 적용 가능 여부: 토큰 유효 + 같은 에이전트의 편집 모달이 여전히 열려 있음. */
    const stillCurrent = () => {
      const m = useAppStore.getState().modal;
      return (
        pixellabSeqRef.current === seq &&
        m.kind === "profile-edit" &&
        m.agentId === targetAgentId
      );
    };
    setPixellabBusy(true);
    setPixellabNote(null);
    const description = buildPixelLabSpriteDescription({
      name: draft.name,
      role: draft.role,
      spriteRequest: draft.spriteRequest,
      appearance: draft.appearance,
      seed: draft.seed,
      archetype: draft.archetype,
    });
    try {
      const res = await tauriApi.generateSpriteImage(description);
      if (!stillCurrent()) return;
      setGeneratedImage(`data:image/png;base64,${res.pngBase64}`);
      setSpriteEditorOpen(true);
      setPixellabNote(
        res.costUsd != null ? `생성 완료 · $${res.costUsd.toFixed(2)}` : "생성 완료",
      );
    } catch (err) {
      if (!stillCurrent()) return;
      setPixellabNote(pixellabErrorCaption(err));
    } finally {
      if (pixellabSeqRef.current === seq) setPixellabBusy(false);
    }
  };

  const onRemoveSprite = async () => {
    if (!editingAgent) return;
    try {
      await tauriApi.deleteSprite(editingAgent.id);
    } catch (err) {
      console.warn("ProfileDialog: deleteSprite failed", err);
    }
    clearSpriteOverride(editingAgent.id);
    removeSpritePreview(editingAgent.id);
    updateAgent(editingAgent.id, { spriteUpdatedAt: undefined });
  };

  const onSave = async () => {
    if (editing && editingAgent) {
      const trimmedCwd = (draft.cwd ?? "").trim();
      const trimmedShell = (draft.shell ?? "").trim();
      const trimmedStartupCommand = (draft.startupCommand ?? "").trim();
      const trimmedPersonalityPrompt = (draft.personalityPrompt ?? "").trim();
      const trimmedAppearance = (draft.appearance ?? "").trim();
      const trimmedSpriteRequest = (draft.spriteRequest ?? "").trim();
      const trimmedKeyboardSound = (draft.keyboardSound ?? "").trim();
      const chosenArchetype =
        draft.archetype && draft.archetype !== "auto" ? draft.archetype : pickArchetype(draft.seed);
      updateAgent(editingAgent.id, {
        name: draft.name,
        role: draft.role,
        note: draft.note,
        seed: draft.seed,
        archetype: chosenArchetype,
        cwd: trimmedCwd || undefined,
        shell: trimmedShell || undefined,
        startupCommand: trimmedStartupCommand || undefined,
        personalityPrompt: trimmedPersonalityPrompt || undefined,
        appearance: trimmedAppearance || undefined,
        spriteRequest: trimmedSpriteRequest || undefined,
        keyboardSound: trimmedKeyboardSound || undefined,
      });
    } else {
      const profile = draftToProfile(draft, agentOrder.length);
      addAgent(profile); // status: 'starting'
      // 캐릭터 등장은 profiles prop 변화 → B의 syncAgents가 처리 (정합화)
      try {
        await tauriApi.createSession(profile.id, sessionOptsFor(profile)); // PTY 시작
      } catch (err) {
        // The profile is already saved; mark the session exited so clicking the
        // character later retries via the bridge's ensureSession.
        setSessionState({ agentId: profile.id, status: "exited" });
        console.warn(`ProfileDialog: createSession failed for ${profile.id}`, err);
      }
    }
    closeModal();
  };

  if (modal.kind !== "profile-create" && modal.kind !== "profile-edit") return null;
  return (
    <div
      className="modal-backdrop"
      // mousedown + target guard (not onClick), mirroring TerminalOverlay's
      // backdrop close (commit 7986f3d): PortraitEditor renders nested
      // inside this backdrop, and a plain onClick={closeModal} here would
      // catch every bubbled synthetic click from the nested editor (its
      // 저장/취소 buttons, 레트로 필터 checkbox, file input), closing this
      // dialog underneath it. mousedown fires at the press point and the
      // target === currentTarget check only matches an actual press on the
      // backdrop itself, never a bubbled event from a descendant — so
      // nested-editor interactions (and the retargeted click that follows
      // the editor's own backdrop-mousedown unmount) never reach this
      // handler at all.
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel profile-dialog">
        {/* ── 헤더 ─────────────────────────────────────────────── */}
        <header className="profile-dialog-header">
          <h2 className="pixel-title">{editing ? "에이전트 편집" : "새 에이전트"}</h2>
          <p className="profile-dialog-sub">
            {editing
              ? "프로필을 수정합니다. 저장하면 바로 반영됩니다."
              : "새 에이전트의 프로필을 만듭니다. 저장하면 터미널 세션이 시작됩니다."}
          </p>
        </header>

        {/* ── 정체성: 이름 · 역할 · 메모 · 아키타입 ────────────── */}
        <section className="form-section">
          <h3 className="form-section-title">정체성</h3>
          <div className="form-row-2">
            <div className="form-field">
              <label>
                <span className="form-label-text">이름</span>
                <input
                  value={draft.name}
                  onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                />
              </label>
            </div>
            <div className="form-field">
              <label>
                <span className="form-label-text">역할</span>
                <input
                  value={draft.role}
                  onChange={(e) => setDraft({ ...draft, role: e.target.value })}
                />
              </label>
            </div>
          </div>
          <div className="form-field">
            <label>
              <span className="form-label-text">메모</span>
              <textarea
                value={draft.note}
                onChange={(e) => setDraft({ ...draft, note: e.target.value })}
              />
            </label>
            <p className="form-hint">에이전트를 설명하는 자유 메모 — 초상 프롬프트에 함께 반영됩니다.</p>
          </div>
          <div className="form-field">
            <label>
              <span className="form-label-text">성격 프롬프트</span>
              <textarea
                value={draft.personalityPrompt ?? ""}
                onChange={(e) => setDraft({ ...draft, personalityPrompt: e.target.value })}
              />
            </label>
            <p className="form-hint">Claude Code의 시스템 프롬프트에 덧붙일 캐릭터 성격입니다. 여러 줄을 그대로 사용할 수 있습니다.</p>
          </div>
          <div className="form-field">
            <label>
              <span className="form-label-text">아키타입</span>
              <select
                value={draft.archetype ?? "auto"}
                onChange={(e) => setDraft({ ...draft, archetype: e.target.value })}
              >
                {ARCHETYPE_SELECT_OPTIONS.map((o) => (
                  <option key={o.value} value={o.value}>{o.label}</option>
                ))}
              </select>
            </label>
            <p className="form-hint">스프라이트의 체형·의상 계열을 정합니다. “자동(시드)”이면 시드에 따라 선택됩니다.</p>
          </div>
        </section>

        {/* ── 외형: 프리뷰 카드 + 외모 힌트 · 픽셀아트 의뢰 문구 ── */}
        <section className="form-section">
          <h3 className="form-section-title">외형</h3>
          <div className="profile-previews">
            <div className="sprite-preview">
              <span className="preview-card-title">스프라이트</span>
              <img
                src={spritePreviewUrl ?? spriteUrl}
                alt="sprite"
                width={96}
                height={96}
              />
              <div className="sprite-buttons">
                <button className="pixel-btn" onClick={regenSeed}>
                  스프라이트 재생성
                </button>
                {spritePreviewUrl && (
                  <span className="sprite-custom-badge">커스텀 사용 중 — 재생성은 외형에 영향 없음</span>
                )}
                <button className="pixel-btn" onClick={onCopySpritePrompt}>
                  픽셀아트 프롬프트 복사
                </button>
                {editing && editingAgent && (
                  <>
                    <button
                      className="pixel-btn"
                      onClick={onGeneratePixellab}
                      disabled={pixellabBusy}
                    >
                      {pixellabBusy ? "생성 중…" : "PixelLab로 생성"}
                    </button>
                    {pixellabNote && (
                      <span className="sprite-custom-badge">{pixellabNote}</span>
                    )}
                    <button className="pixel-btn" onClick={() => setSpriteEditorOpen(true)}>
                      {spritePreviewUrl ? "픽셀아트 변경" : "픽셀아트 업로드"}
                    </button>
                    {spritePreviewUrl && (
                      <button className="pixel-btn" onClick={onRemoveSprite}>
                        커스텀 제거
                      </button>
                    )}
                  </>
                )}
              </div>
            </div>
            <div className="portrait-section">
              <span className="preview-card-title">초상화</span>
              <div className="portrait-current">
                <img
                  // 호버 카드와 동일한 폴백 체인(초상 > 커스텀 스프라이트 프리뷰 >
                  // 프로시저럴) — spritePreviewUrl 누락 시 PixelLab 생성 후에도
                  // 생성 전 프로시저럴 이미지가 잔존하는 버그.
                  src={portraitUrl ?? spritePreviewUrl ?? spriteUrl}
                  alt="portrait"
                  width={90}
                  height={120}
                  style={{ objectFit: "cover", objectPosition: "top center", imageRendering: "pixelated" }}
                />
              </div>
              <div className="portrait-buttons">
                <button className="pixel-btn" onClick={onCopyPrompt}>
                  초상 프롬프트 복사
                </button>
                {editing && editingAgent && (
                  <>
                    <button className="pixel-btn" onClick={() => setEditorOpen(true)}>
                      {portraitUrl ? "이미지 변경" : "이미지 업로드"}
                    </button>
                    {portraitUrl && (
                      <button className="pixel-btn" onClick={onRemovePortrait}>
                        제거
                      </button>
                    )}
                  </>
                )}
              </div>
            </div>
          </div>
          <div className="form-field">
            <label>
              <span className="form-label-text">외모 힌트</span>
              <input
                value={draft.appearance ?? ""}
                onChange={(e) => setDraft({ ...draft, appearance: e.target.value })}
                placeholder="예: 짧은 검은 머리, 안경 (선택)"
              />
            </label>
          </div>
          <div className="form-field">
            <label>
              <span className="form-label-text">픽셀아트 의뢰 문구</span>
              <input
                value={draft.spriteRequest ?? ""}
                onChange={(e) => setDraft({ ...draft, spriteRequest: e.target.value })}
                placeholder="예: 빨간 망토를 두른 마법사 (선택, 비면 외모 힌트 사용)"
              />
            </label>
            <p className="form-hint">프롬프트 복사와 PixelLab 생성에 그대로 반영됩니다.</p>
          </div>
        </section>

        {/* ── 터미널: 시작 폴더 · 시작 명령어 · 셸 ─────────────── */}
        <section className="form-section">
          <h3 className="form-section-title">터미널</h3>
          <div className="form-field">
            <label>
              <span className="form-label-text">시작 폴더</span>
              <input
                value={draft.cwd ?? ""}
                onChange={(e) => setDraft({ ...draft, cwd: e.target.value })}
                placeholder="비워두면 홈 디렉터리"
              />
            </label>
          </div>
          <div className="form-field">
            <label>
              <span className="form-label-text">시작 명령어</span>
              <input
                value={draft.startupCommand ?? ""}
                onChange={(e) => setDraft({ ...draft, startupCommand: e.target.value })}
                placeholder="예: source ./init.sh 또는 mysetup.bat (선택, 새 터미널마다 실행)"
              />
            </label>
            <p className="form-hint">새 터미널 세션이 열릴 때마다 자동으로 실행됩니다.</p>
          </div>
          <div className="form-field">
            <label>
              <span className="form-label-text">키보드 소리</span>
              <select
                value={draft.keyboardSound ?? ""}
                onChange={(e) => {
                  setDraft({ ...draft, keyboardSound: e.target.value });
                  previewKeyboardSound(e.target.value || undefined, editingAgentId);
                }}
              >
                <option value="">기본</option>
                {KEYBOARD_SOUND_PACK_OPTIONS.map((p) => (
                  <option key={p.id} value={p.id}>{p.label}</option>
                ))}
              </select>
            </label>
            <p className="form-hint">이 에이전트가 타이핑할 때 나는 소리입니다. 고르면 미리 들려줍니다.</p>
          </div>
          {shells.length > 0 && (
            <div className="form-field">
              <label>
                <span className="form-label-text">셸</span>
                <select
                  value={draft.shell ?? ""}
                  onChange={(e) => setDraft({ ...draft, shell: e.target.value })}
                >
                  <option value="">자동 (기본)</option>
                  {shells.map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.label}
                      {!s.hooksSupported ? " (시간 추적 미지원)" : ""}
                    </option>
                  ))}
                </select>
              </label>
            </div>
          )}
        </section>

        {/* ── 액션 ─────────────────────────────────────────────── */}
        <div className="dialog-actions">
          {!editing && (
            <button className="pixel-btn dialog-action-aux" onClick={regenAll}>
              전체 랜덤
            </button>
          )}
          <button className="pixel-btn primary" onClick={onSave}>
            저장
          </button>
          <button className="pixel-btn" onClick={closeModal}>
            취소
          </button>
        </div>
      </div>
      {editorOpen && editingAgent && (
        <PortraitEditor
          agentId={editingAgent.id}
          onClose={() => setEditorOpen(false)}
        />
      )}
      {spriteEditorOpen && editingAgent && (
        <SpriteEditor
          agentId={editingAgent.id}
          initialImage={generatedImage ?? undefined}
          onClose={() => {
            setSpriteEditorOpen(false);
            setGeneratedImage(null);
          }}
        />
      )}
    </div>
  );
}
