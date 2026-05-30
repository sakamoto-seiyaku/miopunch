const { emitRuntime, expect, openDesktop, snapshotFor, test } = require("./support/desktop");

test("desktop runtime snapshot events refresh the active console view while preserving runtime stage state", async ({ page }) => {
  await openDesktop(page, { snapshot: snapshotFor("Enroll") });

  await emitRuntime(page, "desktop:state", {
    kind: "snapshot.updated",
    snapshot: snapshotFor("Punch", {
      summary: { text: "payload exchange is running" },
      evidence: {
        facts: [{ message: "attempt=1" }],
        suggestions: [{ message: "wait for the secure-session gate" }],
      },
    }),
  });

  await expect(page.locator("#topbar-title")).toHaveText("Network");
  await expect(page.locator("#stage-chip")).toHaveText("Stage Punch");
  await expect(page.getByText("payload exchange is running")).toBeVisible();
  await expect(page.getByText("attempt=1")).toBeVisible();
  await expect(page.getByText("wait for the secure-session gate")).toBeVisible();
});

test("manual tab selection stays pinned across snapshot refreshes", async ({ page }) => {
  await openDesktop(page, { snapshot: snapshotFor("Enroll") });

  await page.getByRole("button", { name: "Admin" }).click();
  await expect(page.locator("#topbar-title")).toHaveText("Admin");

  await emitRuntime(page, "desktop:state", {
    kind: "snapshot.updated",
    snapshot: snapshotFor("Punch", {
      summary: { text: "payload exchange is running" },
      evidence: {
        facts: [{ message: "attempt=1" }],
        suggestions: [{ message: "wait for the secure-session gate" }],
      },
    }),
  });

  await expect(page.locator("#topbar-title")).toHaveText("Admin");
  await expect(page.locator("#stage-chip")).toHaveText("Stage Punch");
  await expect(page.getByText("payload exchange is running")).toBeVisible();
  await expect(page.getByText("attempt=1")).toBeVisible();
});

test("runtime transport events and connection events re-render the active view", async ({ page }) => {
  await openDesktop(page, { snapshot: snapshotFor("Network") });

  await emitRuntime(page, "desktop:runtime", {
    kind: "stream_retrying",
    error: {
      stage: "desktop",
      reason_code: "UNAVAILABLE",
      exit_code: 69,
      message: "event stream retrying",
    },
  });
  await expect(page.getByText("UNAVAILABLE: event stream retrying")).toBeVisible();

  await emitRuntime(page, "localapi:connection", {
    connected: true,
    selected: "override",
    addr: "unix:/tmp/custom-localapi.sock",
    user_addr: "unix:/tmp/miopunch-localapi.sock",
    override_addr: "unix:/tmp/custom-localapi.sock",
    desktop_managed: false,
  });

  await expect(page.locator("#connection-chip")).toHaveText("Connected via override");
  await expect(page.getByText("unix:/tmp/custom-localapi.sock")).toBeVisible();
});

test("connection failure facts are rendered for diagnostics", async ({ page }) => {
  await openDesktop(page, {
    connection: {
      connected: false,
      selected: "none",
      failure: {
        stage: "Enroll",
        reason_code: "TIMEOUT",
        exit_code: 70,
        message: "timed out waiting for enroll response",
        facts: [
          { message: "broker_endpoint=tcp://203.0.113.10:1883" },
          { message: "join_topic=mp/v1/join/net-01" },
          { message: "reply_topic=mp/v1/reply/peer-a/msg-01" },
        ],
      },
    },
  });

  await expect(page.locator(".helper.helper-error").filter({ hasText: "TIMEOUT: timed out waiting for enroll response" }).first()).toBeVisible();
  await expect(page.getByText("broker_endpoint=tcp://203.0.113.10:1883")).toBeVisible();
  await expect(page.getByText("join_topic=mp/v1/join/net-01")).toBeVisible();
  await expect(page.getByText("reply_topic=mp/v1/reply/peer-a/msg-01")).toBeVisible();
});
