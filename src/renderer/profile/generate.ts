// src/renderer/profile/generate.ts
//
// Pure random-draft generator + draft->AgentProfile normalizer. `pick()`
// reads `Math.random()` directly — no injected rng seam (tests pin this
// down with `vi.spyOn(Math, "random")` instead).
import { nanoid } from "nanoid";
import { NAME_WORDS, ROLE_WORDS, PERSONALITY_WORDS } from "./wordlists";
import type { AgentProfile } from "../store/types";
import type { BotConfig } from "@shared/types";
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
  /** Claude Code에 추가할 캐릭터 성격 프롬프트. 빈 문자열/공백 = 미지정. */
  personalityPrompt?: string;
  /** 외모 힌트(선택). 빈 문자열/공백 = 미지정. */
  appearance?: string;
  /** 픽셀아트 의뢰 문구(선택). 빈 문자열/공백 = 미지정. */
  spriteRequest?: string;
  /** 아키타입 선택. "auto" = 시드 추첨(저장 시 확정). 미지정도 "auto"로 취급. */
  archetype?: string;
  /** 키보드 사운드 팩 id(선택). 빈 문자열 = 기본 팩. */
  keyboardSound?: string;
  /** 봇 슬래시 slug 별칭(이슈 #57). 빈 문자열 = 이름에서 자동 파생. */
  botSlug?: string;
  /** 봇 화이트리스트(추가 허용 Gitea 계정). 콤마/줄바꿈 구분 입력. tea 로그인
   * 계정 본인은 항상 암묵 포함. */
  botWhitelist?: string;
  /** 봇 폴링 주기(초) 입력. 빈 문자열 = 기본 60. 하한 30. */
  botPollIntervalSec?: string;
  /** 봇 turn-taking 유휴 임계(ms). UI에 노출하지 않지만 편집 저장 시 유실되지
   * 않도록 draft에 실어 라운드트립한다(리뷰 M2). 빈 문자열 = 기본 3000. */
  botIdleQuietMs?: string;
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
    personalityPrompt: "",
    appearance: "",
    spriteRequest: "",
    archetype: "auto",
    keyboardSound: "",
    botSlug: "",
    botWhitelist: "",
    botPollIntervalSec: "",
    botIdleQuietMs: "",
  };
}

/** 봇 설정 입력을 `BotConfig`로 조립한다. 아무 값도 없으면 undefined(봇 미설정).
 * whitelist는 콤마/줄바꿈으로 나눠 트림·빈값 제거, 폴링 주기는 하한 30을 적용. */
export function buildBotConfig(d: DraftProfile): BotConfig | undefined {
  const slug = (d.botSlug ?? "").trim();
  const whitelist = (d.botWhitelist ?? "")
    .split(/[\n,]/)
    .map((s) => s.trim())
    .filter(Boolean);
  const pollRaw = (d.botPollIntervalSec ?? "").trim();
  const poll = pollRaw ? Number.parseInt(pollRaw, 10) : NaN;
  const hasPoll = Number.isFinite(poll) && poll > 0;
  const idleRaw = (d.botIdleQuietMs ?? "").trim();
  const idle = idleRaw ? Number.parseInt(idleRaw, 10) : NaN;
  const hasIdle = Number.isFinite(idle) && idle > 0;
  if (!slug && whitelist.length === 0 && !hasPoll && !hasIdle) return undefined;
  return {
    ...(slug ? { slug } : {}),
    whitelist,
    ...(hasPoll ? { pollIntervalSec: Math.max(30, poll) } : {}),
    ...(hasIdle ? { idleQuietMs: idle } : {}),
  };
}

export function draftToProfile(d: DraftProfile, deskIndex: number): AgentProfile {
  const cwd = (d.cwd ?? "").trim();
  const shell = (d.shell ?? "").trim();
  const startupCommand = (d.startupCommand ?? "").trim();
  const personalityPrompt = (d.personalityPrompt ?? "").trim();
  const appearance = (d.appearance ?? "").trim();
  const spriteRequest = (d.spriteRequest ?? "").trim();
  const keyboardSound = (d.keyboardSound ?? "").trim();
  const archetype = d.archetype && d.archetype !== "auto" ? d.archetype : pickArchetype(d.seed);
  const bot = buildBotConfig(d);
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
    ...(personalityPrompt ? { personalityPrompt } : {}),
    ...(appearance ? { appearance } : {}),
    ...(spriteRequest ? { spriteRequest } : {}),
    ...(keyboardSound ? { keyboardSound } : {}),
    ...(bot ? { bot } : {}),
  };
}
