import { beforeEach, describe, expect, it } from "vitest";
import fixture from "@shared/contract-fixtures/link-remote.json";
import type { RemoteOffice } from "@shared/types";
import { resetRemoteImages, syncRemoteImages, type RemoteImageIo } from "../remoteImages";
import { remoteProfile, useRemoteFloors } from "../remoteFloors";

const office = fixture as RemoteOffice;

function fakeIo() {
  const calls: string[] = [];
  const io: RemoteImageIo = {
    load: async (kind, ids) => void calls.push(`load ${kind} ${ids.join(",")}`),
    clear: (kind, id) => void calls.push(`clear ${kind} ${id}`),
  };
  return { io, calls };
}

describe("remoteImages", () => {
  beforeEach(() => {
    resetRemoteImages();
    useRemoteFloors.setState({ imagesRev: 0 });
  });

  it("loads the custom sheet and portrait of online characters once per version", async () => {
    const { io, calls } = fakeIo();
    await syncRemoteImages(office, io);
    expect(calls.sort()).toEqual(["load portrait d-1a2b3c4d:agent-a", "load sprite d-1a2b3c4d:agent-a"]);
    expect(useRemoteFloors.getState().imagesRev).toBe(1);
    calls.length = 0;
    await syncRemoteImages(office, io);
    expect(calls).toEqual([]);
    const o: RemoteOffice = structuredClone(office);
    o.machines[1].characters[0].appearance!.spriteUpdatedAt = 1700000000009;
    await syncRemoteImages(o, io);
    expect(calls).toEqual(["load sprite d-1a2b3c4d:agent-a"]);
  });

  it("drops images whose version went away and skips offline machines", async () => {
    const { io, calls } = fakeIo();
    await syncRemoteImages(office, io);
    calls.length = 0;
    const o: RemoteOffice = structuredClone(office);
    delete o.machines[1].characters[0].portrait;
    o.machines[0].characters[0].appearance = { seed: "c", spriteUpdatedAt: 5 };
    await syncRemoteImages(o, io);
    expect(calls).toEqual(["clear portrait d-1a2b3c4d:agent-a"]);
  });

  it("profiles carry the remote image versions", () => {
    expect(remoteProfile(office.machines[1].characters[0])).toMatchObject({ spriteUpdatedAt: 1700000000001, portraitUpdatedAt: 1700000000002 });
    expect(remoteProfile(office.machines[1].characters[1])).not.toHaveProperty("spriteUpdatedAt");
  });
});
