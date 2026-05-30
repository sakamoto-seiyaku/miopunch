const { calls, clipboardText, expect, inviteCode, openDesktop, snapshotFor, test } = require("./support/desktop");

test("admin invite action uses the runtime action surface and keeps the invite visible and copyable", async ({ page }) => {
  await openDesktop(page, {
    snapshot: snapshotFor("Enroll"),
    inviteDataMode: "string",
  });

  await page.getByRole("button", { name: "Admin" }).click();
  await page.locator("#invite-mode").selectOption("auto");
  await page.locator("#invite-uses").fill("3");
  await page.locator("#invite-expires").fill("30m");
  await page.locator("#form-invite button[type='submit']").click();

  await expect(page.locator("#invite-code-output")).toHaveValue(inviteCode);
  await expect(page.locator("#invite-result-card")).toBeVisible();
  await expect(page.locator("#approve-code")).toHaveValue(inviteCode);
  await expect(page.getByRole("button", { name: "Copy invite code" })).toBeVisible();
  await expect(page.locator("#topbar-title")).toHaveText("Admin");

  await page.getByRole("button", { name: "Copy invite code" }).click();
  await expect.poll(async () => clipboardText(page)).toBe(inviteCode);

  await page.getByRole("button", { name: "Network" }).click();
  await expect(page.locator("#topbar-title")).toHaveText("Network");
  await expect(page.locator("#recent-invite-code")).toHaveValue(inviteCode);

  await expect.poll(async () => {
    const runtimeCalls = (await calls(page)).filter((call) => call.method === "RuntimeAction");
    return runtimeCalls;
  }).toContainEqual({
    method: "RuntimeAction",
    action: "invite",
    args: {
      mode: "auto",
      max_uses: 3,
      expires: "30m",
    },
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
  await expect(page.locator("#btn-init-network-new")).toBeDisabled();

  await page.getByRole("button", { name: "Admin" }).click();
  await expect(page.getByRole("button", { name: "Create invite" })).toBeDisabled();
  await expect(page.locator("#approve-code")).toBeDisabled();
  await expect(page.locator("#join-code")).toBeDisabled();
});
