// Settings "Control" tab web dashboard section, plus the "web viewers" indicator on the office.
//   npm run gui:shot -- --scenario working --script scripts/gui-steps/web-dash.mjs
export default async ({ page, shot }) => {
  await shot("web-dash-indicator");
  await page.locator(".settings-btn").click();
  await page.getByRole("tab", { name: /제어|Control/ }).click();
  await shot("web-dash-settings");
};
