// src/renderer/profile/__tests__/generate.test.ts
//
// TDD for `generate.ts`'s pure draft generator.
//
// `pick()` reads `Math.random()` directly (no injected rng seam). Tests pin
// it down via `vi.spyOn(Math, "random")` rather than statistical sampling,
// so results are exact and non-flaky.
import { afterEach, describe, expect, it, vi } from "vitest";
import { generateDraft, draftToProfile } from "../generate";
import { NAME_WORDS, ROLE_WORDS, PERSONALITY_WORDS } from "../wordlists";
import { pickArchetype } from "../../office/gen/archetypes";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("generateDraft", () => {
  it("picks name/role/personality via Math.random (index 0 when random() returns 0)", () => {
    vi.spyOn(Math, "random").mockReturnValue(0);

    const draft = generateDraft();

    expect(draft.name).toBe(NAME_WORDS[0]);
    expect(draft.role).toBe(ROLE_WORDS[0]);
    expect(draft.note).toBe(`${PERSONALITY_WORDS[0]} 성격`);
  });

  it("picks a different index when random() returns a higher fraction", () => {
    // 1/20 lands just past the index-0 bucket (each word list has 20 entries).
    vi.spyOn(Math, "random").mockReturnValue(1 / 20 + 0.001);

    const draft = generateDraft();

    expect(draft.name).toBe(NAME_WORDS[1]);
    expect(draft.role).toBe(ROLE_WORDS[1]);
    expect(draft.note).toBe(`${PERSONALITY_WORDS[1]} 성격`);
  });

  it("assigns an 8-char nanoid seed", () => {
    const draft = generateDraft();
    expect(draft.seed).toMatch(/^[A-Za-z0-9_-]{8}$/);
  });

  it("initializes cwd as an empty string (Task 3)", () => {
    const draft = generateDraft();
    expect(draft.cwd).toBe("");
  });

  it("initializes shell as an empty string", () => {
    const draft = generateDraft();
    expect(draft.shell).toBe("");
  });

  it("initializes startupCommand as an empty string", () => {
    const draft = generateDraft();
    expect(draft.startupCommand).toBe("");
  });

  it("varies name/role/note/seed across successive calls (real randomness, not memoized)", () => {
    const seen = new Set<string>();
    for (let i = 0; i < 20; i += 1) {
      const d = generateDraft();
      seen.add(`${d.name}|${d.role}|${d.note}|${d.seed}`);
    }
    // 20 draws from 20-way choices is astronomically unlikely to collapse to
    // a single value if Math.random is really being consulted each call.
    expect(seen.size).toBeGreaterThan(1);
  });
});

describe("draftToProfile", () => {
  it("builds an AgentProfile from a draft, trimming name/role/note and passing seed/deskIndex through", () => {
    const before = Date.now();
    const profile = draftToProfile(
      { name: "  Foo  ", role: " Bar ", note: "  hello  ", seed: "seedseed" },
      3
    );
    const after = Date.now();

    expect(profile.name).toBe("Foo");
    expect(profile.role).toBe("Bar");
    expect(profile.note).toBe("hello");
    expect(profile.seed).toBe("seedseed");
    expect(profile.deskIndex).toBe(3);
    expect(profile.createdAt).toBeGreaterThanOrEqual(before);
    expect(profile.createdAt).toBeLessThanOrEqual(after);
  });

  it("assigns a fresh nanoid id, independent of the seed", () => {
    const profile = draftToProfile(
      { name: "Foo", role: "Bar", note: "note", seed: "seedseed" },
      0
    );
    expect(profile.id).toMatch(/^[A-Za-z0-9_-]{21}$/);
    expect(profile.id).not.toBe(profile.seed);
  });

  it("two profiles built from drafts get distinct ids", () => {
    const draft = { name: "Foo", role: "Bar", note: "note", seed: "seedseed" };
    const a = draftToProfile(draft, 0);
    const b = draftToProfile(draft, 1);
    expect(a.id).not.toBe(b.id);
  });

  it("falls back to a random NAME_WORDS entry when the trimmed name is empty", () => {
    vi.spyOn(Math, "random").mockReturnValue(0);
    const profile = draftToProfile({ name: "   ", role: "Bar", note: "note", seed: "s" }, 0);
    expect(profile.name).toBe(NAME_WORDS[0]);
  });
});

describe("draftToProfile cwd handling (Task 3)", () => {
  it("trims surrounding whitespace from cwd", () => {
    const profile = draftToProfile(
      { name: "Foo", role: "Bar", note: "note", seed: "seed", cwd: "  /a/b  " },
      0
    );
    expect(profile.cwd).toBe("/a/b");
  });

  it("omits the cwd field entirely when it is empty after trimming", () => {
    const profile = draftToProfile(
      { name: "Foo", role: "Bar", note: "note", seed: "seed", cwd: "   " },
      0
    );
    expect(profile.cwd).toBeUndefined();
    expect("cwd" in profile).toBe(false);
  });

  it("omits the cwd field entirely when it was never set (undefined draft.cwd)", () => {
    const profile = draftToProfile({ name: "Foo", role: "Bar", note: "note", seed: "seed" }, 0);
    expect(profile.cwd).toBeUndefined();
    expect("cwd" in profile).toBe(false);
  });
});

describe("draftToProfile shell handling", () => {
  it("includes trimmed shell when non-empty", () => {
    const profile = draftToProfile(
      { name: "Foo", role: "Bar", note: "note", seed: "seed", shell: "  pwsh  " },
      0
    );
    expect(profile.shell).toBe("pwsh");
  });

  it("omits the shell field entirely when it is empty after trimming", () => {
    const profile = draftToProfile(
      { name: "Foo", role: "Bar", note: "note", seed: "seed", shell: "   " },
      0
    );
    expect(profile.shell).toBeUndefined();
    expect("shell" in profile).toBe(false);
  });

  it("omits the shell field entirely when it was never set (undefined draft.shell)", () => {
    const profile = draftToProfile({ name: "Foo", role: "Bar", note: "note", seed: "seed" }, 0);
    expect(profile.shell).toBeUndefined();
    expect("shell" in profile).toBe(false);
  });
});

describe("draftToProfile startupCommand", () => {
  it("includes trimmed startupCommand when non-empty", () => {
    const profile = draftToProfile(
      { name: "Foo", role: "Bar", note: "note", seed: "seed", startupCommand: "  source ./init.sh  " },
      0,
    );
    expect(profile.startupCommand).toBe("source ./init.sh");
  });

  it("omits the startupCommand field entirely when it is empty after trimming", () => {
    const profile = draftToProfile(
      { name: "Foo", role: "Bar", note: "note", seed: "seed", startupCommand: "   " },
      0,
    );
    expect(profile.startupCommand).toBeUndefined();
    expect("startupCommand" in profile).toBe(false);
  });

  it("omits the startupCommand field entirely when it was never set", () => {
    const profile = draftToProfile({ name: "Foo", role: "Bar", note: "note", seed: "seed" }, 0);
    expect(profile.startupCommand).toBeUndefined();
    expect("startupCommand" in profile).toBe(false);
  });
});

describe("draftToProfile appearance", () => {
  it("includes trimmed appearance when non-empty", () => {
    const p = draftToProfile(
      { name: "A", role: "r", note: "n", seed: "s", appearance: "  glasses  " },
      0
    );
    expect(p.appearance).toBe("glasses");
  });

  it("omits appearance when blank", () => {
    const p = draftToProfile(
      { name: "A", role: "r", note: "n", seed: "s", appearance: "   " },
      0
    );
    expect("appearance" in p).toBe(false);
  });
});

describe("draftToProfile spriteRequest", () => {
  it("draftToProfile은 spriteRequest를 트리밍해 포함하고, 비면 생략한다", () => {
    const base = { name: "Ada", role: "backend", note: "", seed: "s1" };
    const withReq = draftToProfile({ ...base, spriteRequest: "  red cloak  " }, 0);
    expect(withReq.spriteRequest).toBe("red cloak");
    const without = draftToProfile({ ...base, spriteRequest: "   " }, 0);
    expect(without.spriteRequest).toBeUndefined();
    expect("spriteRequest" in without).toBe(false);
  });

  it("generateDraft는 빈 spriteRequest를 초기화한다", () => {
    expect(generateDraft().spriteRequest).toBe("");
  });
});

describe("draftToProfile archetype", () => {
  it("initializes generateDraft archetype as 'auto'", () => {
    expect(generateDraft().archetype).toBe("auto");
  });

  it("resolves 'auto' (or omitted) to the seed-drawn concrete archetype", () => {
    const seed = "seed-arch";
    const p = draftToProfile({ name: "A", role: "r", note: "n", seed, archetype: "auto" }, 0);
    expect(p.archetype).toBe(pickArchetype(seed));
    const p2 = draftToProfile({ name: "A", role: "r", note: "n", seed }, 0);
    expect(p2.archetype).toBe(pickArchetype(seed));
  });

  it("passes an explicitly chosen archetype through", () => {
    const p = draftToProfile({ name: "A", role: "r", note: "n", seed: "s", archetype: "orc" }, 0);
    expect(p.archetype).toBe("orc");
  });
});
