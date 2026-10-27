// Task detail popup header: task ID and copy button.
//   npm run gui:shot -- --scenario workpanels --script scripts/gui-steps/work-id-copy.mjs
export default async ({ page, shot }) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.getByText("Ship search").first().click();
  await page.waitForTimeout(200);
  const dialog = page.getByRole("dialog");
  console.log(`  id: ${await dialog.locator(".work-dialog-id code").textContent()}`);
  await shot("work-id");
  await dialog.locator(".work-dialog-copy").click();
  await page.waitForTimeout(100);
  console.log(`  after click: ${await dialog.locator(".work-dialog-copy").getAttribute("title")}`);
  console.log(`  clipboard: ${await page.evaluate(() => navigator.clipboard.readText().catch((e) => `(${e.message})`))}`);
  await shot("work-id-copied");
  await page.waitForTimeout(1600);
  console.log(`  after 1.6s: ${await dialog.locator(".work-dialog-copy").getAttribute("title")}`);
  await dialog.getByRole("button", { name: /닫기|Close/ }).click();
  await page.getByRole("button", { name: /새 태스크 만들기|New task/ }).first().click();
  await page.waitForTimeout(200);
  console.log(`  draft has id: ${await page.getByRole("dialog").locator(".work-dialog-id").count()}`);
  await shot("work-id-draft");
};
