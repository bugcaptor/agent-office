import { expect, it } from "vitest";
import { workItemTitle } from "../workItemDraft";

it("명시 제목을 우선하고 자동 제목은 공백을 합쳐 Unicode 40자로 자른다", () => {
  expect(workItemTitle({ title: " Explicit ", body: "Body" })).toBe("Explicit");
  expect(workItemTitle({ title: " ", body: "  First\n\nsecond\tthird  " })).toBe("First second third");
  expect(workItemTitle({ title: "", body: "🙂".repeat(40) })).toBe("🙂".repeat(40));
  expect(workItemTitle({ title: "", body: "🙂".repeat(41) })).toBe("🙂".repeat(40) + "…");
});
