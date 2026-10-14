// Example --script for `npm run gui:shot`: open a running agent's terminal and type.
//   npm run gui:shot -- --scenario working --script scripts/gui-steps/terminal.mjs
export default async ({ page, shot, harness }) => {
  await shot("office");
  await harness.store((s, id) => s.openTerminal(id), "p1");
  await page.waitForTimeout(400);
  await page.keyboard.type("echo hi\r");
  await harness.call("pushOutput", "p1", "hi\r\n");
  await page.waitForTimeout(200);
  await shot("terminal");
};
