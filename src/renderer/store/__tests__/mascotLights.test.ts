// computeMascotLights(docs/mascot-lights-design.md §3) — 신호등 칸 집계.
import { describe, expect, it } from "vitest";
import { computeMascotLights, type ComputeMascotLightsInput } from "../mascotLights";
import type { TurnPhase } from "../../timeline/turnReducer";

const turn = (
  phase: TurnPhase,
  opts: { turnStartedAt?: number | null; waitingSince?: number | null } = {},
) => ({
  phase,
  turnStartedAt: opts.turnStartedAt ?? null,
  waitingSince: opts.waitingSince ?? null,
});

const base = (patch: Partial<ComputeMascotLightsInput> = {}): ComputeMascotLightsInput => ({
  mode: "agents",
  labelMode: "auto",
  projects: [],
  agentOrder: [],
  agents: {},
  timeTracking: {},
  notifications: [],
  taskLabels: {},
  ...patch,
});

describe("computeMascotLights", () => {
  it("mode가 off면 항상 빈 배열", () => {
    expect(
      computeMascotLights(
        base({
          mode: "off",
          agentOrder: ["a"],
          agents: { a: { name: "철수" } },
          timeTracking: { a: turn("working") },
        }),
      ),
    ).toEqual([]);
  });

  describe("agents 모드", () => {
    it("원자 상태가 off가 아닌 에이전트만, agentOrder 순서로 칸을 받는다", () => {
      const lights = computeMascotLights(
        base({
          agentOrder: ["a", "b", "c"],
          agents: { a: { name: "A" }, b: { name: "B" }, c: { name: "C" } },
          timeTracking: {
            a: turn("working", { turnStartedAt: 1 }),
            b: turn("idle"),
            c: turn("waiting", { waitingSince: 2 }),
          },
        }),
      );
      expect(lights.map((l) => l.id)).toEqual(["a", "c"]);
      expect(lights[0]).toEqual({
        id: "a",
        label: "A",
        tooltip: "A",
        state: "working",
        clickAgentId: "a",
        avatar: {
          agentId: "a",
          seed: "a",
          archetype: null,
          colors: null,
          spriteUpdatedAt: null,
          portraitUpdatedAt: null,
        },
      });
      expect(lights[1]).toEqual({
        id: "c",
        label: "C",
        tooltip: "C",
        state: "attention",
        clickAgentId: "c",
        avatar: {
          agentId: "c",
          seed: "c",
          archetype: null,
          colors: null,
          spriteUpdatedAt: null,
          portraitUpdatedAt: null,
        },
      });
    });

    it("전원 idle이면 빈 배열(결정 2 — 일이 없으면 칸이 사라진다)", () => {
      const lights = computeMascotLights(
        base({
          agentOrder: ["a", "b"],
          agents: { a: { name: "A" }, b: { name: "B" } },
          timeTracking: { a: turn("idle"), b: turn("idle") },
        }),
      );
      expect(lights).toEqual([]);
    });

    it("pending만 있고 phase는 idle인 에이전트도 attention(노란불)이다", () => {
      const lights = computeMascotLights(
        base({
          agentOrder: ["a"],
          agents: { a: { name: "A" } },
          timeTracking: { a: turn("idle") },
          notifications: [{ agentId: "a" }],
        }),
      );
      expect(lights).toEqual([
        {
          id: "a",
          label: "A",
          tooltip: "A",
          state: "attention",
          clickAgentId: "a",
          avatar: {
            agentId: "a",
            seed: "a",
            archetype: null,
            colors: null,
            spriteUpdatedAt: null,
            portraitUpdatedAt: null,
          },
        },
      ]);
    });

    it("waiting은 attention으로 매핑되고, 알림을 지운 뒤에도(=pending 없이) 남는다", () => {
      const lights = computeMascotLights(
        base({
          agentOrder: ["a"],
          agents: { a: { name: "A" } },
          timeTracking: { a: turn("waiting", { waitingSince: 10 }) },
          notifications: [],
        }),
      );
      expect(lights).toEqual([
        {
          id: "a",
          label: "A",
          tooltip: "A",
          state: "attention",
          clickAgentId: "a",
          avatar: {
            agentId: "a",
            seed: "a",
            archetype: null,
            colors: null,
            spriteUpdatedAt: null,
            portraitUpdatedAt: null,
          },
        },
      ]);
    });

    it("clockedOut 에이전트는 제외된다", () => {
      const lights = computeMascotLights(
        base({
          agentOrder: ["a"],
          agents: { a: { name: "A", clockedOut: true } },
          timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
        }),
      );
      expect(lights).toEqual([]);
    });

    it("이름이 없으면 라벨은 id로 폴백한다", () => {
      const lights = computeMascotLights(
        base({
          agentOrder: ["a"],
          agents: { a: {} },
          timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
        }),
      );
      expect(lights[0].label).toBe("a");
    });
  });

  describe("projects 모드", () => {
    it("등록된 폴더 순서를 유지하고, 소속 없는 폴더는 off 칸으로 남는다", () => {
      const lights = computeMascotLights(
        base({
          mode: "projects",
          projects: ["/dev/proj-a", "/dev/idle-repo"],
          agentOrder: ["a"],
          agents: { a: { cwd: "/dev/proj-a" } },
          timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
        }),
      );
      expect(lights).toEqual([
        {
          id: "/dev/proj-a",
          label: "proj-a",
          tooltip: "a · proj-a",
          state: "working",
          clickAgentId: "a",
          avatar: {
            agentId: "a",
            seed: "a",
            archetype: null,
            colors: null,
            spriteUpdatedAt: null,
            portraitUpdatedAt: null,
          },
        },
        {
          id: "/dev/idle-repo",
          label: "idle-repo",
          tooltip: "idle-repo",
          state: "off",
          clickAgentId: null,
          avatar: null,
        },
      ]);
    });

    it("소속 에이전트 상태의 max(attention > working > off)로 집계한다", () => {
      const lights = computeMascotLights(
        base({
          mode: "projects",
          projects: ["/dev/proj"],
          agentOrder: ["a", "b"],
          agents: { a: { cwd: "/dev/proj" }, b: { cwd: "/dev/proj" } },
          timeTracking: {
            a: turn("working", { turnStartedAt: 1 }),
            b: turn("waiting", { waitingSince: 5 }),
          },
        }),
      );
      expect(lights[0].state).toBe("attention");
    });

    it("`~` 프로필 cwd도 소속 판정에 쓰인다(isInsideCwd 재사용)", () => {
      const lights = computeMascotLights(
        base({
          mode: "projects",
          projects: ["~/dev/proj"],
          agentOrder: ["a"],
          agents: { a: { cwd: "~/dev/proj/sub" } },
          timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
        }),
      );
      expect(lights[0]).toMatchObject({ state: "working", clickAgentId: "a" });
    });

    it("taskLabels의 세션 실효 cwd가 프로필 cwd보다 우선한다(effectiveCwd)", () => {
      const lights = computeMascotLights(
        base({
          mode: "projects",
          projects: ["/dev/other"],
          agentOrder: ["a"],
          agents: { a: { cwd: "/dev/proj" } },
          taskLabels: { a: { sessionId: "s1", cwd: "/dev/other" } },
          timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
        }),
      );
      expect(lights[0].state).toBe("working");
    });

    it("clockedOut 에이전트는 소속·집계·대표 전부에서 제외된다", () => {
      const lights = computeMascotLights(
        base({
          mode: "projects",
          projects: ["/dev/proj"],
          agentOrder: ["a"],
          agents: { a: { cwd: "/dev/proj", clockedOut: true } },
          timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
        }),
      );
      expect(lights[0]).toEqual({
        id: "/dev/proj",
        label: "proj",
        tooltip: "proj",
        state: "off",
        clickAgentId: null,
        avatar: null,
      });
    });

    it("빈 문자열(공백만)과 중복 경로는 칸 목록에서 제외된다(E1)", () => {
      const lights = computeMascotLights(
        base({
          mode: "projects",
          projects: ["/dev/proj-a", "", "   ", "/dev/proj-a", "/dev/proj-b"],
          agentOrder: ["a"],
          agents: { a: { cwd: "/dev/proj-a" } },
          timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
        }),
      );
      expect(lights.map((l) => l.id)).toEqual(["/dev/proj-a", "/dev/proj-b"]);
    });

    describe("대표(clickAgentId) 선정 우선순위", () => {
      const proj = "/dev/proj";
      const agents = { a: { cwd: proj }, b: { cwd: proj }, c: { cwd: proj } };

      it("① pending 최신(알림 newest-first)이 최우선", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            projects: [proj],
            agentOrder: ["a", "b", "c"],
            agents,
            timeTracking: {
              a: turn("working", { turnStartedAt: 100 }),
              b: turn("waiting", { waitingSince: 50 }),
              c: turn("idle"),
            },
            notifications: [{ agentId: "c" }, { agentId: "a" }],
          }),
        );
        expect(lights[0].clickAgentId).toBe("c");
      });

      it("② pending이 없으면 waiting 중 waitingSince 최신", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            projects: [proj],
            agentOrder: ["a", "b"],
            agents,
            timeTracking: {
              a: turn("waiting", { waitingSince: 10 }),
              b: turn("waiting", { waitingSince: 20 }),
            },
          }),
        );
        expect(lights[0].clickAgentId).toBe("b");
      });

      it("③ waiting이 없으면 working 중 turnStartedAt 최신", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            projects: [proj],
            agentOrder: ["a", "b"],
            agents,
            timeTracking: {
              a: turn("working", { turnStartedAt: 10 }),
              b: turn("working", { turnStartedAt: 99 }),
            },
          }),
        );
        expect(lights[0].clickAgentId).toBe("b");
      });

      it("④ 전부 off면 소속 중 agentOrder 첫째", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            projects: [proj],
            agentOrder: ["a", "b"],
            agents,
            timeTracking: { a: turn("idle"), b: turn("idle") },
          }),
        );
        expect(lights[0].clickAgentId).toBe("a");
      });

      it("칸의 얼굴(avatar)은 대표 에이전트의 스프라이트 좌표를 그대로 나른다", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            projects: [proj],
            agentOrder: ["a"],
            agents: {
              a: {
                cwd: proj,
                seed: "s-a",
                archetype: "cat",
                colors: { hair: "#ff0000" },
                spriteUpdatedAt: 99,
              },
            },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].avatar).toEqual({
          agentId: "a",
          seed: "s-a",
          archetype: "cat",
          colors: { hair: "#ff0000" },
          spriteUpdatedAt: 99,
          portraitUpdatedAt: null,
        });
      });

      it("칸의 얼굴(avatar)은 대표 에이전트의 portraitUpdatedAt도 나른다(§6 개정)", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            projects: [proj],
            agentOrder: ["a"],
            agents: { a: { cwd: proj, portraitUpdatedAt: 42 } },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].avatar).toMatchObject({ portraitUpdatedAt: 42 });
      });

      it("seed가 없는 프로필은 agentId를 시드로 폴백한다(절차 생성 규약)", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            projects: [proj],
            agentOrder: ["a"],
            agents: { a: { cwd: proj } },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].avatar).toMatchObject({ agentId: "a", seed: "a" });
      });

      it("소속 에이전트가 없으면 clickAgentId는 null(클릭 no-op)", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            projects: [proj],
            agentOrder: [],
            agents: {},
            timeTracking: {},
          }),
        );
        expect(lights[0].clickAgentId).toBeNull();
      });

      it("소속이 아닌 에이전트의 최신 알림은 대표 선정에서 건너뛴다(memberSet 검사, T4)", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            projects: ["/dev/proj-a", "/dev/proj-b"],
            agentOrder: ["a", "x"],
            agents: { a: { cwd: "/dev/proj-a" }, x: { cwd: "/dev/proj-b" } },
            timeTracking: { a: turn("waiting", { waitingSince: 10 }), x: turn("idle") },
            // x(proj-b 소속)의 알림이 더 최신이지만 proj-a의 대표가 되면 안 된다.
            notifications: [{ agentId: "x" }],
          }),
        );
        const projA = lights.find((l) => l.id === "/dev/proj-a");
        expect(projA?.clickAgentId).toBe("a");
      });
    });

    it("agentOrder에는 있지만 agents에 없는 유령 id는 소속 계산에서 안전히 건너뛴다(T4)", () => {
      const lights = computeMascotLights(
        base({
          mode: "projects",
          projects: ["/dev/proj"],
          agentOrder: ["ghost", "a"],
          agents: { a: { cwd: "/dev/proj" } },
          timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
        }),
      );
      expect(lights[0].state).toBe("working");
      expect(lights[0].clickAgentId).toBe("a");
    });
  });

  it("agents 모드: agentOrder에는 있지만 agents에 없는 유령 id는 건너뛴다(T4)", () => {
    const lights = computeMascotLights(
      base({
        agentOrder: ["ghost", "a"],
        agents: { a: { name: "A" } },
        timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
      }),
    );
    expect(lights.map((l) => l.id)).toEqual(["a"]);
  });

  // §7 개정 — mascotLightsLabel(칸에 표시할 텍스트) 선택과 tooltip 조립.
  describe("labelMode(§7 개정)", () => {
    const fullAgent = {
      name: "철수",
      cwd: "/dev/proj-a",
    };
    const fullTaskLabel = { sessionId: "s1", goal: "테스트 작성" };

    describe("agents 모드", () => {
      it("auto/agent는 둘 다 에이전트 이름을 쓴다", () => {
        for (const labelMode of ["auto", "agent"] as const) {
          const lights = computeMascotLights(
            base({
              labelMode,
              agentOrder: ["a"],
              agents: { a: fullAgent },
              taskLabels: { a: fullTaskLabel },
              timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
            }),
          );
          expect(lights[0].label).toBe("철수");
        }
      });

      it("project는 프로젝트명(정체성 앵커 basename)을 쓴다", () => {
        const lights = computeMascotLights(
          base({
            labelMode: "project",
            agentOrder: ["a"],
            agents: { a: fullAgent },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].label).toBe("proj-a");
      });

      it("project인데 cwd가 전혀 없으면(값이 비면) auto(이름)로 폴백한다", () => {
        const lights = computeMascotLights(
          base({
            labelMode: "project",
            agentOrder: ["a"],
            agents: { a: { name: "철수" } },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].label).toBe("철수");
      });

      it("task는 목표(goal)를 60자로 절단해 쓴다", () => {
        const lights = computeMascotLights(
          base({
            labelMode: "task",
            agentOrder: ["a"],
            agents: { a: fullAgent },
            taskLabels: { a: fullTaskLabel },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].label).toBe("테스트 작성");
      });

      it("task인데 작업 정보가 전혀 없으면(값이 비면) auto(이름)로 폴백한다", () => {
        const lights = computeMascotLights(
          base({
            labelMode: "task",
            agentOrder: ["a"],
            agents: { a: { name: "철수" } },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].label).toBe("철수");
      });

      it("task는 60자를 넘으면 절단하고 …을 붙인다", () => {
        const longGoal = "가".repeat(80);
        const lights = computeMascotLights(
          base({
            labelMode: "task",
            agentOrder: ["a"],
            agents: { a: fullAgent },
            taskLabels: { a: { sessionId: "s1", goal: longGoal } },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].label).toBe("가".repeat(60) + "…");
      });

      it("tooltip은 labelMode와 무관하게 [이름, 프로젝트명, 작업명]을 이어 붙인다", () => {
        const lights = computeMascotLights(
          base({
            labelMode: "auto",
            agentOrder: ["a"],
            agents: { a: fullAgent },
            taskLabels: { a: fullTaskLabel },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].tooltip).toBe("철수 · proj-a · 테스트 작성");
      });
    });

    describe("projects 모드", () => {
      const proj = "/dev/proj";

      it("auto/project는 둘 다 폴더 basename을 쓴다(현행 그대로)", () => {
        for (const labelMode of ["auto", "project"] as const) {
          const lights = computeMascotLights(
            base({
              mode: "projects",
              labelMode,
              projects: [proj],
              agentOrder: ["a"],
              agents: { a: { name: "철수", cwd: proj } },
              taskLabels: { a: { sessionId: "s1", goal: "테스트 작성" } },
              timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
            }),
          );
          expect(lights[0].label).toBe("proj");
        }
      });

      it("agent는 대표 에이전트 이름을 쓴다", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            labelMode: "agent",
            projects: [proj],
            agentOrder: ["a"],
            agents: { a: { name: "철수", cwd: proj } },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].label).toBe("철수");
      });

      it("agent인데 대표가 없으면(소속 없음) auto(폴더명)로 폴백한다", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            labelMode: "agent",
            projects: [proj],
            agentOrder: [],
            agents: {},
            timeTracking: {},
          }),
        );
        expect(lights[0].label).toBe("proj");
      });

      it("task는 대표 에이전트의 작업명을 쓴다", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            labelMode: "task",
            projects: [proj],
            agentOrder: ["a"],
            agents: { a: { name: "철수", cwd: proj } },
            taskLabels: { a: { sessionId: "s1", goal: "테스트 작성" } },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].label).toBe("테스트 작성");
      });

      it("task인데 대표는 있지만 작업 정보가 없으면 auto(폴더명)로 폴백한다", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            labelMode: "task",
            projects: [proj],
            agentOrder: ["a"],
            agents: { a: { name: "철수", cwd: proj } },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].label).toBe("proj");
      });

      it("tooltip은 [대표 이름, 폴더명, 대표 작업명] 순으로 이어 붙인다", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            labelMode: "auto",
            projects: [proj],
            agentOrder: ["a"],
            agents: { a: { name: "철수", cwd: proj } },
            taskLabels: { a: { sessionId: "s1", goal: "테스트 작성" } },
            timeTracking: { a: turn("working", { turnStartedAt: 1 }) },
          }),
        );
        expect(lights[0].tooltip).toBe("철수 · proj · 테스트 작성");
      });

      it("대표가 없는 칸의 tooltip은 폴더명뿐이다", () => {
        const lights = computeMascotLights(
          base({
            mode: "projects",
            labelMode: "auto",
            projects: [proj],
            agentOrder: [],
            agents: {},
            timeTracking: {},
          }),
        );
        expect(lights[0].tooltip).toBe("proj");
      });
    });
  });
});
