const {
  PEERS,
  calls,
  createTaskCalls,
  emitRuntime,
  expect,
  expectCreateTaskCall,
  openDesktop,
  test,
} = require("./support/desktop");

async function openShell(page, options = {}) {
  await openDesktop(page, options);
  await page.locator(`[data-open-peer="${PEERS.member}"]`).last().click();
  await expect(page.locator(`[data-copy-peer="${PEERS.member}"]`)).toBeVisible();
  await page.getByRole("button", { name: /Open shell/ }).click();
  await expect(page.locator("#shell-form")).toBeVisible();
}

async function openShellOptions(page) {
  await expect(page.locator(".shell-connect-panel")).toBeVisible();
  await expect(page.locator("#shell-form")).toBeVisible();
}

async function datalistOptions(page, selector) {
  return page.locator(selector).evaluate((node) => Array.from(node.options).map((option) => option.value));
}

test("shell demo loop discovers choices, connects, disconnects, and reconnects", async ({ page }) => {
  await openShell(page);
  await openShellOptions(page);

  await expect(page.locator("#shell-target")).toHaveValue("local");
  await expect(page.locator("#shell-session")).toHaveValue("main");

  await page.getByRole("button", { name: "Find targets" }).click();
  await expectCreateTaskCall(page, "sh_ls", { peer_id: PEERS.member, target: "" });
  await expect.poll(() => datalistOptions(page, "#shell-target-options")).toEqual(expect.arrayContaining(["local", "ssh:ops"]));

  await page.locator("#shell-target").fill("ssh:ops");
  await expect(page.locator("#shell-target")).toHaveValue("ssh:ops");

  await page.locator("#btn-shell-connect").click();
  await expectCreateTaskCall(page, "sh_attach", {
    peer_id: PEERS.member,
    target: "ssh:ops",
    session: "main",
  });
  await expect(page.locator("#shell-status")).toContainText("Connected");
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "Terminal.focus").length).toBeGreaterThan(0);

  await page.getByRole("button", { name: "Disconnect" }).click();
  await expect(page.locator("#shell-phase")).toHaveText("disconnected");
  await expect(page.locator("#shell-error")).toBeHidden();
  await expect(page.locator("#shell-target")).toHaveValue("ssh:ops");
  await expect(page.locator("#shell-session")).toHaveValue("main");
  await expect(page.getByRole("button", { name: "Resume", exact: true })).toBeEnabled();
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "WebSocket.close").length).toBeGreaterThan(0);

  await page.getByRole("button", { name: "Resume", exact: true }).click();
  await expect.poll(async () => (await createTaskCalls(page, "sh_attach")).length).toBe(1);
  await expect(page.locator("#shell-status")).toContainText("Resumed");
});

test("shell target entry can discover session names and open one", async ({ page }) => {
  await openShell(page);
  await openShellOptions(page);

  await page.locator("#shell-target").fill("ssh:ops");
  await page.getByRole("button", { name: "Find sessions" }).click();
  await expectCreateTaskCall(page, "sh_ls", { peer_id: PEERS.member, target: "ssh:ops" });
  await expect.poll(() => datalistOptions(page, "#shell-session-options")).toEqual(expect.arrayContaining(["ops-main"]));

  await page.locator("#shell-session").fill("ops-main");
  await page.locator("#btn-shell-connect").click();
  await expectCreateTaskCall(page, "sh_attach", {
    peer_id: PEERS.member,
    target: "ssh:ops",
    session: "ops-main",
  });
});

test("shell overview and peer target rows update the active peer context", async ({ page }) => {
  await openDesktop(page);
  await page.getByRole("button", { name: "Shell", exact: true }).click();

  await expect(page.locator(".shell-overview-grid")).toBeVisible();
  await expect(page.locator("#terminal")).toHaveCount(0);
  const overviewGap = await page.evaluate(() => {
    const tabs = document.querySelector(".shell-page .workspace-tabs");
    const grid = document.querySelector(".shell-overview-grid");
    if (!tabs || !grid) return Number.POSITIVE_INFINITY;
    return Math.round(grid.getBoundingClientRect().top - tabs.getBoundingClientRect().bottom);
  });
  expect(overviewGap).toBeLessThanOrEqual(32);

  await page.locator('.workspace-tab[data-shell-peer="peer-livingroom-mini-03"]').click();

  await expect(page.locator("#shell-peer-id")).toHaveValue(PEERS.member);
  await expect(page.locator(".shell-connect-identity")).toContainText("Living Room Mini");
  await expect(page.locator("#shell-target")).toHaveValue("local");
  await expect(page.locator("#shell-form")).toBeVisible();
  await expect(page.locator(".shell-manager")).toHaveCount(0);
  await expect(page.locator(".shell-session-side")).toHaveCount(0);
  await expect(page.getByRole("button", { name: /Hide targets|Hide sessions/ })).toHaveCount(0);
  await expect(page.locator("[data-shell-disconnect]")).toHaveCount(0);
  await expect(page.locator("#shell-phase")).toBeHidden();
});

test("shell connection bar keeps target and session discovery in one workflow", async ({ page }) => {
  await openShell(page);

  await page.locator("#shell-target").fill("ssh:ops");
  await page.getByRole("button", { name: "Find sessions" }).click();
  await expectCreateTaskCall(page, "sh_ls", { peer_id: PEERS.member, target: "ssh:ops" });
  await expect.poll(() => datalistOptions(page, "#shell-session-options")).toEqual(expect.arrayContaining(["ops-main"]));
  await expect(page.locator(".shell-target-card")).toHaveCount(0);
  await expect(page.locator(".shell-session-name-card")).toHaveCount(0);
});

test("shell connection bar keeps controls compact", async ({ page }) => {
  await openShell(page);

  const sizes = await page.evaluate(() => {
    const rect = (selector) => {
      const node = document.querySelector(selector);
      const box = node.getBoundingClientRect();
      return { top: box.top, bottom: box.bottom, width: box.width, height: box.height };
    };
    return {
      find: rect("#btn-shell-find-sessions"),
      create: rect("#btn-shell-connect"),
      connectPanel: rect(".shell-connect-panel"),
      terminal: rect(".shell-terminal-panel"),
    };
  });

  expect(sizes.find.height).toBeLessThanOrEqual(40);
  expect(sizes.create.height).toBeLessThanOrEqual(40);
  expect(sizes.find.width).toBeLessThan(132);
  expect(sizes.create.width).toBeLessThan(112);
  expect(sizes.terminal.top).toBeGreaterThan(sizes.connectPanel.bottom);
});

test("active terminal survives runtime rerender and keeps input wired", async ({ page }) => {
  await openShell(page);

  await page.locator("#btn-shell-connect").click();
  await expectCreateTaskCall(page, "sh_attach", {
    peer_id: PEERS.member,
    target: "local",
    session: "main",
  });
  await expect(page.locator("#shell-status")).toContainText("Connected");
  await expect(page.locator("#terminal .xterm")).toBeVisible();
  await expect.poll(() => page.locator("#terminal .xterm-viewport").evaluate((node) => getComputedStyle(node).overflowY)).toBe("scroll");

  await page.evaluate(() => {
    const ws = window.__miopunchWebSockets && window.__miopunchWebSockets[0];
    if (!ws || typeof ws.onmessage !== "function") throw new Error("missing fake websocket");
    ws.onmessage({ data: "__MIO_REMOTE_TO_UI__\r\n" });
  });
  await expect(page.locator("#terminal .xterm")).toHaveAttribute("data-test-output", /__MIO_REMOTE_TO_UI__/);

  const binarySendCount = async () => (await calls(page))
    .filter((call) => call.method === "WebSocket.send" && String(call.data || "").startsWith("[binary:")).length;
  const sendsBefore = await binarySendCount();
  await emitRuntime(page, "desktop:state", {
    kind: "shell_sessions.replace",
    base_rev: 0,
    rev: 1,
    shell_sessions: [{
      task_id: "runtime-shell-attach",
      peer_id: PEERS.member,
      target: "local",
      session: "main",
      status: "running",
      stage: "attached",
      created_at: "2026-05-12T14:33:15Z",
      report_ready: false,
      attachable: true,
    }],
  });

  await expect(page.locator("#terminal .xterm")).toBeVisible();
  await expect(page.locator("#terminal .xterm")).toHaveAttribute("data-test-output", /__MIO_REMOTE_TO_UI__/);
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "Terminal.focus").length).toBeGreaterThan(1);

  await page.keyboard.type("x");
  await expect.poll(binarySendCount).toBeGreaterThan(sendsBefore);
});

test("existing shell sessions can be selected and resumed without creating a new attach task", async ({ page }) => {
  await openShell(page, {
    initialShellSessions: [
      {
        task_id: "resume-shell-main",
        peer_id: PEERS.member,
        target: "local",
        session: "main",
        status: "running",
        stage: "attached",
        created_at: "2026-05-12T14:33:15Z",
        report_ready: false,
        attachable: true,
      },
      {
        task_id: "resume-shell-maintenance",
        peer_id: PEERS.member,
        target: "local",
        session: "maintenance",
        status: "running",
        stage: "attached",
        created_at: "2026-05-12T14:35:15Z",
        report_ready: false,
        attachable: true,
      },
    ],
  });

  await expect(page.locator(".shell-live-panel")).toBeVisible();
  await expect(page.locator(".shell-live-title")).toContainText("Live sessions");
  await expect(page.locator(".shell-live-chip[data-shell-session-task]")).toHaveCount(2);
  const mainSession = page.locator('.shell-live-chip[data-shell-session-task="resume-shell-main"]');
  await mainSession.click();
  await expect.poll(async () => (await createTaskCalls(page, "sh_attach")).length).toBe(0);
  await expect.poll(async () => (await calls(page)).some((call) =>
    call.method === "WebSocket" && String(call.url || "").includes("resume-shell-main")
  )).toBe(true);
  await expect(page.locator("#shell-status")).toContainText("Resumed local/main");
});

test("non-attachable shell sessions stay visible but cannot resume", async ({ page }) => {
  await openShell(page, {
    initialShellSessions: [
      {
        task_id: "detached-shell-main",
        peer_id: PEERS.member,
        target: "local",
        session: "main",
        status: "running",
        stage: "detached",
        created_at: "2026-05-12T14:33:15Z",
        report_ready: false,
        attachable: false,
      },
    ],
  });

  await expect(page.locator('.shell-live-chip[data-shell-session-task="detached-shell-main"]')).toBeVisible();
  await expect(page.locator("#btn-shell-resume")).toHaveCount(0);
  await page.locator('.shell-live-chip[data-shell-session-task="detached-shell-main"]').click();
  await expect(page.locator("#shell-error")).toContainText("not attachable");
  await expect(page.locator("#btn-shell-connect")).toBeEnabled();
});

test("terminal bridge waits for first remote output before marking shell connected", async ({ page }) => {
  await openShell(page, {
    webSocketModes: ["open_no_data"],
  });

  await page.locator("#btn-shell-connect").click();
  await expectCreateTaskCall(page, "sh_attach", {
    peer_id: PEERS.member,
    target: "local",
    session: "main",
  });
  await expect(page.locator("#shell-phase")).toHaveText("connecting");
  await expect(page.locator("#shell-status")).toContainText("Waiting for remote shell output");
  await expect(page.locator("#terminal .xterm")).toHaveAttribute("data-test-output", /Connecting/);
  await expect(page.locator("[data-shell-disconnect]").first()).toBeEnabled();
  await expect(page.locator(".shell-connect-panel").getByRole("button", { name: "Disconnect" })).toBeVisible();
});

test("shell discovery failure stays visible and retryable", async ({ page }) => {
  await openShell(page, {
    createTaskModes: { sh_ls: ["failure", "success"] },
  });
  await openShellOptions(page);

  await page.getByRole("button", { name: "Find targets" }).click();
  await expect(page.locator("#shell-error")).toContainText("Shell lookup failed:");
  await expect(page.getByRole("button", { name: "Find targets" })).toBeEnabled();

  await page.getByRole("button", { name: "Find targets" }).click();
  await expect.poll(async () => (await createTaskCalls(page, "sh_ls")).length).toBe(2);
  await expect.poll(() => datalistOptions(page, "#shell-target-options")).toEqual(expect.arrayContaining(["local"]));
  await expect(page.locator("#shell-phase")).toHaveText("idle");
});

test("shell attach creation failure keeps values available for retry", async ({ page }) => {
  await openShell(page, {
    createTaskModes: { sh_attach: ["failure", "success"] },
  });
  await openShellOptions(page);

  await page.locator("#shell-target").fill("ssh:ops");
  await page.locator("#btn-shell-connect").click();

  await expect(page.locator("#shell-error")).toContainText("Connect failed:");
  await expect(page.locator("#shell-target")).toHaveValue("ssh:ops");
  await expect(page.locator("#btn-shell-connect")).toBeEnabled();

  await page.locator("#btn-shell-connect").click();
  await expectCreateTaskCall(page, "sh_attach", {
    peer_id: PEERS.member,
    target: "ssh:ops",
    session: "main",
  });
  await expect.poll(async () => (await createTaskCalls(page, "sh_attach")).length).toBe(2);
  await expect(page.locator("#shell-status")).toContainText("Connected");
});

test("terminal bridge setup failure remains retryable", async ({ page }) => {
  await openShell(page, {
    terminalBridgeInfoModes: ["failure", "success"],
  });

  await page.locator("#btn-shell-connect").click();
  await expect(page.locator("#shell-error")).toContainText("Connect failed:");
  await expect(page.locator('.shell-live-chip[data-shell-session-task]')).toBeVisible();

  await page.getByRole("button", { name: "Resume", exact: true }).click();
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "TerminalBridgeInfo").length).toBe(2);
  await expect(page.locator("#shell-status")).toContainText("Resumed");
});

test("terminal websocket close leaves reconnect available", async ({ page }) => {
  await openShell(page, {
    webSocketModes: ["close_after_open", "success"],
  });

  await page.locator("#btn-shell-connect").click();
  await expect(page.locator("#shell-error")).toContainText("Disconnected:");
  await expect(page.locator("#shell-phase")).toHaveText("disconnected");
  await expect(page.locator("#btn-shell-connect")).toBeEnabled();

  await page.locator("#btn-shell-connect").click();
  await expect.poll(async () => (await createTaskCalls(page, "sh_attach")).length).toBe(2);
  await expect(page.locator("#shell-status")).toContainText("Connected");
});

test("terminal websocket close prefers final shell diagnostics when task output is available", async ({ page }) => {
  await openShell(page, {
    webSocketModes: [
      {
        mode: "close_after_open",
        taskResult: {
          stage: "SessionAttach",
          reason_code: "SH_CONNECTOR_FAIL",
          exit_code: 69,
          report_ready: true,
          facts: [
            { term_id: "shell_layer", message: "shell_layer=ssh" },
            { term_id: "shell_close", message: "shell_close=ssh process exited: process exited: 255" },
          ],
          suggestions: [{ message: "retry" }],
        },
      },
      "success",
    ],
  });

  await page.locator("#btn-shell-connect").click();
  await expect(page.locator("#shell-error")).toContainText("Disconnected: SH_CONNECTOR_FAIL: ssh process exited: process exited: 255");
  await expect(page.locator("#shell-phase")).toHaveText("disconnected");
  await expect(page.locator("#shell-target")).toHaveValue("local");
  await expect(page.locator("#shell-session")).toHaveValue("main");
  await expect(page.locator("#btn-shell-connect")).toBeEnabled();

  await page.locator("#btn-shell-connect").click();
  await expect.poll(async () => (await createTaskCalls(page, "sh_attach")).length).toBe(2);
  await expect(page.locator("#shell-status")).toContainText("Connected");
});
