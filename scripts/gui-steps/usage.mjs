// Usage badges in the bottom bar (the 2 most recently used of CL·CX·OC·OR) and the detail dialog.
//   npm run gui:shot -- --script scripts/gui-steps/usage.mjs [--size 1280x800]
export default async ({ page, shot }) => {
  const widget = page.locator(".usage-widget");
  console.log(`  badges: ${(await widget.textContent())?.replace(/\s+/g, " ")}`);
  for (const b of await widget.locator("> span").all()) console.log(`    ${await b.getAttribute("title")}`);
  await shot("usage-bar");
  await widget.click();
  await page.waitForTimeout(300);
  console.log(`  dialog title: ${await page.locator(".usage-dialog h2").textContent()}`);
  for (const name of ["OpenCode Go", "OpenRouter"]) {
    const block = page.locator(".usage-provider-block", { hasText: name });
    console.log(`  ${name}: ${(await block.textContent())?.replace(/\s+/g, " ")}`);
  }
  await shot("usage-dialog");
  await page.locator(".usage-body").evaluate((el) => el.scrollTo(0, el.scrollHeight));
  await shot("usage-dialog-bottom");
};
