const { PEERS, calls, emitRuntime, expect, openDesktop, setRuntimeSnapshot, test } = require("./support/desktop");

test("runtime task snapshot and events update visible peer task state", async ({ page }) => {
  await openDesktop(page);
  await page.locator(`[data-open-peer="${PEERS.member}"]`).last().click();
  await expect(page.locator(`[data-copy-peer="${PEERS.member}"]`)).toBeVisible();

  await emitRuntime(page, "desktop:state", {
    kind: "task.upsert",
    base_rev: 0,
    rev: 1,
    task: {
      task_id: "runtime-peer-task",
      kind: "ping",
      status: "running",
      stage: "started",
      facts: [{ message: `peer_id=${PEERS.member}` }],
      suggestions: [],
    },
  });

  await expect(page.getByText("runtime-peer-task | stage=started")).toBeVisible();

  await emitRuntime(page, "desktop:state", {
    kind: "task.upsert",
    base_rev: 1,
    rev: 2,
    task: {
      task_id: "runtime-peer-task",
      kind: "ping",
      status: "running",
      stage: "payload exchanged",
      facts: [{ message: `peer_id=${PEERS.member}` }],
      suggestions: [],
    },
  });

  await expect(page.getByText("runtime-peer-task | stage=payload exchanged")).toBeVisible();

  await emitRuntime(page, "desktop:state", {
    kind: "task.upsert",
    base_rev: 2,
    rev: 3,
    task: {
      task_id: "runtime-peer-task",
      kind: "ping",
      status: "done",
      stage: "payload exchanged",
      reason_code: "OK",
      exit_code: 0,
      report_ready: true,
      facts: [{ message: `peer_id=${PEERS.member}` }],
      suggestions: [],
    },
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

test("refresh restarts runtime stream after connected bootstrap failure", async ({ page }) => {
  await openDesktop(page, { runtimeStartFailures: 1, path: "/?tab=settings&section=diagnostics" });

  await expect(page.getByText("desktop_stream=failed")).toBeVisible();
  const initialStarts = (await calls(page)).filter((call) => call.method === "DesktopRuntimeStart").length;
  const initialResyncs = (await calls(page)).filter((call) => call.method === "DesktopRuntimeResync").length;

  await page.getByRole("button", { name: "Refresh" }).click();

  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "DesktopRuntimeStart").length).toBeGreaterThan(initialStarts);
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "DesktopRuntimeResync").length).toBe(initialResyncs);
  await expect(page.getByText("desktop_stream=live")).toBeVisible();
});

test("revision gap resyncs runtime snapshot and applies caught-up state", async ({ page }) => {
  await openDesktop(page, { runtimeResyncDelayMs: 40, timeoutMs: 500 });
  await page.locator(`[data-open-peer="${PEERS.member}"]`).last().click();
  await expect(page.locator(`[data-copy-peer="${PEERS.member}"]`)).toBeVisible();

  const initialStarts = (await calls(page)).filter((call) => call.method === "DesktopRuntimeStart").length;
  const initialResyncs = (await calls(page)).filter((call) => call.method === "DesktopRuntimeResync").length;

  await setRuntimeSnapshot(page, {
    rev: 3,
    tasks: [],
    diagnostics: [{ message: "desktop_runtime_rev=3" }],
  });

  await emitRuntime(page, "desktop:state", {
    kind: "task.upsert",
    base_rev: 2,
    rev: 3,
    task: {
      task_id: "gap-trigger-task",
      kind: "ping",
      status: "running",
      stage: "missed event",
      facts: [{ message: `peer_id=${PEERS.member}` }],
      suggestions: [],
    },
  });

  await emitRuntime(page, "desktop:state", {
    kind: "task.upsert",
    base_rev: 3,
    rev: 4,
    task: {
      task_id: "gap-recovered-task",
      kind: "ping",
      status: "running",
      stage: "caught up after restart",
      facts: [{ message: `peer_id=${PEERS.member}` }],
      suggestions: [],
    },
  });

  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "DesktopRuntimeResync").length).toBeGreaterThan(initialResyncs);
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "DesktopRuntimeStart").length).toBe(initialStarts);
  await expect(page.getByText("gap-recovered-task | stage=caught up after restart")).toBeVisible();
});

test("runtime connection events re-render the active Diagnostics view immediately", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=settings&section=diagnostics" });
  await emitRuntime(page, "desktop:runtime", {
    kind: "connection",
    connection: {
      connected: false,
      failure: {
        reason_code: "daemon_not_running",
        suggestions: [{ message: "retry desktop connection" }],
        facts: [],
      },
    },
  });
  await expect(page.getByText("retry desktop connection")).toBeVisible();
});

test("runtime stream retrying event shows toast and diagnostics", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=settings&section=diagnostics" });

  await emitRuntime(page, "desktop:runtime", {
    kind: "stream_retrying",
    error: {
      reason_code: "unavailable",
      message: "desktop events disconnected",
    },
  });

  await expect(page.locator("#toast")).toContainText("Runtime stream retrying: unavailable: desktop events disconnected");
  await expect(page.getByText("desktop_stream=retrying")).toBeVisible();
  await expect(page.getByText("desktop_stream_error=unavailable")).toBeVisible();
});

test("runtime diagnostics replace re-renders the active Diagnostics view immediately", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=settings&section=diagnostics" });

  await emitRuntime(page, "desktop:state", {
    kind: "diagnostics.replace",
    base_rev: 0,
    rev: 1,
    diagnostics: [{ message: "running_tasks=1" }],
  });

  await expect(page.getByText("running_tasks=1")).toBeVisible();
});
