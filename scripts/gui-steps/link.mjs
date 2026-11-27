// 머신 연동: 설정 › 머신 연동 탭. 호스트(코드·기기 목록, 가짜 API 기본)와 클라이언트(이벤트로 바꿔 끼움).
//   npm run gui:shot -- --scenario office --script scripts/gui-steps/link.mjs
export default async ({ page, shot }) => {
  await page.evaluate(async () => (await window.__guiHarness.store()).getState().openModal({ kind: "settings" }));
  await page.getByRole("tab", { name: /머신 연동|Machines/ }).click();
  await page.waitForTimeout(300);
  await shot("link-host");
  await page.evaluate(() => {
    window.__guiHarness.emitTauriEvent("link-status", {
      config: { mode: "client", hostAddress: "https://192.168.0.5:7788" },
      protocol: 1,
      appVersion: "0.7.2",
      machineId: "m",
      client: {
        state: "rejected",
        paired: true,
        hostName: "host-mac",
        deviceId: "d-1",
        address: "https://192.168.0.5:7788",
        pinnedFingerprint: "AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89:AB:CD:EF:01:23:45:67:89",
        error: "link-protocol-mismatch",
        hostProtocol: 2,
        hostAppVersion: "0.9.0",
        pending: 3,
      },
    });
  });
  await page.waitForTimeout(200);
  await shot("link-client");
  await page.getByRole("button", { name: /다시 페어링|Pair again/ }).click();
  await page.waitForTimeout(100);
  await shot("link-client-pair");
  await page.evaluate(async () => (await window.__guiHarness.store()).getState().closeModal());
  await page.waitForTimeout(200);
  await shot("link-indicator");
};
