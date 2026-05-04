const { PEERS, emitRuntime, expect, openDesktop, test } = require("./support/desktop");

test("runtime task snapshot and events update visible peer task state", async ({ page }) => {
  await openDesktop(page);
  await page.locator(`[data-open-peer="${PEERS.member}"]`).last().click();
  await expect(page.locator(`[data-copy-peer="${PEERS.member}"]`)).toBeVisible();

  await emitRuntime(page, "localapi:event", {
    kind: "snapshot",
    tasks: [
      {
        task_id: "runtime-peer-task",
        kind: "ping",
        status: "running",
        stage: "started",
        facts: [{ message: `peer_id=${PEERS.member}` }],
        suggestions: [],
      },
    ],
  });

  await expect(page.getByText("runtime-peer-task | stage=started")).toBeVisible();

  await emitRuntime(page, "localapi:event", {
    task_id: "runtime-peer-task",
    kind: "stage",
    stage: "payload exchanged",
  });

  await expect(page.getByText("runtime-peer-task | stage=payload exchanged")).toBeVisible();

  await emitRuntime(page, "localapi:event", {
    task_id: "runtime-peer-task",
    kind: "done",
    reason_code: "OK",
    exit_code: 0,
  });

  await expect(page.locator(".row-card").filter({ hasText: "runtime-peer-task" }).getByText("done")).toBeVisible();
});

test("startup error runtime event shows a visible toast", async ({ page }) => {
  await openDesktop(page);

  await emitRuntime(page, "desktop:startup_error", {
    component: "desktop",
    error: {
      reason_code: "daemon_not_running",
      message: "LocalAPI is not reachable",
    },
  });

  await expect(page.locator("#toast")).toContainText("desktop: daemon_not_running: LocalAPI is not reachable");
});

test("runtime connection events re-render the active Diagnostics view immediately", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=settings&section=diagnostics" });
  await emitRuntime(page, "localapi:connection", {
    connected: false,
    failure: {
      reason_code: "daemon_not_running",
      suggestions: [{ message: "Start the miopunch service, then refresh" }],
      facts: [],
    },
  });
  await expect(page.getByText("Start the miopunch service, then refresh")).toBeVisible();
});
