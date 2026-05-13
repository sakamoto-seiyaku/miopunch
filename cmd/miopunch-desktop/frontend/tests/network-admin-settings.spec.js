const {
  PEERS,
  calls,
  expect,
  expectCreateTaskCall,
  openDesktop,
  test,
} = require("./support/desktop");

async function openPeer(page, peerID) {
  await page.locator(`[data-open-peer="${peerID}"]`).last().click();
  await expect(page.locator(`[data-copy-peer="${peerID}"]`)).toBeVisible();
}

async function openMember(page, peerID) {
  await page.locator(`[data-open-member="${peerID}"]`).last().click();
  await expect(page.getByRole("button", { name: "Revoke" })).toBeVisible();
}

async function openAdmin(page) {
  await openDesktop(page);
  await page.getByRole("button", { name: "Admin", exact: true }).click();
  await expect(page.locator(".page-title")).toHaveText("Governance");
}

test("Network peer actions are enabled only for valid remote peers", async ({ page }) => {
  await openDesktop(page);

  await openPeer(page, PEERS.owner);
  await expect(page.getByRole("button", { name: "Ping" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "List sessions" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Shell" })).toBeDisabled();

  await page.locator(`[data-open-peer="${PEERS.revoked}"]`).click();
  await expect(page.locator(`[data-copy-peer="${PEERS.revoked}"]`)).toBeVisible();
  await expect(page.getByRole("button", { name: "Ping" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "List sessions" })).toBeDisabled();
  await expect(page.getByRole("button", { name: "Shell" })).toBeDisabled();

  await page.locator(`[data-open-peer="${PEERS.member}"]`).click();
  await expect(page.locator(`[data-copy-peer="${PEERS.member}"]`)).toBeVisible();
  await expect(page.getByRole("button", { name: "Ping" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "List sessions" })).toBeEnabled();
  await expect(page.getByRole("button", { name: "Shell" })).toBeEnabled();
});

test("Network peer actions create expected task calls", async ({ page }) => {
  await openDesktop(page);
  await openPeer(page, PEERS.member);

  await page.getByRole("button", { name: "Ping" }).click();
  await expectCreateTaskCall(page, "ping", { peer_id: PEERS.member });
  await expect(page.getByText("payload exchanged")).toBeVisible();

  await page.getByRole("button", { name: "List sessions" }).click();
  await expectCreateTaskCall(page, "sh_ls", { peer_id: PEERS.member, target: "" });
  await expect(page.getByText("targets listed")).toBeVisible();
});

test("Network shell flow creates sh_attach and opens the terminal bridge", async ({ page }) => {
  await openDesktop(page);
  await openPeer(page, PEERS.member);

  await page.getByRole("button", { name: "Shell" }).click();
  await expect(page.locator(".page-title")).toContainText("peer-livingr");
  await expect(page.locator("#shell-form")).toBeVisible();

  await page.locator("#shell-form").getByRole("button", { name: "Connect" }).click();

  await expectCreateTaskCall(page, "sh_attach", {
    peer_id: PEERS.member,
    target: "local",
    session: "main",
  });
  await expect.poll(() => calls(page)).toContainEqual({ method: "TerminalBridgeInfo" });
  await expect(page.locator("#shell-status")).toContainText(/Connected|connected|task=/);
});

test("Admin revoke is enabled only for revocable members", async ({ page }) => {
  await openAdmin(page);

  await openMember(page, PEERS.owner);
  await expect(page.getByRole("button", { name: "Revoke" })).toBeDisabled();

  await page.locator(`[data-open-member="${PEERS.admin}"]`).click();
  await expect(page.getByRole("button", { name: "Revoke" })).toBeDisabled();

  await page.locator(`[data-open-member="${PEERS.revoked}"]`).click();
  await expect(page.getByRole("button", { name: "Revoke" })).toBeDisabled();

  await page.locator(`[data-open-member="${PEERS.member}"]`).click();
  await expect(page.getByRole("button", { name: "Revoke" })).toBeEnabled();
});

test("Admin revoke creates the expected dangerous task call", async ({ page }) => {
  await openAdmin(page);
  await openMember(page, PEERS.member);

  await page.getByRole("button", { name: "Revoke" }).click();

  await expectCreateTaskCall(page, "revoke_member", { peer_id: PEERS.member, dangerous: true });
  await expect(page.getByText("decl written")).toBeVisible();
});

test("Settings Local daemon apply and clear call bridge methods", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=settings&section=localapi" });

  await expect(page.getByRole("heading", { name: "Local daemon" })).toBeVisible();
  await page.locator("#localapi-override").fill("unix:/tmp/miopunch-ui-test-override.sock");
  await page.getByRole("button", { name: "Apply" }).click();

  await expect.poll(() => calls(page)).toContainEqual({
    method: "SetLocalAPIOverride",
    addr: "unix:/tmp/miopunch-ui-test-override.sock",
  });
  await expect(page.locator("#localapi-known")).toContainText("override=unix:/tmp/miopunch-ui-test-override.sock");

  await page.getByRole("button", { name: "Clear" }).click();
  await expect.poll(() => calls(page)).toContainEqual({ method: "ClearLocalAPIOverride" });
});

test("Settings Runtime config saves through the desktop bridge", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=settings&section=runtime" });

  await expect(page.locator(".page-title")).toHaveText("Runtime config");
  await page.locator("#settings-mqtt-brokers").fill("broker-a:1883, broker-b:1883");
  await page.locator("#settings-p2p-network").selectOption("tcp_only");
  await page.locator("#settings-log-level").selectOption("debug");
  await page.getByRole("button", { name: "Save" }).click();

  await expect.poll(() => calls(page)).toContainEqual(expect.objectContaining({
    method: "SaveDesktopConfig",
    update: expect.objectContaining({
      runtime: expect.objectContaining({
        mqtt_brokers: ["broker-a:1883", "broker-b:1883"],
        p2p_network: "tcp_only",
      }),
      preferences: expect.objectContaining({ log_level: "debug" }),
    }),
  }));
  await expect(page.locator("#toast")).toHaveText("Runtime config saved");
  await expect(page.getByText("broker-a:1883, broker-b:1883")).toBeVisible();
});

test("Settings Runtime config shows validation suggestions after rejected save", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=settings&section=runtime" });

  await page.locator("#settings-mqtt-brokers").fill("broker-without-port");
  await page.getByRole("button", { name: "Save" }).click();

  await expect.poll(() => calls(page)).toContainEqual(expect.objectContaining({
    method: "SaveDesktopConfig",
    update: expect.objectContaining({
      runtime: expect.objectContaining({
        mqtt_brokers: ["broker-without-port"],
      }),
    }),
  }));
  await expect(page.locator("#runtime-config-form").getByText("Save failed: BAD_REQUEST: invalid MQTT brokers")).toBeVisible();
  await expect(page.getByText("use host:port")).toBeVisible();
  await expect(page.getByRole("button", { name: "Save" })).toBeEnabled();
});

test("Settings Diagnostics renders disconnected LocalAPI guidance", async ({ page }) => {
  await openDesktop(page, { fixture: "disconnected", path: "/?tab=settings&section=diagnostics" });

  await expect(page.locator(".page-title")).toHaveText("Diagnostics");
  await expect(page.getByText("retry desktop connection")).toBeVisible();
  await expect(page.getByText("reason_code=daemon_not_running")).toBeVisible();
});

test("Settings Diagnostics exports runtime diagnostics", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=settings&section=diagnostics" });

  await page.getByRole("button", { name: "Export diagnostics" }).click();

  await expect.poll(() => calls(page)).toContainEqual({ method: "ExportDiagnostics" });
  await expect(page.getByText("/tmp/miopunch-diagnostics.zip")).toBeVisible();
});

test("Settings Diagnostics renders desktop-managed session bootstrap facts", async ({ page }) => {
  await openDesktop(page, { fixture: "desktop-managed", path: "/?tab=settings&section=diagnostics" });

  await expect(page.getByText("desktop_managed=desktop-managed")).toBeVisible();
  await expect(page.getByText("bootstrap_state=ready")).toBeVisible();
  await expect(page.getByText("bootstrap_stage=ready")).toBeVisible();
});

test("Settings Diagnostics renders bootstrap failure guidance", async ({ page }) => {
  await openDesktop(page, { fixture: "bootstrap-failure", path: "/?tab=settings&section=diagnostics" });

  await expect(page.getByText("same-user session daemon bootstrap failed")).toBeVisible();
  await expect(page.getByText("check that ./miopunch is next to ./miopunch-desktop and executable")).toBeVisible();
  await expect(page.getByText("bootstrap_state=failed")).toBeVisible();
});

test("Settings explicit Quit calls bridge method", async ({ page }) => {
  await openDesktop(page, { path: "/?tab=settings" });

  await page.getByRole("button", { name: "Quit" }).click();

  await expect.poll(() => calls(page)).toContainEqual({ method: "Quit" });
});

test("Refresh triggers a new snapshot load", async ({ page }) => {
  await openDesktop(page);

  await page.getByRole("button", { name: "Refresh" }).click();

  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "DesktopRuntimeResync").length).toBeGreaterThan(0);
});

test("Refresh reconnects before loading snapshot when bridge is disconnected", async ({ page }) => {
  await openDesktop(page, { fixture: "reconnect-on-refresh" });

  await page.getByRole("button", { name: "Refresh" }).click();

  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "DesktopRuntimeStart").length).toBeGreaterThan(1);
});
