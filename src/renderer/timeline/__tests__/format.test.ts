import { describe, expect, it } from "vitest";
import { formatDuration } from "../format";

describe("formatDuration", () => {
  it("formats under one hour as 'Nm SSs' with zero-padded seconds", () => {
    expect(formatDuration(0)).toBe("0m 00s");
    expect(formatDuration(2_000)).toBe("0m 02s");
    expect(formatDuration(62_000)).toBe("1m 02s");
    expect(formatDuration(12 * 60_000 + 34_000)).toBe("12m 34s");
    expect(formatDuration(15 * 60_000 + 2_000)).toBe("15m 02s");
  });

  it("formats one hour or more as 'Hh MMm' with zero-padded minutes", () => {
    expect(formatDuration(3_600_000)).toBe("1h 00m");
    expect(formatDuration(3_600_000 + 5 * 60_000)).toBe("1h 05m");
    expect(formatDuration(2 * 3_600_000 + 30 * 60_000)).toBe("2h 30m");
  });

  it("clamps negative/NaN to zero", () => {
    expect(formatDuration(-100)).toBe("0m 00s");
    expect(formatDuration(Number.NaN)).toBe("0m 00s");
  });
});
