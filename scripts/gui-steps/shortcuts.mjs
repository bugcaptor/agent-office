// Settings "Shortcuts" tab (placeholders, recording, conflicts) and the keys opening terminals.
//   npm run gui:shot -- --scenario working --script scripts/gui-steps/shortcuts.mjs
// Uses mac keys (⌘⌥) when the harness runs on macOS, Ctrl+Alt elsewhere.
export default async ({ page, shot }) => {
  const isMac = await page.evaluate(() => /mac/i.test(navigator.platform));
  const mod = isMac ? ["Alt", "Meta"] : ["Control", "Alt"];
  const press = async (key) => {
    for (const m of mod) await page.keyboard.down(m);
    await page.keyboard.press(key);
    for (const m of [...mod].reverse()) await page.keyboard.up(m);
  };
  const activeTab = () => page.locator(".agent-tab-strip [aria-selected='true'], .agent-tab-strip .active").first().textContent().catch(() => null);

  await page.locator(".settings-btn").click();
  await page.getByRole("tab", { name: /단축키|Shortcuts/ }).click();
  await shot("shortcuts-tab");

  // Ada: take the suggested key. Bo: record a key, then try Ada's key (conflict).
  const rows = page.locator(".settings-shortcut-row");
  await rows.filter({ hasText: "Ada" }).getByRole("button", { name: /추천값|Suggested/ }).click();
  const bo = rows.filter({ hasText: "Bo" }).locator(".settings-shortcut-key");
  await bo.click();
  await press("KeyK");
  await bo.click();
  await press("Digit1");
  console.log(`  conflict: ${await rows.filter({ hasText: "Bo" }).locator("[role=alert]").textContent()}`);
  await shot("shortcuts-recorded");

  await page.getByRole("button", { name: /^닫기$|^Close$/ }).click();
  await press("Digit1");
  await page.waitForTimeout(300);
  console.log(`  after agent key: overlay=${await page.locator(".agent-tab-strip").count()} active=${await activeTab()}`);
  await shot("shortcuts-agent-key");
};
