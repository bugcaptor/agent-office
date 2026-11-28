// 머신 간 파일 보내기: 하단 바 📁 대화상자(대상·기록), 원격 캐릭터를 골라 연 대화상자, 설정의 파일 받기.
//   npm run gui:shot -- --scenario files --script scripts/gui-steps/files.mjs
export default async ({ page, shot }) => {
  await page.getByRole("button", { name: /📁/ }).first().click();
  await page.waitForTimeout(400);
  await shot("files-dialog");
  await page.evaluate(async () => {
    const store = (await window.__guiHarness.store()).getState();
    store.closeModal();
    store.openModal({ kind: "files", target: { remoteId: "d-1a2b3c4d:agent-a" } });
  });
  await page.waitForTimeout(400);
  await page.getByRole("button", { name: /파일 고르기|Choose files/ }).click();
  await page.waitForTimeout(200);
  await shot("files-dialog-remote");
  await page.evaluate(() => {
    window.__guiHarness.emitTauriEvent("file-received", {
      id: "f-9", createdAt: Date.now(), updatedAt: Date.now(),
      from: { machineId: "m-pc", machineName: "studio-pc", agentId: "agent-b", agentName: "Bo" },
      to: { machineId: "m-host", machineName: "Studio", agentId: "p1", agentName: "Ada" },
      items: ["notes.md"], files: [], totalBytes: 10, doneBytes: 10, state: "done", notify: true,
      saved: ["/Users/me/Downloads/Agent Office/Ada/notes.md"], via: "agent",
    });
  });
  await page.evaluate(async () => (await window.__guiHarness.store()).getState().closeModal());
  await page.waitForTimeout(300);
  await shot("files-arrival");
  await page.evaluate(async () => (await window.__guiHarness.store()).getState().openModal({ kind: "settings" }));
  await page.getByRole("tab", { name: /머신 연동|Machines/ }).click();
  await page.waitForTimeout(300);
  await page.locator(".settings-form").last().evaluate((el) => el.scrollIntoView({ block: "end" }));
  await page.evaluate(() => document.querySelectorAll(".settings-item").forEach((el) => el.scrollIntoView({ block: "end" })));
  await page.waitForTimeout(200);
  await shot("files-settings");
};
