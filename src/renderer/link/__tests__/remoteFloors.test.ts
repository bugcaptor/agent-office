import { beforeEach, describe, expect, it } from "vitest";
import fixture from "@shared/contract-fixtures/link-remote.json";
import type { RemoteOffice } from "@shared/types";
import {
  findRemote,
  machineSummary,
  openRemoteChat,
  remoteFloorView,
  remoteProfile,
  selectFloor,
  setRemoteOffice,
  useRemoteFloors,
} from "../remoteFloors";

const office = fixture as RemoteOffice;

describe("remoteFloors", () => {
  beforeEach(() => useRemoteFloors.setState({ office: { machines: [] }, floor: null, chat: null, hover: null }));

  it("this machine's floor has no remote view; another floor maps characters to scene agents", () => {
    expect(remoteFloorView(office, null)).toBeNull();
    expect(remoteFloorView(office, "d-unknown")).toBeNull();
    const view = remoteFloorView(office, "d-1a2b3c4d")!;
    expect(view.key).toBe("d-1a2b3c4d");
    expect(view.agents.map((a) => [a.profile.id, a.active, a.pending, a.online])).toEqual([
      ["d-1a2b3c4d:agent-a", true, true, true],
      ["d-1a2b3c4d:agent-b", false, false, true],
    ]);
    const offline = remoteFloorView(office, "d-5e6f7a8b")!;
    expect(offline.agents[0].online).toBe(false);
  });

  it("clocked-out remote characters do not stand on the floor", () => {
    const o: RemoteOffice = structuredClone(office);
    o.machines[1].characters[1].clockedOut = true;
    expect(remoteFloorView(o, "d-1a2b3c4d")!.agents.map((a) => a.profile.id)).toEqual(["d-1a2b3c4d:agent-a"]);
  });

  it("profiles use the remote appearance and fall back to the id as seed", () => {
    const [a, b] = office.machines[1].characters;
    expect(remoteProfile(a)).toMatchObject({ id: a.id, seed: "ada-seed", archetype: "human", colors: { hair: "#aa3311" } });
    expect(remoteProfile(office.machines[0].characters[0]).seed).toBe("d-5e6f7a8b:agent-c");
    expect(remoteProfile(b)).not.toHaveProperty("archetype");
  });

  it("summaries count working and waiting characters", () => {
    expect(machineSummary(office.machines[1])).toEqual({ working: 0, waiting: 1 });
    expect(findRemote(office, "d-1a2b3c4d:agent-b")?.machine.name).toBe("studio-pc");
    expect(findRemote(office, "agent-b")).toBeNull();
  });

  it("a revoked machine sends the view back to this floor and closes its chat", () => {
    setRemoteOffice(office);
    selectFloor("d-1a2b3c4d");
    openRemoteChat("d-1a2b3c4d:agent-a");
    setRemoteOffice({ machines: office.machines.filter((m) => m.deviceId !== "d-1a2b3c4d") });
    expect(useRemoteFloors.getState().floor).toBeNull();
    expect(useRemoteFloors.getState().chat).toBeNull();
  });
});
