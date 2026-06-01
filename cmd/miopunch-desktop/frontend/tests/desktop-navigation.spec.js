const { expect, openDesktop, PEERS, snapshotFor, test } = require("./support/desktop");

test("desktop console exposes the four operator tabs", async ({ page }) => {
  await openDesktop(page, { snapshot: snapshotFor("Network") });

  const tabLabels = await page.locator("[data-tab] span").allTextContents();
  expect(tabLabels).toEqual(["Network", "Shell", "Admin", "Settings"]);
  await expect(page.locator("[data-tab]")).toHaveCount(4);
});

test("runtime summary and evidence are rendered without redefining the contract", async ({ page }) => {
  await openDesktop(page, {
    snapshot: snapshotFor("Discover", {
      summary: { text: "1 peer discovered and ready for punch" },
      evidence: {
        facts: [
          { message: `peer_id=${PEERS.remote}` },
          { message: "online_state=online" },
        ],
        suggestions: [{ message: "continue to Punch" }],
      },
    }),
  });

  await expect(page.locator("#topbar-title")).toHaveText("Network");
  await expect(page.locator("#stage-chip")).toHaveText("Stage Discover");
  await expect(page.getByText("1 peer discovered and ready for punch")).toBeVisible();
  await expect(page.getByText(`peer_id=${PEERS.remote}`)).toBeVisible();
  await expect(page.getByText("continue to Punch")).toBeVisible();
});

test("network page renders typed network_id without relying on evidence facts", async ({ page }) => {
  await openDesktop(page, {
    snapshot: snapshotFor("Network", {
      discover_view: {
        network_id: "net_typed_projection",
      },
      evidence: {
        facts: [{ message: "some_other_fact=1" }],
        suggestions: [{ message: "bootstrap if needed" }],
      },
    }),
  });

  await expect(page.locator("#topbar-title")).toHaveText("Network");
  await expect(page.locator("#stage-chip")).toHaveText("Stage Network");
  await expect(page.getByText("net_typed_projection")).toBeVisible();
  await expect(page.getByText("some_other_fact=1")).toBeVisible();
});
