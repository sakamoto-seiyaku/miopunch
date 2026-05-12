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
  await page.getByRole("button", { name: "Shell" }).click();
  await expect(page.locator("#shell-form")).toBeVisible();
}

test("shell demo loop discovers choices, connects, disconnects, and reconnects", async ({ page }) => {
  await openShell(page);

  await expect(page.locator("#shell-target")).toHaveValue("local");
  await expect(page.locator("#shell-session")).toHaveValue("main");

  await page.getByRole("button", { name: "Discover" }).click();
  await expectCreateTaskCall(page, "sh_ls", { peer_id: PEERS.member, target: "" });
  await expect(page.locator("#shell-target-choices")).toContainText("local");
  await expect(page.locator("#shell-target-choices")).toContainText("ssh:ops");

  await page.getByRole("button", { name: "Discover" }).click();
  await expectCreateTaskCall(page, "sh_ls", { peer_id: PEERS.member, target: "local" });
  await expect(page.locator("#shell-session-choices")).toContainText("main");
  await expect(page.locator("#shell-session-choices")).toContainText("maintenance");

  await page.locator("#btn-shell-connect").click();
  await expectCreateTaskCall(page, "sh_attach", {
    peer_id: PEERS.member,
    target: "local",
    session: "main",
  });
  await expect(page.locator("#shell-status")).toContainText("Connected");
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "Terminal.focus").length).toBeGreaterThan(0);

  await page.getByRole("button", { name: "Disconnect" }).click();
  await expect(page.locator("#shell-phase")).toHaveText("disconnected");
  await expect(page.locator("#shell-error")).toBeHidden();
  await expect(page.locator("#shell-target")).toHaveValue("local");
  await expect(page.locator("#shell-session")).toHaveValue("main");
  await expect(page.locator("#btn-shell-connect")).toBeEnabled();
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "WebSocket.close").length).toBeGreaterThan(0);

  await page.locator("#btn-shell-connect").click();
  await expect.poll(async () => (await createTaskCalls(page, "sh_attach")).length).toBe(2);
  await expect(page.locator("#shell-status")).toContainText("Connected");
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
    }],
  });

  await expect(page.locator("#terminal .xterm")).toBeVisible();
  await expect(page.locator("#terminal .xterm")).toHaveAttribute("data-test-output", /__MIO_REMOTE_TO_UI__/);
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "Terminal.focus").length).toBeGreaterThan(1);

  await page.keyboard.type("x");
  await expect.poll(binarySendCount).toBeGreaterThan(sendsBefore);
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
  await expect(page.locator("#shell-status")).toContainText("Waiting for shell output");
  await expect(page.locator("#terminal .xterm")).toHaveAttribute("data-test-output", /Connecting/);
  await expect(page.locator("#btn-shell-disconnect")).toBeEnabled();
});

test("shell discovery failure stays visible and retryable", async ({ page }) => {
  await openShell(page, {
    createTaskModes: { sh_ls: ["failure", "success"] },
  });

  await page.getByRole("button", { name: "Discover" }).click();
  await expect(page.locator("#shell-error")).toContainText("Discover failed:");
  await expect(page.getByRole("button", { name: "Discover" })).toBeEnabled();

  await page.getByRole("button", { name: "Discover" }).click();
  await expect.poll(async () => (await createTaskCalls(page, "sh_ls")).length).toBe(2);
  await expect(page.locator("#shell-target-choices")).toContainText("local");
  await expect(page.locator("#shell-phase")).toHaveText("idle");
});

test("shell attach creation failure keeps values available for retry", async ({ page }) => {
  await openShell(page, {
    createTaskModes: { sh_attach: ["failure", "success"] },
  });

  await page.locator("#shell-session").fill("demo-loop");
  await page.locator("#btn-shell-connect").click();

  await expect(page.locator("#shell-error")).toContainText("Connect failed:");
  await expect(page.locator("#shell-session")).toHaveValue("demo-loop");
  await expect(page.locator("#btn-shell-connect")).toBeEnabled();

  await page.locator("#btn-shell-connect").click();
  await expectCreateTaskCall(page, "sh_attach", {
    peer_id: PEERS.member,
    target: "local",
    session: "demo-loop",
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
  await expect(page.locator("#btn-shell-connect")).toBeEnabled();

  await page.locator("#btn-shell-connect").click();
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "TerminalBridgeInfo").length).toBe(2);
  await expect(page.locator("#shell-status")).toContainText("Connected");
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
