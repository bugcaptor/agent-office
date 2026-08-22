// src/renderer/profile/generate.ts
//
// Pure random-draft generator + draft->AgentProfile normalizer. `pick()`
// reads `Math.random()` directly — no injected rng seam (tests pin this
// down with `vi.spyOn(Math, "random")` instead).
import { nanoid } from "nanoid";
import { NAME_WORDS, ROLE_WORDS, PERSONALITY_WORDS } from "./wordlists";
import type { AgentProfile } from "../store/types";
import type { BotConfig, ColorOverrides } from "@shared/types";
import { pickArchetype } from "../office/gen/archetypes";

const pick = <T,>(arr: T[]): T => arr[Math.floor(Math.random() * arr.length)];

export interface DraftProfile {
  name: string;
  role: string;
  seed: string;
  /** 시작 폴더 입력 값. 빈 문자열 = 홈 디렉터리. */
  cwd?: string;
  /** 셸 선택 값. 빈 문자열 = 자동/기본 셸. */
  shell?: string;
  /** 시작 명령어(선택). 빈 문자열/공백 = 미지정 → 세션에서 주입 안 함. */
  startupCommand?: string;
  /** Claude Code에 추가할 캐릭터 성격 프롬프트. 빈 문자열/공백 = 미지정. */
  personalityPrompt?: string;
  /** 초상화 추가 프롬프트(선택). 빈 문자열/공백 = 미지정. */
  portraitRequest?: string;
  /** 스프라이트 추가 프롬프트(선택). 빈 문자열/공백 = 미지정. */
  spriteRequest?: string;
  /** 미니미(소환수) 추가 프롬프트(선택). 빈 문자열/공백 = 미지정 → 자동 위임 문구. */
  minimiRequest?: string;
  /** 아키타입 선택. "auto" = 시드 추첨(저장 시 확정). 미지정도 "auto"로 취급. */
  archetype?: string;
  /** 팔레트 슬롯별 색 오버라이드. 부재/빈 객체 = 시드 기본색 그대로. */
  colors?: ColorOverrides;
  /** 키보드 사운드 팩 id(선택). 빈 문자열 = 기본 팩. */
  keyboardSound?: string;
  /** 대사 TTS 보이스 id(선택). 빈 문자열 = 종족 기반 자동 캐스팅. */
  voiceId?: string;
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

/**
 * 레거시 `note`를 성격 프롬프트에 합친다(순수). 메모 입력창을 없애고 성격
 * 프롬프트 하나로 통합하면서, 옛 번들에 남아 있는 메모가 유실되지 않게
 * 가져올 때 이 함수를 통과시킨다. 저장된 프로필 쪽은 백엔드
 * `ProfileStore::load`의 `migrate_loaded`가 같은 규칙으로 먼저 통합한다.
 *
 * - 둘 중 하나만 있으면 그것을 쓴다.
 * - 둘 다 있으면 줄바꿈으로 잇는다. 단 성격 프롬프트가 이미 메모 문구를
 *   포함하면(이전에 한 번 합쳐진 경우) 그대로 둔다 — 반복 병합 방지.
 */
export function mergeLegacyNote(
  personalityPrompt: string | undefined,
  note: string | undefined
): string {
  const p = (personalityPrompt ?? "").trim();
  const n = (note ?? "").trim();
  if (!n) return p;
  if (!p) return n;
  if (p.includes(n)) return p;
  return `${p}\n${n}`;
}

export function generateDraft(): DraftProfile {
  const personality = pick(PERSONALITY_WORDS);
  return {
    name: pick(NAME_WORDS),
    role: pick(ROLE_WORDS),
    seed: nanoid(8),
    cwd: "",
    shell: "",
    startupCommand: "",
    personalityPrompt: `${personality} 성격`,
    portraitRequest: "",
    spriteRequest: "",
    minimiRequest: "",
    archetype: "auto",
    colors: {},
    keyboardSound: "",
    voiceId: "",
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

/** 빈 슬롯을 걷어낸 색 오버라이드. 남는 게 없으면 undefined — 저장된 프로필에
 *  빈 객체가 쌓이지 않게 한다(부재 = 기본색이라는 계약과 같은 뜻). */
export function normalizeColors(colors: ColorOverrides | undefined): ColorOverrides | undefined {
  if (!colors) return undefined;
  const out: ColorOverrides = {};
  for (const slot of ["skin", "hair", "shirt"] as const) {
    const v = (colors[slot] ?? "").trim();
    if (v) out[slot] = v;
  }
  return Object.keys(out).length > 0 ? out : undefined;
}

export function draftToProfile(d: DraftProfile, deskIndex: number): AgentProfile {
  const cwd = (d.cwd ?? "").trim();
  const shell = (d.shell ?? "").trim();
  const startupCommand = (d.startupCommand ?? "").trim();
  const personalityPrompt = (d.personalityPrompt ?? "").trim();
  const portraitRequest = (d.portraitRequest ?? "").trim();
  const spriteRequest = (d.spriteRequest ?? "").trim();
  const minimiRequest = (d.minimiRequest ?? "").trim();
  const keyboardSound = (d.keyboardSound ?? "").trim();
  const voiceId = (d.voiceId ?? "").trim();
  // 목록에 없는 자유 입력(커스텀 종족)도 그대로 살린다 — 공백만 다듬는다.
  const typed = (d.archetype ?? "").trim();
  const archetype = typed && typed !== "auto" ? typed : pickArchetype(d.seed);
  const bot = buildBotConfig(d);
  const colors = normalizeColors(d.colors);
  return {
    id: nanoid(),
    name: d.name.trim() || pick(NAME_WORDS),
    role: d.role.trim(),
    seed: d.seed,
    createdAt: Date.now(),
    deskIndex,
    archetype,
    ...(colors ? { colors } : {}),
    ...(cwd ? { cwd } : {}),
    ...(shell ? { shell } : {}),
    ...(startupCommand ? { startupCommand } : {}),
    ...(personalityPrompt ? { personalityPrompt } : {}),
    ...(portraitRequest ? { portraitRequest } : {}),
    ...(spriteRequest ? { spriteRequest } : {}),
    ...(minimiRequest ? { minimiRequest } : {}),
    ...(keyboardSound ? { keyboardSound } : {}),
    ...(voiceId ? { voiceId } : {}),
    ...(bot ? { bot } : {}),
  };
}
