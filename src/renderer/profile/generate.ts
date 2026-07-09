// src/renderer/profile/generate.ts
//
// Pure random-draft generator + draft->AgentProfile normalizer. `pick()`
// reads `Math.random()` directly — no injected rng seam (tests pin this
// down with `vi.spyOn(Math, "random")` instead).
import { nanoid } from "nanoid";
import { NAME_WORDS, ROLE_WORDS, PERSONALITY_WORDS } from "./wordlists";
import type { AgentProfile } from "../store/types";
import { pickArchetype } from "../office/gen/archetypes";

const pick = <T,>(arr: T[]): T => arr[Math.floor(Math.random() * arr.length)];

export interface DraftProfile {
  name: string;
  role: string;
  note: string; // personality 기반 초기 노트
  seed: string;
  /** 시작 폴더 입력 값. 빈 문자열 = 홈 디렉터리. */
  cwd?: string;
  /** 셸 선택 값. 빈 문자열 = 자동/기본 셸. */
  shell?: string;
  /** 시작 명령어(선택). 빈 문자열/공백 = 미지정 → 세션에서 주입 안 함. */
  startupCommand?: string;
  /** 외모 힌트(선택). 빈 문자열/공백 = 미지정. */
  appearance?: string;
  /** 픽셀아트 의뢰 문구(선택). 빈 문자열/공백 = 미지정. */
  spriteRequest?: string;
  /** 아키타입 선택. "auto" = 시드 추첨(저장 시 확정). 미지정도 "auto"로 취급. */
  archetype?: string;
}

export function generateDraft(): DraftProfile {
  const personality = pick(PERSONALITY_WORDS);
  return {
    name: pick(NAME_WORDS),
    role: pick(ROLE_WORDS),
    note: `${personality} 성격`,
    seed: nanoid(8),
    cwd: "",
    shell: "",
    startupCommand: "",
    appearance: "",
    spriteRequest: "",
    archetype: "auto",
  };
}

export function draftToProfile(d: DraftProfile, deskIndex: number): AgentProfile {
  const cwd = (d.cwd ?? "").trim();
  const shell = (d.shell ?? "").trim();
  const startupCommand = (d.startupCommand ?? "").trim();
  const appearance = (d.appearance ?? "").trim();
  const spriteRequest = (d.spriteRequest ?? "").trim();
  const archetype = d.archetype && d.archetype !== "auto" ? d.archetype : pickArchetype(d.seed);
  return {
    id: nanoid(),
    name: d.name.trim() || pick(NAME_WORDS),
    role: d.role.trim(),
    note: d.note.trim(),
    seed: d.seed,
    createdAt: Date.now(),
    deskIndex,
    archetype,
    ...(cwd ? { cwd } : {}),
    ...(shell ? { shell } : {}),
    ...(startupCommand ? { startupCommand } : {}),
    ...(appearance ? { appearance } : {}),
    ...(spriteRequest ? { spriteRequest } : {}),
  };
}
