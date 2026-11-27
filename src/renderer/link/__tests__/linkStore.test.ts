// src/renderer/link/__tests__/linkStore.test.ts
//
// 머신 연동 표시기 판정과 이유 코드 → 번역 키 나누기.
import { describe, expect, it } from "vitest";
import type { LinkStatus } from "@shared/types";
import { linkErrorKey, linkIndicator } from "../linkStore";

const base: LinkStatus = { config: { mode: "off", hostAddress: "" }, protocol: 1, appVersion: "0.7.2", machineId: "m" };
const device = (id: string, connected: boolean) => ({ id, name: id, machineId: id, pairedAt: 0, connected });

describe("linkIndicator", () => {
  it("호스트는 붙은 기기가 있을 때만 수를 보인다", () => {
    expect(linkIndicator({ ...base, host: { devices: [device("a", false)] } })).toBeNull();
    expect(linkIndicator({ ...base, host: { devices: [device("a", true), device("b", true), device("c", false)] } })).toEqual({ kind: "host", count: 2 });
  });

  it("클라이언트는 페어링하고 주소가 있으면 연결 여부를 보인다", () => {
    const client = { state: "waiting" as const, paired: true, address: "https://192.168.0.5:7788", pending: 0 };
    expect(linkIndicator({ ...base, client })).toEqual({ kind: "client", connected: false, name: "https://192.168.0.5:7788" });
    expect(linkIndicator({ ...base, client: { ...client, state: "connected", hostName: "host-mac" } })).toEqual({ kind: "client", connected: true, name: "host-mac" });
    expect(linkIndicator({ ...base, client: { ...client, paired: false } })).toBeNull();
    expect(linkIndicator(null)).toBeNull();
  });
});

describe("linkErrorKey", () => {
  it("설명이 붙은 코드와 HTTP 상태 코드를 나눈다", () => {
    expect(linkErrorKey("link-unreachable: connection refused (os error 61)")).toEqual({ key: "link-unreachable", detail: "connection refused (os error 61)" });
    expect(linkErrorKey("link-http-502")).toEqual({ key: "link-http", detail: "502" });
    expect(linkErrorKey("link-unauthorized")).toEqual({ key: "link-unauthorized", detail: "" });
  });
});
