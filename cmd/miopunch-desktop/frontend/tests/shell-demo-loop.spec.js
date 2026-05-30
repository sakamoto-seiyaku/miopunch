const { calls, expect, openDesktop, shellLog, snapshotFor, test } = require("./support/desktop");

test("shell attach stays gated on ping success and bridges with shell_session_id", async ({ page }) => {
  await openDesktop(page, { snapshot: snapshotFor("SecureSession") });

  await page.getByRole("button", { name: "Shell" }).click();
  await expect(page.locator("#topbar-title")).toHaveText("Shell");
  await expect(page.locator("#stage-chip")).toHaveText("Stage SecureSession");
  await expect(page.locator(".helper.mt").filter({ hasText: "Run Ping first" })).toBeVisible();
  await expect(page.locator("#btn-ping")).toBeVisible();
  await expect(page.locator("#btn-shell-open")).toBeDisabled();

  await page.locator("#btn-ping").click();

  await expect.poll(async () => {
    const runtimeCalls = (await calls(page)).filter((call) => call.method === "RuntimeAction");
    return runtimeCalls.some((call) => call.action === "ping");
  }).toBe(true);

  await expect(page.locator("#btn-shell-open")).toBeEnabled();
  await page.locator("#btn-shell-open").click();

  await expect(page.locator("#shell-output")).toContainText("[attached shell-session-01]");
  await expect(page.locator("#shell-output")).toContainText("welcome to miopunch shell");

  await expect.poll(async () => shellLog(page)).toContainEqual(expect.objectContaining({
    type: "open",
    sessionID: "shell-session-01",
    url: expect.stringContaining("/api/v1/shell/shell-session-01/ws?token=ui-test-token"),
  }));

  await page.locator("#shell-input").fill("uname -a");
  await page.locator("#form-shell-input button[type='submit']").click();

  await expect.poll(async () => shellLog(page)).toContainEqual(expect.objectContaining({
    type: "send",
    sessionID: "shell-session-01",
    data: "uname -a\n",
  }));
});
