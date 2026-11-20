// Meeting room: start the ready discussion (simultaneous first round), let resting participants walk to the meeting
// room while the working one (Bo) stays at the desk with a speech bubble, then shoot it in the three themes. Ending the
// discussion sends everyone back (last shot).
//   npm run gui:shot -- --scenario meeting --script scripts/gui-steps/discussion-office.mjs
export default async ({ page, shot }) => {
  await page.waitForTimeout(1500);
  const blocked = await page.evaluate(async () => {
    const { startDiscussion } = await window.__guiHarness.discussionRun();
    return startDiscussion("disc-meeting") ?? null;
  });
  console.log(`start blocked: ${blocked}`);
  // 쉬던 캐릭터가 탕비실에서 회의실까지 걷는 시간(28px/s, 맵 대각 ≈ 300px).
  await page.waitForTimeout(14000);
  const state = await page.evaluate(async () => {
    const { useDiscussionStore } = await window.__guiHarness.discussionStore();
    const d = useDiscussionStore.getState().discussions.find((x) => x.id === "disc-meeting");
    return { state: d.state, phase: d.run?.phase, turns: d.run?.turns.map((t) => [t.agentId, t.sentAt !== undefined, t.text !== undefined]) };
  });
  console.log(`discussion: ${JSON.stringify(state)}`);
  for (const theme of ["daylight", "midnight", "sakura"]) {
    await page.evaluate(async (id) => (await window.__guiHarness.store()).getState().setTheme(id), theme);
    await page.waitForTimeout(400);
    await shot(`meeting-${theme}`);
  }
  await page.evaluate(async () => {
    const { useDiscussionStore } = await window.__guiHarness.discussionStore();
    const d = useDiscussionStore.getState().discussions.find((x) => x.id === "disc-meeting");
    await useDiscussionStore.getState().commit({ ...d, state: "finished", run: undefined, endedAt: Date.now() });
  });
  await page.waitForTimeout(12000);
  await shot("meeting-ended");
};
