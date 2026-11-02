// Auto-run usage wait: a plan paused until the Claude 5-hour window resets, and an idle plan's forecast line.
//   npm run gui:shot -- --scenario workpanels --script scripts/gui-steps/auto-run-usage.mjs
export default async ({ page, shot }) => {
  await shot("auto-run-usage-list");
  await page.getByText("Release 2.0").first().click();
  await page.waitForTimeout(200);
  await page.getByRole("dialog").getByRole("button", { name: /기다리지 않고 계속|Continue without waiting/ }).scrollIntoViewIfNeeded();
  await shot("auto-run-usage-wait");
  await page.getByRole("dialog").getByRole("button", { name: /닫기|Close/ }).first().click();
  await page.getByText("Clean up logs").first().click();
  await page.waitForTimeout(200);
  await shot("auto-run-usage-idle");
};
