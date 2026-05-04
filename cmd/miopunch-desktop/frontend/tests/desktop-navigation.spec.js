const { PEERS, expect, openDesktop, test } = require("./support/desktop");

async function returnToOverview(page, heading) {
  await page.locator(".module-switch [data-open-overview]").click();
  await expect(page.locator(".page-title")).toHaveText(heading);
}

test("primary tabs render owner overview pages without browser errors", async ({ page }) => {
  await openDesktop(page);

  await expect(page.getByRole("heading", { name: "Device network" })).toBeVisible();

  await page.getByRole("button", { name: "Access", exact: true }).click();
  await expect(page.locator(".page-title")).toHaveText("Access");
  await expect(page.getByRole("button", { name: /Create invite/ })).toBeVisible();

  await page.getByRole("button", { name: "Admin", exact: true }).click();
  await expect(page.locator(".page-title")).toHaveText("Governance");
  await expect(page.getByText("Access control")).toBeVisible();

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.locator(".page-title")).toHaveText("Settings");
  await expect(page.locator('[data-open-setting="localapi"]').last()).toBeVisible();

  await page.getByRole("button", { name: "Network", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Device network" })).toBeVisible();
});

test("second-level views return to their primary tab overview", async ({ page }) => {
  await openDesktop(page);

  await page.locator(`[data-open-peer="${PEERS.member}"]`).last().click();
  await expect(page.locator(`[data-copy-peer="${PEERS.member}"]`)).toBeVisible();
  await returnToOverview(page, "Device network");

  await page.getByRole("button", { name: "Access", exact: true }).click();
  await page.locator('[data-open-flow="invite"]').last().click();
  await expect(page.getByRole("heading", { name: "Create invite" })).toBeVisible();
  await returnToOverview(page, "Access");

  await page.getByRole("button", { name: "Admin", exact: true }).click();
  await page.locator(`[data-open-member="${PEERS.member}"]`).last().click();
  await expect(page.getByRole("button", { name: "Revoke" })).toBeVisible();
  await returnToOverview(page, "Governance");

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.locator('[data-open-setting="diagnostics"]').last().click();
  await expect(page.getByRole("heading", { name: "Diagnostics" })).toBeVisible();
  await returnToOverview(page, "Settings");
});

test("member role cannot open the Admin primary tab", async ({ page }) => {
  await openDesktop(page, { fixture: "member", path: "/?tab=admin" });

  await expect(page.locator("[data-admin-nav]")).toBeHidden();
  await expect(page.getByRole("heading", { name: "Device network" })).toBeVisible();
});

test("empty network fixture renders no-network overview and hides admin controls", async ({ page }) => {
  await openDesktop(page, { fixture: "empty" });

  await expect(page.getByRole("heading", { name: "Device network" })).toBeVisible();
  await expect(page.locator(".tile-title", { hasText: "peer-new-node-0000" })).toBeVisible();
  await expect(page.getByText("Not joined").first()).toBeVisible();
  await expect(page.locator("[data-admin-nav]")).toBeHidden();
});

test("owner Admin deep link lands on Admin after topology snapshot", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=admin" });
  await expect(page.locator(".page-title")).toHaveText("Governance");
});
