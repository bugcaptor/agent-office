// Plan roots: a memo-only plan root (no "root has work" tag, root not counted) and a root that runs last.
//   npm run gui:shot -- --scenario workpanels --script scripts/gui-steps/plan-root.mjs
export default async ({ page, shot }) => {
  await shot("plan-root-list");
  await page.getByText("Clean up logs").first().click();
  await page.waitForTimeout(200);
  await shot("plan-root-memo");
  await page.getByRole("dialog").getByRole("button", { name: /닫기|Close/ }).click();
  await page.getByText("Ship search").first().click();
  await page.waitForTimeout(200);
  await shot("plan-root-runs-last");
};
