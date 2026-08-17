import * as path from "node:path";
import { describe, expect, it } from "vitest";

import { IDENTIFIER, resolveAppData, type AppDataInput } from "./appData";

const base: AppDataInput = { platform: "darwin", home: "/Users/me" };

describe("resolveAppData", () => {
  it("설정값이 env와 OS 기본을 모두 이긴다", () => {
    expect(
      resolveAppData({ ...base, configValue: "/from/config", env: "/from/env" }),
    ).toBe("/from/config");
  });

  it("설정값이 비었으면 env를 쓴다", () => {
    expect(resolveAppData({ ...base, configValue: "  ", env: "/from/env" })).toBe("/from/env");
  });

  it("둘 다 없으면 macOS 기본 경로", () => {
    expect(resolveAppData(base)).toBe(
      path.join("/Users/me", "Library", "Application Support", IDENTIFIER),
    );
  });

  it("Linux는 XDG_DATA_HOME을 먼저 쓰고 없으면 ~/.local/share", () => {
    expect(
      resolveAppData({ platform: "linux", home: "/home/me", xdgDataHome: "/data" }),
    ).toBe(path.join("/data", IDENTIFIER));
    expect(resolveAppData({ platform: "linux", home: "/home/me", xdgDataHome: "" })).toBe(
      path.join("/home/me", ".local", "share", IDENTIFIER),
    );
  });

  it("Windows는 %APPDATA%를 쓴다", () => {
    expect(resolveAppData({ platform: "win32", appData: "C:\\Users\\me\\AppData\\Roaming" })).toBe(
      path.join("C:\\Users\\me\\AppData\\Roaming", IDENTIFIER),
    );
  });

  it("경로를 만들 재료가 없거나 모르는 플랫폼이면 undefined", () => {
    expect(resolveAppData({ platform: "darwin" })).toBeUndefined();
    expect(resolveAppData({ platform: "linux" })).toBeUndefined();
    expect(resolveAppData({ platform: "win32" })).toBeUndefined();
    expect(resolveAppData({ platform: "aix", home: "/home/me" })).toBeUndefined();
  });

  it("설정값의 ~는 홈으로 펼친다", () => {
    expect(resolveAppData({ ...base, configValue: "~/office" })).toBe(
      path.join("/Users/me", "office"),
    );
    expect(resolveAppData({ ...base, configValue: "~" })).toBe("/Users/me");
    // 중간의 ~는 건드리지 않는다.
    expect(resolveAppData({ ...base, configValue: "/opt/~x" })).toBe("/opt/~x");
  });
});
