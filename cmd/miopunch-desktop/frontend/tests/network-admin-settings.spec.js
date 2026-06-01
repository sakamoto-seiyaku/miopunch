const { calls, expect, openDesktop, test } = require("./support/desktop");

test("settings tab drives LocalAPI override, diagnostics export, and quit through the current bridge", async ({ page }) => {
  await openDesktop(page);

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.locator("#topbar-title")).toHaveText("Settings");

  await page.locator('[data-open-setting="localapi"]').first().click();
  await page.locator("#localapi-override").fill("unix:/tmp/miopunch-ui-test-override.sock");
  await page.locator("#btn-localapi-apply").click();

  await expect.poll(async () => calls(page)).toContainEqual({
    method: "SetLocalAPIOverride",
    addr: "unix:/tmp/miopunch-ui-test-override.sock",
  });
  await expect(page.locator("#connection-addr")).toHaveText("unix:/tmp/miopunch-ui-test-override.sock");

  await page.locator("#btn-localapi-clear").click();
  await expect.poll(async () => calls(page)).toContainEqual({ method: "ClearLocalAPIOverride" });

  await page.locator('[data-open-setting="runtime"]').first().click();
  await page.locator("#settings-log-level").selectOption("debug");
  await page.locator("#runtime-config-form button[type='submit']").click();
  await expect.poll(async () => calls(page)).toContainEqual({
    method: "SaveDesktopConfig",
    update: { preferences: { log_level: "debug" } },
  });
  await expect(page.locator("#runtime-config-form").getByText("Log level saved")).toBeVisible();
  await expect(page.locator(".detail-row", { hasText: "Log level" }).locator("strong")).toHaveText("debug");

  await page.locator('[data-open-setting="diagnostics"]').first().click();
  await page.locator("#btn-export-diagnostics").click();
  await expect.poll(async () => calls(page)).toContainEqual({ method: "ExportDiagnostics" });
  await expect(page.getByText("/tmp/miopunch-diagnostics.zip")).toBeVisible();

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.locator("#btn-app-quit").click();
  await expect.poll(async () => calls(page)).toContainEqual({ method: "Quit" });
});
