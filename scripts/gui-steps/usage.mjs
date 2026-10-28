// Usage badge (Claude/Codex/OpenRouter) in the bottom bar and the detail dialog.
//   npm run gui:shot -- --script scripts/gui-steps/usage.mjs [--size 1280x800]
export default async ({ page, shot }) => {
  const widget = page.locator(".usage-widget");
  console.log(`  badge: ${(await widget.textContent())?.replace(/\s+/g, " ")}`);
  console.log(`  OR title: ${await widget.locator(".usage-badge-or").getAttribute("title")}`);
  console.log(`  OR visible: ${await widget.locator(".usage-badge-or").isVisible()}`);
  await shot("usage-bar");
  await widget.click();
  await page.waitForTimeout(300);
  const block = page.locator(".usage-provider-block", { hasText: "OpenRouter" });
  console.log(`  dialog OR: ${(await block.textContent())?.replace(/\s+/g, " ")}`);
  await shot("usage-dialog");
  await page.locator(".usage-body").evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await shot("usage-dialog-openrouter");
};
