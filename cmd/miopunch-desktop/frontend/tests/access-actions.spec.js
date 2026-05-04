const {
  calls,
  createTaskCalls,
  expect,
  expectCreateTaskCall,
  inviteCode,
  openDesktop,
  test,
} = require("./support/desktop");

async function openAccessFlow(page, flow, options = {}) {
  await openDesktop(page, { ...options, path: `/?tab=access&flow=${flow}` });
  const title = flow === "invite" ? "Create invite" : flow === "approve" ? "Approve request" : "Join network";
  await expect(page.getByRole("heading", { name: title })).toBeVisible();
}

test("Access Join submits object args and exports the completed report", async ({ page }) => {
  await openAccessFlow(page, "join");

  await page.locator("#join-code").fill("mp:v0.join.test");
  await page.locator("#join-form").getByRole("button", { name: "Join" }).click();

  await expectCreateTaskCall(page, "join", { code: "mp:v0.join.test" });
  await expect(page.getByText("membership accepted")).toBeVisible();
  await expect(page.getByRole("button", { name: "Export report" })).toBeEnabled();

  await page.getByRole("button", { name: "Export report" }).click();

  await expect(page.locator("#join-report-path")).toContainText("/tmp/ui-join-001.md");
  await expect.poll(() => calls(page)).toContainEqual({ method: "ExportTaskReport", taskID: "ui-join-001" });
});

test("Access Join validates missing invite code without creating a task", async ({ page }) => {
  await openAccessFlow(page, "join");

  await page.locator("#join-form").getByRole("button", { name: "Join" }).click();

  await expect(page.locator("#toast")).toContainText("Missing invite code");
  await expect.poll(() => createTaskCalls(page, "join")).toEqual([]);
});

test("Access invite Create calls bridge with object args and renders code", async ({ page }) => {
  await openAccessFlow(page, "invite");

  await page.getByRole("button", { name: "Create" }).click();

  await expect(page.locator("#invite-code")).toHaveValue(inviteCode);
  await expect(page.getByRole("button", { name: "Copy" })).toBeEnabled();
  await expectCreateTaskCall(page, "invite", {});
});

test("Access invite Create recovers from bridge failure", async ({ page }) => {
  await openAccessFlow(page, "invite", { createTaskModes: { invite: "failure" } });

  await page.getByRole("button", { name: "Create" }).click();

  await expect(page.locator("#invite-hint")).toContainText("Create failed");
  await expect(page.locator("#invite-hint")).toContainText("invite failed in fake bridge");
  await expect(page.getByRole("button", { name: "Create" })).toBeEnabled();
});

test("Access invite Create recovers from bridge timeout", async ({ page }) => {
  await openAccessFlow(page, "invite", { createTaskModes: { invite: "timeout" } });

  await page.getByRole("button", { name: "Create" }).click();

  await expect(page.locator("#invite-hint")).toContainText("Create invite timed out");
  await expect(page.getByRole("button", { name: "Create" })).toBeEnabled();
});

test("Access Approve submits object args and renders progress", async ({ page }) => {
  await openAccessFlow(page, "approve");

  await page.locator("#approve-code").fill("mp:v0.approve.test");
  await page.locator("#approve-form").getByRole("button", { name: "Start approval" }).click();

  await expectCreateTaskCall(page, "approve", { code: "mp:v0.approve.test" });
  await expect(page.getByText("waiting for joiner")).toBeVisible();
});

test("Access Approve validates missing invite code without creating a task", async ({ page }) => {
  await openAccessFlow(page, "approve");

  await page.locator("#approve-form").getByRole("button", { name: "Start approval" }).click();

  await expect(page.locator("#toast")).toContainText("Missing invite code");
  await expect.poll(() => createTaskCalls(page, "approve")).toEqual([]);
});

test("Access hides admin-only flows for member role", async ({ page }) => {
  await openDesktop(page, { fixture: "member", path: "/?tab=access" });

  await expect(page.locator(".page-title")).toHaveText("Access");
  await expect(page.getByRole("button", { name: /Join network/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Create invite/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /Approve request/ })).toHaveCount(0);
});

test("Access invite Create is disabled when the desktop bridge is disconnected", async ({ page }) => {
  await openAccessFlow(page, "invite", { fixture: "disconnected" });

  await expect(page.getByRole("button", { name: "Create" })).toBeDisabled();
});
