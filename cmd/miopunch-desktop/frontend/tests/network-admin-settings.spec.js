const { calls, expect, openDesktop, test } = require("./support/desktop");

test("settings tab drives LocalAPI override, diagnostics export, and quit through the current bridge", async ({ page }) => {
  await openDesktop(page);

  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.locator("#topbar-title")).toHaveText("Settings");

  await page.locator("#override-addr").fill("unix:/tmp/miopunch-ui-test-override.sock");
  await page.getByRole("button", { name: "Apply" }).click();

  await expect.poll(async () => calls(page)).toContainEqual({
    method: "SetLocalAPIOverride",
    addr: "unix:/tmp/miopunch-ui-test-override.sock",
  });
  await expect(page.getByText("unix:/tmp/miopunch-ui-test-override.sock")).toBeVisible();

  await page.getByRole("button", { name: "Clear" }).click();
  await expect.poll(async () => calls(page)).toContainEqual({ method: "ClearLocalAPIOverride" });

  await page.getByRole("button", { name: "Export diagnostics" }).click();
  await expect.poll(async () => calls(page)).toContainEqual({ method: "ExportDiagnostics" });
  await expect(page.locator(".helper.mono").filter({ hasText: "/tmp/miopunch-diagnostics.zip" })).toBeVisible();

  await page.getByRole("button", { name: "Quit miopunch desktop" }).click();
  await expect.poll(async () => calls(page)).toContainEqual({ method: "Quit" });
});
