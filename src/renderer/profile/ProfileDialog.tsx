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
import { generateDraft, draftToProfile, buildBotConfig, type DraftProfile } from "./generate";
import {
  portableFromDraft,
  applyBundleToDraft,
  serializeBundle,
  buildExportFileName,
} from "./characterIo";
import { parseCharacterBundle } from "@shared/types";
import { pngBase64ToDataUrl } from "../portrait/portraitCache";
import { loadSpritesFor } from "../sprite/spriteCache";
import { generateSpritePreview } from "../office/gen/characterFactory";
import { resolveArchetype, pickArchetype, archetypeOrAuto } from "../office/gen/archetypes";
import { tauriApi } from "../ipc/tauriApi";
import { sessionOptsFor } from "../ipc/sessionOpts";
import {
  buildPortraitPrompt,
  buildSpritePrompt,
  buildCodexPortraitPrompt,
  buildCodexSpritePrompt,
} from "../portrait/promptBuilder";
import { PortraitEditor } from "../portrait/PortraitEditor";
import { ArchetypePicker } from "./ArchetypePicker";
import {
  CodexGenPanel,
  codexGenErrorCaption,
  type CodexGenKind,
} from "../portrait/CodexGenPanel";
import { SpriteEditor } from "../sprite/SpriteEditor";
import { clearSpriteOverride } from "../office/gen/spriteOverrides";
import { clearMinimiOverride } from "../office/gen/minimiOverrides";
import { loadMinimisFor } from "../sprite/minimiCache";
import { KEYBOARD_SOUND_PACK_OPTIONS } from "../sound/packs";
import { previewKeyboardSound, previewVoice } from "../sound/soundManager";
import type { AvailableShell, TtsVoiceOption } from "@shared/types";
import "../portrait/portrait.css";

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
  const setPortrait = useAppStore((s) => s.setPortrait);
  const portraitUrl = useAppStore((s) =>
    editingAgent ? s.portraits[editingAgent.id] : undefined
  );
  const [editorOpen, setEditorOpen] = useState(false);
  const removeSpritePreview = useAppStore((s) => s.removeSpritePreview);
  const spritePreviewUrl = useAppStore((s) =>
    editingAgent ? s.spritePreviews[editingAgent.id] : undefined
  );
  const [spriteEditorOpen, setSpriteEditorOpen] = useState(false);
  const removeMinimiPreview = useAppStore((s) => s.removeMinimiPreview);
  const minimiPreviewUrl = useAppStore((s) =>
    editingAgent ? s.minimiPreviews[editingAgent.id] : undefined
  );
  const [minimiEditorOpen, setMinimiEditorOpen] = useState(false);
  /** Codex 생성 결과 data URL — 각각 SpriteEditor/PortraitEditor initialImage로 전달. */
  const [generatedImage, setGeneratedImage] = useState<string | null>(null);
  const [generatedPortrait, setGeneratedPortrait] = useState<string | null>(null);
  /** "외형" 섹션 모드. 직접 만들기(프롬프트 복사 + 업로드) / Codex로 생성.
   * 한 번에 하나만 보여 준다 — 두 경로를 나란히 두면 두서없이 보인다. */
  const [appearanceMode, setAppearanceMode] = useState<"manual" | "codex">("manual");
  const [codexBusy, setCodexBusy] = useState<CodexGenKind | null>(null);
  const [codexNote, setCodexNote] = useState<string | null>(null);
  /** 진행 중 생성 요청의 세션 토큰 — 편집 대상이 바뀌거나 다이얼로그가
   * 닫히면 무효화된다 (상시 마운트 컴포넌트라 unmount 가드는 무의미). */
  const codexSeqRef = useRef(0);

  const [draft, setDraft] = useState<DraftProfile>(() => generateDraft());
  const [spriteUrl, setSpriteUrl] = useState<string>("");
  const [shells, setShells] = useState<AvailableShell[]>([]);
  /** 캐릭터 내보내기/가져오기(이슈 #77) 진행 표시 + 결과/오류 캡션. */
  const [ioBusy, setIoBusy] = useState(false);
  const [ioNote, setIoNote] = useState<string | null>(null);

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
    codexSeqRef.current++;
    setCodexBusy(null);
    setCodexNote(null);
    setGeneratedImage(null);
    setGeneratedPortrait(null);
    setIoBusy(false);
    setIoNote(null);
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
      voiceId: agent.voiceId ?? "",
      botSlug: agent.bot?.slug ?? "",
      botWhitelist: (agent.bot?.whitelist ?? []).join(", "),
      botPollIntervalSec: agent.bot?.pollIntervalSec ? String(agent.bot.pollIntervalSec) : "",
      botIdleQuietMs: agent.bot?.idleQuietMs ? String(agent.bot.idleQuietMs) : "",
    });
  }, [editingAgentId]);

  // seed 또는 archetype 변경 시 라이브 스프라이트 프리뷰 (B의 순수 함수 — 동기, 아키타입 반영)
  useEffect(() => {
    const eff = resolveArchetype(archetypeOrAuto(draft.archetype), draft.seed);
    setSpriteUrl(generateSpritePreview(draft.seed, 6, undefined, undefined, eff));
  }, [draft.seed, draft.archetype]);

  const regenSeed = () => setDraft((d) => ({ ...d, seed: nanoid(8) }));
  const regenAll = () => setDraft(generateDraft());

  // 시작 폴더를 네이티브 폴더 선택 다이얼로그로 지정 — 텍스트 입력과 병행.
  // 현재 입력값이 실존 폴더면 그 위치에서 다이얼로그를 연다.
  const onBrowseCwd = async () => {
    try {
      const picked = await tauriApi.pickDirectory(draft.cwd?.trim() || undefined);
      if (picked) setDraft((d) => ({ ...d, cwd: picked }));
    } catch (err) {
      console.warn("폴더 선택 다이얼로그 실패", err);
    }
  };

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

  /** codex CLI로 초상/스프라이트 원본 1장을 만든 뒤, 해당 크롭 편집기를
   * 프리로드해 연다. 규격화(240×320 / 4프레임 시트)는 편집기가 담당한다. */
  const onGenerateCodex = async (kind: CodexGenKind) => {
    if (codexBusy || !editingAgent) return;
    const seq = ++codexSeqRef.current;
    const targetAgentId = editingAgent.id;
    /** 응답 적용 가능 여부: 토큰 유효 + 같은 에이전트의 편집 모달이 여전히 열려 있음. */
    const stillCurrent = () => {
      const m = useAppStore.getState().modal;
      return (
        codexSeqRef.current === seq &&
        m.kind === "profile-edit" &&
        m.agentId === targetAgentId
      );
    };
    setCodexBusy(kind);
    setCodexNote(null);
    const prompt =
      kind === "portrait"
        ? buildCodexPortraitPrompt({
            name: draft.name,
            role: draft.role,
            note: draft.note,
            appearance: draft.appearance,
            seed: draft.seed,
            archetype: draft.archetype,
          })
        : buildCodexSpritePrompt({
            name: draft.name,
            role: draft.role,
            spriteRequest: draft.spriteRequest,
            appearance: draft.appearance,
            seed: draft.seed,
            archetype: draft.archetype,
          });
    try {
      const res = await tauriApi.generateCodexImage(prompt);
      if (!stillCurrent()) return;
      const url = `data:image/png;base64,${res.pngBase64}`;
      if (kind === "portrait") {
        setGeneratedPortrait(url);
        setEditorOpen(true);
      } else {
        setGeneratedImage(url);
        setSpriteEditorOpen(true);
      }
      setCodexNote("생성 완료 — 편집기에서 확인하고 저장하세요.");
    } catch (err) {
      if (!stillCurrent()) return;
      setCodexNote(codexGenErrorCaption(err));
    } finally {
      if (codexSeqRef.current === seq) setCodexBusy(null);
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

  const onRemoveMinimi = async () => {
    if (!editingAgent) return;
    try {
      await tauriApi.deleteMinimi(editingAgent.id);
    } catch (err) {
      console.warn("ProfileDialog: deleteMinimi failed", err);
    }
    clearMinimiOverride(editingAgent.id);
    removeMinimiPreview(editingAgent.id);
    updateAgent(editingAgent.id, { minimiUpdatedAt: undefined });
  };

  // ── 캐릭터 내보내기(이슈 #77) ──────────────────────────────
  // 현재 편집 draft(진행 중 편집 반영) + 백엔드에 저장된 초상/스프라이트를 모아
  // 자기완결형 번들 파일로 쓴다. 로컬 환경 필드(cwd/셸/시작명령/봇)는 제외한다.
  const onExportCharacter = async () => {
    if (ioBusy || !editingAgent) return;
    setIoBusy(true);
    setIoNote(null);
    try {
      const profile = portableFromDraft(draft);
      const [portraitB64, spriteB64, minimiB64] = await Promise.all([
        tauriApi.loadPortrait(editingAgent.id),
        tauriApi.loadSprite(editingAgent.id),
        tauriApi.loadMinimi(editingAgent.id),
      ]);
      const json = serializeBundle(
        profile,
        portraitB64 ?? undefined,
        spriteB64 ?? undefined,
        minimiB64 ?? undefined,
      );
      const saved = await tauriApi.exportCharacterFile(buildExportFileName(profile.name), json);
      setIoNote(saved ? "내보냈습니다." : null); // null=사용자가 취소
    } catch (err) {
      console.warn("ProfileDialog: exportCharacter failed", err);
      setIoNote("내보내기에 실패했습니다.");
    } finally {
      setIoBusy(false);
    }
  };

  // ── 캐릭터 가져오기(이슈 #77) ──────────────────────────────
  // 편집 중인 캐릭터에 번들을 적용한다(새 캐릭터를 만들지 않는다). 이미지는
  // replace 시맨틱: 번들에 있으면 저장·표시, 없으면 기존 이미지를 제거해 소스와
  // 똑같은 외형이 되게 한다. 텍스트 필드는 draft에 실어 ‘저장’ 시 확정한다.
  const onImportCharacter = async () => {
    if (ioBusy || !editingAgent) return;
    setIoBusy(true);
    setIoNote(null);
    const id = editingAgent.id;
    try {
      const text = await tauriApi.importCharacterFile();
      if (text == null) {
        setIoBusy(false);
        return; // 사용자가 취소
      }
      const res = parseCharacterBundle(text);
      if (!res.ok) {
        setIoNote(res.error);
        setIoBusy(false);
        return;
      }
      const b = res.bundle;

      if (b.portraitPngBase64) {
        await tauriApi.savePortrait(id, b.portraitPngBase64);
        setPortrait(id, pngBase64ToDataUrl(b.portraitPngBase64));
        updateAgent(id, { portraitUpdatedAt: Date.now() });
      } else {
        await tauriApi.deletePortrait(id);
        removePortrait(id);
        updateAgent(id, { portraitUpdatedAt: undefined });
      }

      if (b.spritePngBase64) {
        await tauriApi.saveSprite(id, b.spritePngBase64);
        updateAgent(id, { spriteUpdatedAt: Date.now() });
        await loadSpritesFor([id]); // 백엔드에서 디코드 → 오버라이드 + 프리뷰 갱신
      } else {
        await tauriApi.deleteSprite(id);
        clearSpriteOverride(id);
        removeSpritePreview(id);
        updateAgent(id, { spriteUpdatedAt: undefined });
      }

      // 미니미도 스프라이트와 같은 replace 시맨틱(번들에 없으면 제거).
      if (b.minimiPngBase64) {
        await tauriApi.saveMinimi(id, b.minimiPngBase64);
        updateAgent(id, { minimiUpdatedAt: Date.now() });
        await loadMinimisFor([id]); // 백엔드에서 디코드 → 오버라이드 + 프리뷰 갱신
      } else {
        await tauriApi.deleteMinimi(id);
        clearMinimiOverride(id);
        removeMinimiPreview(id);
        updateAgent(id, { minimiUpdatedAt: undefined });
      }

      setDraft((d) => applyBundleToDraft(d, b.profile));
      setIoNote("가져왔습니다. ‘저장’을 눌러 반영하세요.");
    } catch (err) {
      console.warn("ProfileDialog: importCharacter failed", err);
      setIoNote("가져오기에 실패했습니다.");
    } finally {
      setIoBusy(false);
    }
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
      const trimmedVoiceId = (draft.voiceId ?? "").trim();
      // 목록에 없는 자유 입력도 그대로 저장한다(공백만 다듬는다) — 스프라이트는
      // human으로 폴백하고 그림 프롬프트에는 적은 문구가 들어간다.
      const trimmedArchetype = (draft.archetype ?? "").trim();
      const chosenArchetype =
        trimmedArchetype && trimmedArchetype !== "auto"
          ? trimmedArchetype
          : pickArchetype(draft.seed);
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
        voiceId: trimmedVoiceId || undefined,
        bot: buildBotConfig(draft),
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
              <ArchetypePicker
                value={draft.archetype ?? "auto"}
                onChange={(v) => setDraft({ ...draft, archetype: v })}
              />
            </label>
            <p className="form-hint">
              스프라이트의 체형·의상 계열을 정합니다. “자동(시드)”이면 시드에 따라
              선택됩니다. 목록에 없는 종족(예: 드래곤)을 적어 넣으면 초상·픽셀아트
              프롬프트에 그대로 반영되고, 도트 캐릭터는 인간 체형으로 그려집니다.
            </p>
          </div>
        </section>

        {/* ── 외형: 프리뷰 카드 + 외모 힌트 · 픽셀아트 의뢰 문구 ── */}
        <section className="form-section">
          <h3 className="form-section-title">외형</h3>
          <div className="profile-previews">
            <div className="portrait-section">
              <span className="preview-card-title">초상화</span>
              <div className="portrait-current">
                <img
                  // 호버 카드와 동일한 폴백 체인(초상 > 커스텀 스프라이트 프리뷰 >
                  // 프로시저럴) — spritePreviewUrl 누락 시 스프라이트 생성 후에도
                  // 생성 전 프로시저럴 이미지가 잔존하는 버그.
                  src={portraitUrl ?? spritePreviewUrl ?? spriteUrl}
                  alt="portrait"
                  width={90}
                  height={120}
                  style={{ objectFit: "cover", objectPosition: "top center", imageRendering: "pixelated" }}
                />
              </div>
              <div className="portrait-buttons">
                {editing && editingAgent && portraitUrl && (
                  <button className="pixel-btn" onClick={onRemovePortrait}>
                    초상 제거
                  </button>
                )}
              </div>
            </div>
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
                {editing && editingAgent && spritePreviewUrl && (
                  <button className="pixel-btn" onClick={onRemoveSprite}>
                    커스텀 제거
                  </button>
                )}
              </div>
              {/* 서브에이전트 미니미 — 머리 옆에 뜨는 작은 분신. 지정이 없으면
                  스프라이트를 그대로 축소해 쓴다. */}
              <div className="minimi-subsection">
                <span className="preview-card-title">미니미</span>
                <img
                  src={minimiPreviewUrl ?? spritePreviewUrl ?? spriteUrl}
                  alt="minimi"
                  width={48}
                  height={48}
                  style={{ imageRendering: "pixelated" }}
                />
                <span className="sprite-custom-badge">
                  {minimiPreviewUrl
                    ? "커스텀 미니미 사용 중"
                    : "지정 없음 — 스프라이트를 축소해 사용합니다"}
                </span>
                {editing && editingAgent && (
                  <div className="sprite-buttons">
                    <button className="pixel-btn" onClick={() => setMinimiEditorOpen(true)}>
                      {minimiPreviewUrl ? "미니미 변경" : "미니미 업로드"}
                    </button>
                    {minimiPreviewUrl && (
                      <button className="pixel-btn" onClick={onRemoveMinimi}>
                        미니미 제거
                      </button>
                    )}
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* 만드는 방법은 한 번에 하나만 — 직접 만들기와 Codex 생성을 나란히
              늘어놓으면 무엇을 눌러야 할지 알 수 없다. SettingsDialog와 같은
              tablist 관례를 작은 크기로 재사용한다. */}
          <div className="appearance-tabs" role="tablist" aria-label="외형 만들기 방법">
            {([
              { id: "manual", label: "직접 만들기" },
              { id: "codex", label: "Codex로 생성" },
            ] as const).map((t) => (
              <button
                key={t.id}
                type="button"
                role="tab"
                id={`appearance-tab-${t.id}`}
                aria-selected={appearanceMode === t.id}
                aria-controls={`appearance-tabpanel-${t.id}`}
                className={
                  appearanceMode === t.id
                    ? "appearance-tab appearance-tab-active"
                    : "appearance-tab"
                }
                onClick={() => setAppearanceMode(t.id)}
              >
                {t.label}
              </button>
            ))}
          </div>
          <div
            className="appearance-tabpanel"
            role="tabpanel"
            id={`appearance-tabpanel-${appearanceMode}`}
            aria-labelledby={`appearance-tab-${appearanceMode}`}
          >
            {appearanceMode === "manual" ? (
              <>
                <p className="form-hint">
                  프롬프트를 복사해 원하는 이미지 생성 도구에 넣고, 결과 이미지를
                  올리면 크롭 편집기가 규격에 맞춰 줍니다.
                </p>
                <div className="appearance-manual-row">
                  <span className="form-label-text">초상화</span>
                  <div className="portrait-buttons">
                    <button className="pixel-btn" onClick={onCopyPrompt}>
                      초상 프롬프트 복사
                    </button>
                    {editing && editingAgent && (
                      <button className="pixel-btn" onClick={() => setEditorOpen(true)}>
                        {portraitUrl ? "이미지 변경" : "이미지 업로드"}
                      </button>
                    )}
                  </div>
                </div>
                <div className="appearance-manual-row">
                  <span className="form-label-text">스프라이트</span>
                  <div className="sprite-buttons">
                    <button className="pixel-btn" onClick={onCopySpritePrompt}>
                      픽셀아트 프롬프트 복사
                    </button>
                    {editing && editingAgent && (
                      <button className="pixel-btn" onClick={() => setSpriteEditorOpen(true)}>
                        {spritePreviewUrl ? "픽셀아트 변경" : "픽셀아트 업로드"}
                      </button>
                    )}
                  </div>
                </div>
                {!(editing && editingAgent) && (
                  <p className="form-hint">저장한 뒤 편집에서 이미지를 올릴 수 있습니다.</p>
                )}
              </>
            ) : (
              <CodexGenPanel
                enabled={Boolean(editing && editingAgent)}
                busy={codexBusy}
                note={codexNote}
                onGenerate={onGenerateCodex}
              />
            )}
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
            <p className="form-hint">프롬프트 복사와 Codex 생성에 그대로 반영됩니다.</p>
          </div>
        </section>

        {/* ── 터미널: 시작 폴더 · 시작 명령어 · 셸 ─────────────── */}
        <section className="form-section">
          <h3 className="form-section-title">터미널</h3>
          <div className="form-field">
            <label>
              <span className="form-label-text">시작 폴더</span>
              <div className="form-control-row">
                <input
                  value={draft.cwd ?? ""}
                  onChange={(e) => setDraft({ ...draft, cwd: e.target.value })}
                  placeholder="비워두면 홈 디렉터리 (직접 입력·붙여넣기 가능)"
                />
                <button type="button" className="pixel-btn" onClick={onBrowseCwd}>
                  찾아보기…
                </button>
              </div>
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
          <VoiceField
            draft={draft}
            agentId={editingAgentId}
            onChange={(voiceId) => setDraft((d) => ({ ...d, voiceId }))}
          />
          <div className="form-field">
            <span className="form-label-text">봇 모드 설정</span>
            <p className="form-hint">
              터미널 탭 우클릭 → “봇 모드 시작”으로 켜면, 이 캐릭터가 담당 저장소의 Gitea
              이슈에 달린 슬래시 명령에 반응해 자동으로 작업합니다. 아래는 봇의 지속 설정입니다.
            </p>
          </div>
          <div className="form-field">
            <label>
              <span className="form-label-text">봇 슬래시 별칭</span>
              <input
                value={draft.botSlug ?? ""}
                onChange={(e) => setDraft({ ...draft, botSlug: e.target.value })}
                placeholder="예: nova (선택, 비우면 이름에서 자동 파생)"
              />
            </label>
            <p className="form-hint">
              이슈에서 <code>/별칭</code> 으로 이 캐릭터를 호출합니다. 비우면 이름에서 파생합니다(공백 제거·소문자).
            </p>
          </div>
          <div className="form-field">
            <label>
              <span className="form-label-text">봇 화이트리스트</span>
              <input
                value={draft.botWhitelist ?? ""}
                onChange={(e) => setDraft({ ...draft, botWhitelist: e.target.value })}
                placeholder="추가 허용 Gitea 계정, 콤마 구분 (선택)"
              />
            </label>
            <p className="form-hint">명령을 발동할 수 있는 계정. tea 로그인 계정 본인은 항상 포함됩니다.</p>
          </div>
          <div className="form-field">
            <label>
              <span className="form-label-text">봇 폴링 주기(초)</span>
              <input
                type="number"
                min={30}
                value={draft.botPollIntervalSec ?? ""}
                onChange={(e) => setDraft({ ...draft, botPollIntervalSec: e.target.value })}
                placeholder="기본 60, 하한 30"
              />
            </label>
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
          {editing && editingAgent && (
            <div className="dialog-io-group">
              <button
                className="pixel-btn"
                onClick={onExportCharacter}
                disabled={ioBusy}
              >
                내보내기
              </button>
              <button
                className="pixel-btn"
                onClick={onImportCharacter}
                disabled={ioBusy}
              >
                가져오기
              </button>
              {ioNote && <span className="profile-io-note">{ioNote}</span>}
            </div>
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
          initialImage={generatedPortrait ?? undefined}
          onClose={() => {
            setEditorOpen(false);
            setGeneratedPortrait(null);
          }}
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
      {minimiEditorOpen && editingAgent && (
        <SpriteEditor
          agentId={editingAgent.id}
          target="minimi"
          onClose={() => setMinimiEditorOpen(false)}
        />
      )}
    </div>
  );
}

/**
 * 대사 TTS 보이스 선택 + 그 자리 미리듣기.
 *
 * 기본은 "자동" — 캐릭터 종족(archetype)에 어울리는 성별·연령 라벨로 후보를
 * 좁힌 뒤 시드 해시로 고정 배정한다(백엔드 `tts::voice`). 여기서 고르는 것은
 * 그 자동 배정을 덮어쓰는 수동 지정이다.
 *
 * 목록은 백엔드가 ElevenLabs에서 1회 조회해 캐시한 것과 **같은 것**이라, 여기
 * 보이는 이름이 실제 발화 목소리와 어긋나지 않는다. 키 값은 오지 않는다.
 * TTS가 꺼져 있거나 키가 없으면 고를 것이 없으므로 비활성 + 사유 안내.
 */
function VoiceField({
  draft,
  agentId,
  onChange,
}: {
  draft: DraftProfile;
  agentId?: string;
  onChange: (voiceId: string) => void;
}) {
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
        setNote(
          String(err).startsWith("missing_elevenlabs_key")
            ? "설정에서 ElevenLabs API 키를 저장하면 목소리를 고를 수 있습니다."
            : `목소리 목록을 불러오지 못했습니다: ${String(err)}`
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
      setNote(line ? `발화: ${line}` : "발화할 수 없었습니다.");
    } catch (err) {
      setNote(`미리듣기 실패: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="form-field">
      <label>
        <span className="form-label-text">목소리 (TTS)</span>
        <div className="form-control-row">
          <select
            value={selected}
            disabled={disabled}
            onChange={(e) => onChange(e.target.value)}
          >
            <option value="">자동 (종족에 맞춰 배정)</option>
            {missing && <option value={selected}>{selected} (목록에 없음)</option>}
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
            미리듣기
          </button>
        </div>
      </label>
      <p className="form-hint">
        {!ttsEnabled
          ? "설정에서 ‘알림 대사 읽어주기(TTS)’를 켜면 목소리를 고를 수 있습니다."
          : "비워두면 캐릭터 종족과 시드에 맞춰 자동으로 정해집니다."}
        {muted && " 무음 모드가 켜져 있어 실제 알림은 발화되지 않습니다(미리듣기는 들립니다)."}
      </p>
      {note && <p className="form-hint">{note}</p>}
    </div>
  );
}
