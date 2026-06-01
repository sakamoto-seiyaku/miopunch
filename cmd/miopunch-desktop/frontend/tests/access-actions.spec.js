const { calls, clipboardText, expect, inviteCode, openDesktop, snapshotFor, test } = require("./support/desktop");

test("admin invite action uses the runtime action surface and keeps the invite visible and copyable", async ({ page }) => {
  await openDesktop(page, {
    snapshot: snapshotFor("Enroll"),
    inviteDataMode: "string",
  });

  await page.getByRole("button", { name: "Admin", exact: true }).click();
  await page.locator('[data-open-flow="invite"]').click();
  await page.locator("#btn-invite").click();

  await expect(page.locator("#invite-code")).toHaveValue(inviteCode);
  await expect(page.locator("#invite-qr")).toBeVisible();
  await expect(page.locator("[data-copy-invite]")).toBeVisible();
  await expect(page.locator("#topbar-title")).toHaveText("Admin");

  await page.locator("[data-copy-invite]").click();
  await expect.poll(async () => clipboardText(page)).toBe(inviteCode);

  await page.getByRole("button", { name: "Network", exact: true }).click();
  await expect(page.locator("#topbar-title")).toHaveText("Network");
  await expect(page.locator("#recent-invite-code")).toHaveValue(inviteCode);

  await expect.poll(async () => {
    const runtimeCalls = (await calls(page)).filter((call) => call.method === "RuntimeAction");
    return runtimeCalls;
  }).toContainEqual({
    method: "RuntimeAction",
    action: "invite",
    args: {},
  });
});

test("runtime actions stay disabled while LocalAPI is disconnected", async ({ page }) => {
  await openDesktop(page, {
    snapshot: snapshotFor("Network"),
    connection: {
      connected: false,
      selected: "none",
      addr: "",
      bootstrap_state: "failed",
      failure: {
        stage: "desktop",
        reason_code: "DAEMON_NOT_RUNNING",
        exit_code: 69,
        message: "LocalAPI is not connected",
      },
    },
  });

  await expect(page.locator(".helper.helper-error").filter({ hasText: "LocalAPI is not connected" }).first()).toBeVisible();
  await page.getByRole("button", { name: "Admin", exact: true }).click();
  await page.locator('[data-open-flow="invite"]').click();
  await expect(page.locator("#btn-invite")).toBeDisabled();
  await page.locator("[data-open-overview]").click();
  await page.locator('[data-open-flow="approve"]').click();
  await expect(page.locator("#approve-code")).toBeDisabled();
});
