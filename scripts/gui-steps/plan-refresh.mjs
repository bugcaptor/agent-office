// External file updates must reach the mounted task board through its normal poll.
// npm run gui:shot -- --scenario workpanels --script scripts/gui-steps/plan-refresh.mjs
import assert from "node:assert/strict";
export default async ({ page, shot, harness }) => {
  const task = (id, title, extra = {}) => ({ id, title, body: "", status: "todo", order: 0,
    assigneeAgentId: "p1", sessionIds: [], createdAt: 1, updatedAt: 1, ...extra });
  const root = task("refresh-root", "External plan", { plan: true });
  await harness.call("replaceWorkItems", [root]);
  await page.locator("#work-item-refresh-root").waitFor();
  assert.equal(await page.locator(".work-plan-members").count(), 0);
  await shot("plan-refresh-root-only");

  const first = task("refresh-first", "First external step", { order: 1 });
  const second = task("refresh-second", "Second external step", { order: 2, dependsOn: [first.id] });
  const linked = { ...root, body: "Updated plan memo", dependsOn: [second.id] };
  await harness.call("replaceWorkItems", [linked, first, second]);
  await page.locator(".work-plan-members #work-item-refresh-second").waitFor();
  assert.equal(await page.locator(".work-plan-members .work-card").count(), 2);
  await shot("plan-refresh-live-steps");

  // A second external update changes the graph while a local editor draft is open.
  await page.locator("#work-item-refresh-root .work-card-heading").click();
  await page.getByLabel("제목", { exact: true }).fill("Local draft");
  const third = task("refresh-third", "Third external step", { order: 3, dependsOn: [second.id] });
  await harness.call("replaceWorkItems", [{ ...linked, body: "Latest external memo", dependsOn: [third.id] }, first, second, third]);
  await page.locator(".work-plan-members #work-item-refresh-third").waitFor();
  assert.equal(await page.getByLabel("제목", { exact: true }).inputValue(), "Local draft");
  assert.equal(await page.locator(".work-editor textarea").inputValue(), "Latest external memo");
  await page.getByRole("button", { name: "저장", exact: true }).click();
  await page.getByRole("dialog").getByRole("button", { name: /닫기|Close/ }).click();
  assert.equal(await page.locator(".work-plan-members .work-card").count(), 3);
  await shot("plan-refresh-after-edit");

  const toggle = page.locator(".work-plan-toggle");
  await toggle.click();
  assert.equal(await page.locator(".work-plan-members").count(), 0);
  await toggle.click();
  assert.equal(await page.locator(".work-plan-members .work-card").count(), 3);
};
