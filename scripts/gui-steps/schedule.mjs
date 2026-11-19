// Schedules: card chips, a recurring template, a schedule an agent left turned off, and the schedule editor.
//   npm run gui:shot -- --scenario workpanels --script scripts/gui-steps/schedule.mjs
export default async ({ page, shot }) => {
  await shot("schedule-list");
  const close = () => page.getByRole("dialog").getByRole("button", { name: /닫기|접기|Close|Collapse/ }).first().click();
  await page.locator(".work-card.status-todo .work-card-heading", { hasText: "Morning dependency audit" }).click();
  await page.waitForTimeout(200);
  await page.locator(".work-schedule").scrollIntoViewIfNeeded();
  await shot("schedule-template");
  await close();
  await page.locator(".work-card-heading", { hasText: "Clean build caches" }).first().click();
  await page.waitForTimeout(200);
  await page.locator(".work-schedule").scrollIntoViewIfNeeded();
  await shot("schedule-off");
  await page.locator(".work-schedule").getByRole("button", { name: /예약 바꾸기|Change schedule/ }).click();
  await page.locator(".work-schedule select").first().selectOption("weekly");
  await page.waitForTimeout(100);
  await page.locator(".work-schedule-editor").scrollIntoViewIfNeeded();
  await shot("schedule-editor");
};
