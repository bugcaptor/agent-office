// Markdown editor "open in browser" button.
//   npm run gui:shot -- --scenario office --script scripts/gui-steps/doc-view.mjs
export default async ({ page, shot }) => {
  await page.evaluate(async () => {
    const store = await window.__guiHarness.markdownStore();
    await store.getState().openFile("/work/agent-office", "docs/plan.md", "p1");
  });
  await page.waitForTimeout(300);
  const button = page.getByRole("button", { name: /브라우저|browser/i });
  console.log(`  title(clean): ${await button.getAttribute("title")}`);
  await shot("doc-view-preview");
  await button.click();
  console.log(`  docViewOpen: ${JSON.stringify(await page.evaluate(() => window.__guiHarness.docViewOpened))}`);
  await page.getByRole("button", { name: /소스|Source/ }).click();
  await page.locator("textarea").first().fill("# edited\n");
  console.log(`  title(dirty): ${await button.getAttribute("title")}`);
  await shot("doc-view-dirty");
};
