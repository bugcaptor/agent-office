// 머신 연동 호스트: 오피스 위 층 단추(이 머신 · 붙은 머신 · 끊긴 머신), 다른 층(원격 캐릭터), 끊긴 층(흐림),
// 원격 캐릭터 대화 보기(보기만).
//   npm run gui:shot -- --scenario machines --script scripts/gui-steps/machines.mjs
export default async ({ page, shot }) => {
  await page.waitForTimeout(600);
  await shot("machines-local");
  await page.getByRole("tab", { name: /studio-pc/ }).click();
  await page.waitForTimeout(1200);
  await shot("machines-remote");
  await page.getByRole("tab", { name: /laptop/ }).click();
  await page.waitForTimeout(800);
  await shot("machines-offline");
  await page.getByRole("tab", { name: /studio-pc/ }).click();
  await page.evaluate(async () => {
    const m = await import("/src/renderer/link/remoteFloors.ts");
    m.openRemoteChat("d-1a2b3c4d:agent-a");
  });
  await page.waitForTimeout(800);
  await shot("machines-chat");
  await page.keyboard.press("Escape");
  await page.waitForTimeout(200);
  await page.getByRole("tab", { name: /이 머신|This machine/ }).click();
  await page.waitForTimeout(500);
  await shot("machines-back");
};
