const { PEERS, expect, openDesktop, test } = require("./support/desktop");

async function returnToOverview(page, tabTitle) {
  await page.locator(".workspace-tabs [data-open-overview]").click();
  await expect(page.locator("#topbar-title")).toHaveText(tabTitle);
}

test("primary tabs render owner overview pages without browser errors", async ({ page }) => {
  await openDesktop(page);

  await expect(page.locator("#topbar-title")).toHaveText("Network");
  await expect(page.locator(".page-title", { hasText: "Device network" })).toHaveCount(0);
  await expect(page.locator(".module-switch")).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Access", exact: true })).toHaveCount(0);

  await page.getByRole("button", { name: "Shell", exact: true }).click();
  await expect(page.locator("#topbar-title")).toHaveText("Shell");
  await expect(page.locator(".page-title", { hasText: "Shell" })).toHaveCount(0);
  await expect(page.locator(".shell-overview-grid")).toBeVisible();
  await expect(page.locator("#terminal")).toHaveCount(0);

  await page.getByRole("button", { name: "Admin", exact: true }).click();
  await expect(page.locator(".page-title")).toHaveText("Governance");
  await expect(page.getByText("Who can access this network")).toBeVisible();
  await expect(page.getByRole("button", { name: /Create invite/ })).toBeVisible();

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(page.locator(".page-title")).toHaveText("Settings");
  await expect(page.locator('[data-open-setting="localapi"]').last()).toBeVisible();

  await page.getByRole("button", { name: "Network", exact: true }).click();
  await expect(page.locator("#topbar-title")).toHaveText("Network");
  await expect(page.locator(".page-title", { hasText: "Device network" })).toHaveCount(0);
});

test("second-level views return to their primary tab overview", async ({ page }) => {
  await openDesktop(page);

  await page.locator(`[data-open-peer="${PEERS.member}"]`).last().click();
  await expect(page.locator(`[data-copy-peer="${PEERS.member}"]`)).toBeVisible();
  await returnToOverview(page, "Network");

  await page.getByRole("button", { name: "Shell", exact: true }).click();
  await page.locator('[data-shell-peer]').first().click();
  await expect(page.locator("#shell-form")).toBeVisible();
  await page.getByRole("button", { name: "Network", exact: true }).click();
  await expect(page.locator("#topbar-title")).toHaveText("Network");

  await page.getByRole("button", { name: "Admin", exact: true }).click();
  await page.locator('[data-open-flow="invite"]').last().click();
  await expect(page.getByRole("heading", { name: "Create invite" })).toBeVisible();
  await returnToOverview(page, "Admin");

  await page.locator(`[data-open-member="${PEERS.member}"]`).last().click();
  await expect(page.getByRole("button", { name: "Revoke" })).toBeVisible();
  await returnToOverview(page, "Admin");

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.locator('[data-open-setting="diagnostics"]').last().click();
  await expect(page.locator(".page-title")).toHaveText("Diagnostics");
  await returnToOverview(page, "Settings");
});

test("member role cannot open the Admin primary tab", async ({ page }) => {
  await openDesktop(page, { fixture: "member", path: "/?tab=admin" });

  await expect(page.locator("[data-admin-nav]")).toBeHidden();
  await expect(page.locator("#topbar-title")).toHaveText("Network");
  await expect(page.locator(".page-title", { hasText: "Device network" })).toHaveCount(0);
});

test("selected inactive peer renders as target with recent failure evidence", async ({ page }) => {
  await openDesktop(page, { fixture: "selected-inactive" });

  const travelerTile = page.locator(`[data-map-peer="${PEERS.traveler}"]`).last();
  const statusChip = travelerTile.locator(".chip").last();
  await expect(statusChip).toHaveText("target");
  await expect(statusChip).toHaveClass(/chip-muted/);
  await expect(statusChip).not.toHaveClass(/chip-running/);

  await travelerTile.click();
  const peerDetails = page.locator(".network-device-panel .detail-table").first();
  await expect(peerDetails).toContainText("target");
  await expect(peerDetails).toContainText("ready");
  await expect(peerDetails).toContainText("stage=peer_contact");
  await expect(peerDetails).toContainText("reason=UNAVAILABLE");
  await expect(peerDetails).toContainText("stop=dial_failed");
});

test("empty network fixture renders join-first network setup without shell or admin", async ({ page }) => {
  await openDesktop(page, { fixture: "empty" });

  await expect(page.getByRole("heading", { name: "Join a network" })).toBeVisible();
  await expect(page.locator("#join-form")).toBeVisible();
  await expect(page.locator("[data-admin-nav]")).toBeHidden();
  await expect(page.locator("[data-shell-nav]")).toBeHidden();
});

test("empty network fixture can enable first-run owner admin mode from Settings", async ({ page }) => {
  await openDesktop(page, { fixture: "empty", path: "/?tab=settings" });

  await expect(page.locator("[data-admin-nav]")).toBeHidden();
  await expect(page.getByRole("button", { name: "Enable Owner/Admin mode" })).toBeVisible();

  await page.getByRole("button", { name: "Enable Owner/Admin mode" }).click();

  await expect(page.locator("[data-admin-nav]")).toBeVisible();
  await expect(page.locator("#topbar-title")).toHaveText("Admin");
  await expect(page.locator(".page-title")).toHaveText("Governance");
  await expect(page.getByRole("button", { name: /Create invite/ })).toBeVisible();
});

test("owner Admin deep link lands on Admin after topology snapshot", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=admin" });
  await expect(page.locator(".page-title")).toHaveText("Governance");
});
