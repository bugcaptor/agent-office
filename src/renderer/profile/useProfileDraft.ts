// src/renderer/profile/useProfileDraft.ts
//
// 프로필 다이얼로그가 편집 중인 **초안**(draft)과 거기서 곧장 나오는 파생값들.
//
// 초안은 저장을 누르기 전까지 스토어에 닿지 않는다 — 그래서 이 훅이 사는 동안
// 스토어를 읽는 곳은 편집 모드 진입 시 기존 값을 한 번 퍼 오는 자리 하나뿐이다.
import { useEffect, useMemo, useState } from "react";
import { nanoid } from "nanoid";

import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { generateSpritePreview } from "../office/gen/characterFactory";
import {
  archetypeOrAuto,
  basePaletteFor,
  keyColorsFor,
  resolveArchetype,
} from "../office/gen/archetypes";
import { generateDraft, type DraftProfile } from "./generate";
import type { AvailableShell, PaletteSlot } from "@shared/types";

/** 편집 중인 캐릭터 초안과 그 파생값. 다이얼로그가 뜬 동안 사는 상태다 —
 *  저장(`onSave`)에 닿기 전까지 스토어에는 아무것도 반영되지 않는다. */
export function useProfileDraft(editingAgentId: string | undefined) {
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
    if (!editingAgentId) return;
    const agent = useAppStore.getState().agents[editingAgentId];
    if (!agent) return;
    setDraft({
      name: agent.name,
      role: agent.role,
      seed: agent.seed,
      cwd: agent.cwd ?? "",
      shell: agent.shell ?? "",
      startupCommand: agent.startupCommand ?? "",
      // 레거시 메모는 백엔드(`ProfileStore::load`)가 이미 성격 프롬프트로
      // 합쳐 실어 준다 — 여기서는 그대로 보여 주기만 한다.
      personalityPrompt: agent.personalityPrompt ?? "",
      portraitRequest: agent.portraitRequest ?? "",
      spriteRequest: agent.spriteRequest ?? "",
      minimiRequest: agent.minimiRequest ?? "",
      archetype: agent.archetype ?? "auto",
      colors: agent.colors ?? {},
      keyboardSound: agent.keyboardSound ?? "",
      voiceId: agent.voiceId ?? "",
      botSlug: agent.bot?.slug ?? "",
      botWhitelist: (agent.bot?.whitelist ?? []).join(", "),
      botPollIntervalSec: agent.bot?.pollIntervalSec ? String(agent.bot.pollIntervalSec) : "",
      botIdleQuietMs: agent.bot?.idleQuietMs ? String(agent.bot.idleQuietMs) : "",
      talkReceive: agent.talkReceive !== false,
      tmuxHost: agent.tmuxHost === true,
    });
  }, [editingAgentId]);

  // seed 또는 archetype 변경 시 라이브 스프라이트 프리뷰 (B의 순수 함수 — 동기, 아키타입 반영)
  useEffect(() => {
    const eff = resolveArchetype(archetypeOrAuto(draft.archetype), draft.seed);
    setSpriteUrl(generateSpritePreview(draft.seed, 6, undefined, undefined, eff, draft.colors));
  }, [draft.seed, draft.archetype, draft.colors]);

  /** 그림 프롬프트에 그대로 실리는 키 컬러(시드+아키타입 결정, 사용자 오버라이드
   * 반영). 내부 자료로만 두면 "왜 이 색인지" 알 수 없어 편집창에 그대로 노출하고,
   * 칩을 누르면 그 자리에서 색만 갈아 끼울 수 있다(kbm #2fj). */
  const keyColors = useMemo(
    () => keyColorsFor(draft.seed, archetypeOrAuto(draft.archetype), draft.colors),
    [draft.seed, draft.archetype, draft.colors],
  );

  /** 시드+아키타입이 정하는 기본 팔레트 — 피커의 "기본값으로"가 돌아갈 색. */
  const basePalette = useMemo(
    () => basePaletteFor(draft.seed, archetypeOrAuto(draft.archetype)),
    [draft.seed, draft.archetype],
  );

  /** 슬롯 하나의 색을 확정/해제한다. 해제는 키를 지워 시드 기본색으로 되돌린다 —
   *  나중에 시드나 아키타입을 바꿔도 색이 따라 움직이게 하기 위해서다. */
  const setSlotColor = (slot: PaletteSlot, hex: string | null) =>
    setDraft((d) => {
      const next = { ...(d.colors ?? {}) };
      if (hex) next[slot] = hex;
      else delete next[slot];
      return { ...d, colors: next };
    });

  const regenSeed = () => setDraft((d) => ({ ...d, seed: nanoid(8) }));
  const regenAll = () => setDraft(generateDraft());

  // 시작 폴더를 네이티브 폴더 선택 다이얼로그로 지정 — 텍스트 입력과 병행.
  // 현재 입력값이 실존 폴더면 그 위치에서 다이얼로그를 연다.
  const onBrowseCwd = async () => {
    try {
      const picked = await tauriApi.pickDirectory(draft.cwd?.trim() || undefined);
      if (picked) setDraft((d) => ({ ...d, cwd: picked }));
    } catch (err) {
      console.warn("ProfileDialog: pickDirectory failed", err);
    }
  };

  return {
    draft,
    setDraft,
    /** 시드·아키타입·색에서 즉석으로 그린 프로시저럴 스프라이트 미리보기. */
    spriteUrl,
    /** Windows에서만 채워진다(그 외 플랫폼은 빈 배열 → 셀렉터 미노출). */
    shells,
    keyColors,
    basePalette,
    setSlotColor,
    regenSeed,
    regenAll,
    onBrowseCwd,
  };
}
