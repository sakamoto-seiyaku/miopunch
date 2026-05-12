const {
  calls,
  createTaskCalls,
  emitRuntime,
  expect,
  expectCreateTaskCall,
  inviteCode,
  openDesktop,
  test,
} = require("./support/desktop");

async function openAccessFlow(page, flow, options = {}) {
  await openDesktop(page, { ...options, path: `/?tab=access&flow=${flow}` });
  const title = flow === "invite" ? "Create invite" : flow === "approve" ? "Approve request" : "Join network";
  await expect(page.getByRole("heading", { name: title })).toBeVisible();
}

test("Access Join submits object args and exports the completed report", async ({ page }) => {
  await openAccessFlow(page, "join");

  await page.locator("#join-code").fill("mp:v0.join.test");
  await page.locator("#join-form").getByRole("button", { name: "Join" }).click();

  await expectCreateTaskCall(page, "join", { code: "mp:v0.join.test" });
  await expect(page.getByText("membership accepted")).toBeVisible();
  await expect(page.getByRole("button", { name: "Export report" })).toBeEnabled();

  await page.getByRole("button", { name: "Export report" }).click();

  await expect(page.locator("#join-report-path")).toContainText("/tmp/ui-join-001.md");
  await expect.poll(() => calls(page)).toContainEqual({ method: "ExportTaskReport", taskID: "ui-join-001" });
});

test("Access Join validates missing invite code without creating a task", async ({ page }) => {
  await openAccessFlow(page, "join");

  await page.locator("#join-form").getByRole("button", { name: "Join" }).click();

  await expect(page.locator("#toast")).toContainText("Missing invite code");
  await expect.poll(() => createTaskCalls(page, "join")).toEqual([]);
});

test("Access invite Create calls bridge with object args and renders code", async ({ page }) => {
  await openAccessFlow(page, "invite");

  await page.getByRole("button", { name: "Create" }).click();

  await expect(page.locator("#invite-code")).toHaveValue(inviteCode);
  await expect(page.getByRole("button", { name: "Copy" })).toBeEnabled();
  await expect(page.locator("#invite-qr svg")).toHaveCount(1);
  await expectCreateTaskCall(page, "invite", {});
});

test("Access invite Create waits for delayed task output", async ({ page }) => {
  await openAccessFlow(page, "invite", { inviteCodeDelivery: "delayed-fetch" });

  await page.getByRole("button", { name: "Create" }).click();

  await expect(page.locator("#invite-code")).toHaveValue(inviteCode);
  await expect(page.getByRole("button", { name: "Copy" })).toBeEnabled();
  await expect(page.locator("#invite-qr svg")).toHaveCount(1);
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "GetTask").length).toBeGreaterThanOrEqual(2);
});

test("Access invite Create fetches code after partial done task response", async ({ page }) => {
  await openAccessFlow(page, "invite", { inviteCodeDelivery: "partial-done-fetch" });

  await page.getByRole("button", { name: "Create" }).click();

  await expect(page.locator("#invite-code")).toHaveValue(inviteCode);
  await expect(page.locator("#invite-hint")).not.toContainText("no invite code");
  await expect(page.getByRole("button", { name: "Copy" })).toBeEnabled();
  await expect(page.locator("#invite-qr svg")).toHaveCount(1);
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "GetTask").length).toBeGreaterThanOrEqual(1);
});

test("Access invite Create fetches code after partial done runtime events", async ({ page }) => {
  await openAccessFlow(page, "invite", { inviteCodeDelivery: "partial-event-fetch" });

  await page.getByRole("button", { name: "Create" }).click();
  await emitRuntime(page, "desktop:state", {
    kind: "task.upsert",
    base_rev: 0,
    rev: 1,
    task: {
      task_id: "ui-invite-001",
      kind: "invite",
      status: "running",
      stage: "prepare invite code",
      facts: [{ term_id: "peer_id", message: "peer_id=peer-ui-test-owner" }],
      suggestions: [],
    },
  });
  await emitRuntime(page, "desktop:state", {
    kind: "task.upsert",
    base_rev: 1,
    rev: 2,
    task: {
      task_id: "ui-invite-001",
      kind: "invite",
      status: "done",
      stage: "invite code ready",
      reason_code: "OK",
      exit_code: 0,
      report_ready: true,
      facts: [{ term_id: "peer_id", message: "peer_id=peer-ui-test-owner" }],
      suggestions: [],
    },
  });

  await expect(page.locator("#invite-code")).toHaveValue(inviteCode);
  await expect(page.locator("#invite-hint")).not.toContainText("no invite code");
  await expect(page.getByRole("button", { name: "Copy" })).toBeEnabled();
  await expect(page.locator("#invite-qr svg")).toHaveCount(1);
  await expect.poll(async () => (await calls(page)).filter((call) => call.method === "GetTask").length).toBeGreaterThanOrEqual(2);
});

test("Access invite flow renders code from runtime fact event", async ({ page }) => {
  await openAccessFlow(page, "invite", { inviteCodeDelivery: "event" });

  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.locator("#invite-code")).toHaveValue("");

  await emitRuntime(page, "desktop:state", {
    kind: "task.upsert",
    base_rev: 0,
    rev: 1,
    task: {
      task_id: "ui-invite-001",
      kind: "invite",
      status: "running",
      stage: "prepare invite code",
      facts: [{ term_id: "invite_code", message: inviteCode }],
      suggestions: [],
    },
  });

  await expect(page.locator("#invite-code")).toHaveValue(inviteCode);
  await expect(page.getByRole("button", { name: "Copy" })).toBeEnabled();
});

test("Access invite flow renders code from runtime task snapshot", async ({ page }) => {
  await openAccessFlow(page, "invite", { inviteCodeDelivery: "event" });

  await page.getByRole("button", { name: "Create" }).click();
  await emitRuntime(page, "desktop:state", {
    kind: "task.upsert",
    base_rev: 0,
    rev: 1,
    task: {
      task_id: "ui-invite-001",
      kind: "invite",
      status: "done",
      stage: "invite code ready",
      reason_code: "OK",
      exit_code: 0,
      report_ready: true,
      created_at: new Date().toISOString(),
      facts: [
        { term_id: "peer_id", message: "peer_id=peer-ui-test-owner" },
        { term_id: "invite_code", message: inviteCode },
      ],
      suggestions: [{ message: "on another machine: miopunch join <invite_code>" }],
    },
  });

  await expect(page.locator("#invite-code")).toHaveValue(inviteCode);
  await expect(page.locator("#invite-hint")).not.toContainText("no invite code");
  await expect(page.getByRole("button", { name: "Copy" })).toBeEnabled();
});

test("Access invite Create shows a diagnostic when task completes without code", async ({ page }) => {
  await openAccessFlow(page, "invite", { inviteCodeDelivery: "missing" });

  await page.getByRole("button", { name: "Create" }).click();

  await expect(page.locator("#invite-code")).toHaveValue("");
  await expect(page.locator("#invite-hint")).toContainText("no invite code");
  await expect(page.getByRole("button", { name: "Copy" })).toBeDisabled();
});

test("Access invite Create recovers from bridge failure", async ({ page }) => {
  await openAccessFlow(page, "invite", { createTaskModes: { invite: "failure" } });

  await page.getByRole("button", { name: "Create" }).click();

  await expect(page.locator("#invite-hint")).toContainText("Create failed");
  await expect(page.locator("#invite-hint")).toContainText("invite failed in fake bridge");
  await expect(page.getByRole("button", { name: "Create" })).toBeEnabled();
});

test("Access invite Create recovers from bridge timeout", async ({ page }) => {
  await openAccessFlow(page, "invite", { createTaskModes: { invite: "timeout" } });

  await page.getByRole("button", { name: "Create" }).click();

  await expect(page.locator("#invite-hint")).toContainText("Create invite timed out");
  await expect(page.getByRole("button", { name: "Create" })).toBeEnabled();
});

test("Access Approve submits object args and renders progress", async ({ page }) => {
  await openAccessFlow(page, "approve");

  await page.locator("#approve-code").fill("mp:v0.approve.test");
  await page.locator("#approve-form").getByRole("button", { name: "Start approval" }).click();

  await expectCreateTaskCall(page, "approve", { code: "mp:v0.approve.test", explicit_review: true });
  await expect(page.getByText("waiting for joiner")).toBeVisible();
});

test("Access Approve validates missing invite code without creating a task", async ({ page }) => {
  await openAccessFlow(page, "approve");

  await page.locator("#approve-form").getByRole("button", { name: "Start approval" }).click();

  await expect(page.locator("#toast")).toContainText("Missing invite code");
  await expect.poll(() => createTaskCalls(page, "approve")).toEqual([]);
});

const pendingApprovalRequest = {
  approve_task_id: "task-approve-ui-001",
  invite_id: "invite-ui-001",
  request_msg_id: "request-ui-001",
  member_peer_id: "peer-new-tablet-06",
  member_name: "New tablet",
  platform: "linux",
  v4_hint: "easy",
  status: "pending",
  created_at: "2026-05-04T00:02:00Z",
};

test("Access renders pending approval requests and submits approve decisions", async ({ page }) => {
  await openAccessFlow(page, "approve", { approvalRequests: [pendingApprovalRequest] });

  await expect(page.getByText("New tablet")).toBeVisible();
  await expect(page.locator(".approval-row", { hasText: "peer-new-tablet-06" })).toBeVisible();

  await page.getByRole("button", { name: "Approve", exact: true }).click();

  await expectCreateTaskCall(page, "approve_decision", {
    approve_task_id: "task-approve-ui-001",
    request_msg_id: "request-ui-001",
    decision: "approve",
  });
});

test("Access submits reject approval decisions", async ({ page }) => {
  await openAccessFlow(page, "approve", { approvalRequests: [pendingApprovalRequest] });

  await page.getByRole("button", { name: "Reject", exact: true }).click();

  await expectCreateTaskCall(page, "approve_decision", {
    approve_task_id: "task-approve-ui-001",
    request_msg_id: "request-ui-001",
    decision: "reject",
  });
});

test("Access hides approval decision controls for member role", async ({ page }) => {
  await openDesktop(page, { fixture: "member", path: "/?tab=access", approvalRequests: [pendingApprovalRequest] });

  await expect(page.getByRole("button", { name: "Approve", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Reject", exact: true })).toHaveCount(0);
});

test("Access approval requests update from runtime events", async ({ page }) => {
  await openAccessFlow(page, "approve", { approvalRequests: [pendingApprovalRequest] });

  await emitRuntime(page, "desktop:state", {
    kind: "approval_requests.replace",
    base_rev: 0,
    rev: 1,
    approval_requests: [{ ...pendingApprovalRequest, status: "approved", decision: "approve" }],
  });

  await expect(page.getByText("approved", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Approve", exact: true })).toBeDisabled();
});

test("Access approval decision bridge failures are visible and recoverable", async ({ page }) => {
  await openAccessFlow(page, "approve", {
    approvalRequests: [pendingApprovalRequest],
    createTaskModes: { approve_decision: "failure" },
  });

  await page.getByRole("button", { name: "Reject", exact: true }).click();

  await expect(page.locator(".helper-error", { hasText: "approve_decision failed in fake bridge" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Reject", exact: true })).toBeEnabled();
});

test("Access approval decision task failures are visible and recoverable", async ({ page }) => {
  await openAccessFlow(page, "approve", {
    approvalRequests: [pendingApprovalRequest],
    createTaskModes: { approve_decision: "task_failure" },
  });

  await page.getByRole("button", { name: "Approve", exact: true }).click();

  await expect(page.locator(".helper-error", { hasText: "approve_decision failed in fake task" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Approve", exact: true })).toBeEnabled();
});

test("Access hides admin-only flows for member role", async ({ page }) => {
  await openDesktop(page, { fixture: "member", path: "/?tab=access" });

  await expect(page.locator(".page-title")).toHaveText("Access");
  await expect(page.getByRole("button", { name: /Join network/ })).toBeVisible();
  await expect(page.getByRole("button", { name: /Create invite/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /Approve request/ })).toHaveCount(0);
});

test("Access shows setup flows for first-run empty node", async ({ page }) => {
  await openDesktop(page, { fixture: "empty", path: "/?tab=access" });

  await expect(page.locator(".page-title")).toHaveText("Access");
  await expect(page.locator('.grid [data-open-flow="join"]')).toBeVisible();
  await expect(page.locator('.grid [data-open-flow="invite"]')).toBeVisible();
  await expect(page.locator('.grid [data-open-flow="approve"]')).toBeVisible();
});

test("Access first-run invite Create calls bridge with object args", async ({ page }) => {
  await openAccessFlow(page, "invite", { fixture: "empty" });

  await page.getByRole("button", { name: "Create" }).click();

  await expect(page.locator("#invite-code")).toHaveValue(inviteCode);
  await expectCreateTaskCall(page, "invite", {});
});

test("Admin is available for first-run empty node", async ({ page }) => {
  await openDesktop(page, { fixture: "empty", path: "/?tab=admin" });

  await expect(page.getByRole("button", { name: "Admin", exact: true })).toBeVisible();
  await expect(page.locator(".page-title")).toHaveText("Governance");
  await expect(page.locator(".row-meta", { hasText: "peer-new-node-0000" })).toBeVisible();
  await expect(page.getByText("owner", { exact: true }).first()).toBeVisible();
});

test("Access invite Create is disabled when the desktop bridge is disconnected", async ({ page }) => {
  await openAccessFlow(page, "invite", { fixture: "disconnected" });

  await expect(page.getByRole("button", { name: "Create" })).toBeDisabled();
});
