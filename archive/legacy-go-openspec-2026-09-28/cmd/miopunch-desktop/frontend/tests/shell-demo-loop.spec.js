const { calls, closeShell, expect, openDesktop, shellLog, snapshotFor, test } = require("./support/desktop");

test("shell attach stays gated on ping success and bridges with shell_session_id", async ({ page }) => {
  await openDesktop(page, { snapshot: snapshotFor("SecureSession") });

  await page.getByRole("button", { name: "Shell", exact: true }).click();
  await expect(page.locator("#topbar-title")).toHaveText("Shell");
  await expect(page.locator("#stage-chip")).toHaveText("Stage SecureSession");
  await page.locator("[data-shell-peer]").first().click();
  await expect(page.locator("#shell-status")).toContainText("Run Ping first");
  await expect(page.locator("#btn-ping")).toBeVisible();
  await expect(page.locator("#btn-shell-connect")).toHaveCount(0);

  await page.locator("#btn-ping").click();

  await expect.poll(async () => {
    const runtimeCalls = (await calls(page)).filter((call) => call.method === "RuntimeAction");
    return runtimeCalls.some((call) => call.action === "ping");
  }).toBe(true);

  await expect(page.locator("#btn-shell-connect")).toBeEnabled();
  await page.locator("#btn-shell-connect").click();

  await expect.poll(async () => shellLog(page)).toContainEqual(expect.objectContaining({
    type: "open",
    sessionID: "shell-session-01",
    url: expect.stringContaining("/api/v1/shell/shell-session-01/ws?token=ui-test-token"),
  }));

  await expect.poll(async () => shellLog(page)).toContainEqual(expect.objectContaining({
    type: "message",
    sessionID: "shell-session-01",
    data: "welcome to miopunch shell\n",
  }));
});

test("shell actions pass the selected p2p path policy", async ({ page }) => {
  await openDesktop(page, { snapshot: snapshotFor("SecureSession") });

  await page.getByRole("button", { name: "Shell", exact: true }).click();
  await page.locator("[data-shell-peer]").first().click();
  await page.locator('[data-p2p-policy="p2p_network"]').selectOption("udp_only");
  await page.locator('[data-p2p-policy="p2p_ip_family"]').selectOption("v4");

  await page.locator("#btn-ping").click();
  await expectRuntimeAction(page, "ping", {
    p2p_network: "udp_only",
    p2p_ip_family: "v4",
  });

  await page.locator("#btn-shell-discover").click();
  await expectRuntimeAction(page, "sh_ls", {
    target: "",
    p2p_network: "udp_only",
    p2p_ip_family: "v4",
  });

  await page.locator("#btn-shell-find-sessions").click();
  await expectRuntimeAction(page, "sh_ls", {
    target: "local",
    p2p_network: "udp_only",
    p2p_ip_family: "v4",
  });

  await expect(page.locator("#btn-shell-connect")).toBeEnabled();
  await page.locator("#btn-shell-connect").click();
  await expectRuntimeAction(page, "sh", {
    target: "local",
    session: "main",
    p2p_network: "udp_only",
    p2p_ip_family: "v4",
  });
});

test("remote shell exit closes without showing a websocket error", async ({ page }) => {
  await openDesktop(page, { snapshot: snapshotFor("SecureSession") });

  await openLiveShellAfterPing(page);
  await closeShell(page, "shell-session-01", {
    control: { op: "shell_exit", ok: true },
    code: 1006,
  });

  await expect(page.locator("#shell-phase")).toHaveText("disconnected");
  await expect(page.locator("#shell-status")).toContainText("Remote shell exited.");
  await expect(page.locator("#shell-error")).toBeHidden();
});

test("unexpected shell websocket close remains visible as an error", async ({ page }) => {
  await openDesktop(page, { snapshot: snapshotFor("SecureSession") });

  await openLiveShellAfterPing(page);
  await closeShell(page, "shell-session-01", { code: 1006 });

  await expect(page.locator("#shell-phase")).toHaveText("disconnected");
  await expect(page.locator("#shell-error")).toContainText("Disconnected: websocket closed (1006)");
});

async function openLiveShellAfterPing(page) {
  await page.getByRole("button", { name: "Shell", exact: true }).click();
  await page.locator("[data-shell-peer]").first().click();
  await page.locator("#btn-ping").click();
  await expect(page.locator("#btn-shell-connect")).toBeEnabled();
  await page.locator("#btn-shell-connect").click();
  await expect.poll(async () => shellLog(page)).toContainEqual(expect.objectContaining({
    type: "open",
    sessionID: "shell-session-01",
  }));
  await expect.poll(async () => shellLog(page)).toContainEqual(expect.objectContaining({
    type: "message",
    sessionID: "shell-session-01",
  }));
}

async function expectRuntimeAction(page, action, expectedArgs) {
  await expect.poll(async () => {
    const runtimeCalls = (await calls(page)).filter((call) => call.method === "RuntimeAction" && call.action === action);
    return runtimeCalls.some((call) => Object.entries(expectedArgs).every(([key, value]) => String(call.args && call.args[key] || "") === String(value)));
  }).toBe(true);
}
