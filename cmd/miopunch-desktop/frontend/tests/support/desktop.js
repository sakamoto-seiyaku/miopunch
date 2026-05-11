const base = require("@playwright/test");

const { expect } = base;

const PEERS = {
  owner: "peer-owner-zima-blue-0001",
  admin: "peer-studio-workstation-02",
  member: "peer-livingroom-mini-03",
  traveler: "peer-travel-laptop-04",
  revoked: "peer-old-phone-05",
};

const inviteCode = "miopunch1inviteuismoketest000000000000000000000000000000000000";

const clone = (value) => JSON.parse(JSON.stringify(value));

const test = base.test.extend({
  page: async ({ page }, use) => {
    const browserErrors = [];
    page.on("pageerror", (err) => {
      browserErrors.push(`pageerror: ${err.message}`);
    });
    page.on("console", (msg) => {
      if (msg.type() !== "error") return;
      const text = msg.text();
      if (/favicon\.ico/i.test(text)) return;
      browserErrors.push(`console error: ${text}`);
    });

    await use(page);

    expect(browserErrors).toEqual([]);
  },
});

function ownerTopology() {
  return {
    format: "miopunch.topology.ui-test",
    observed_at: "2026-05-04T00:00:00Z",
    self: { peer_id: PEERS.owner, role: "owner", v4_hint: "easy", v6_hint: "direct" },
    net: { net_id: "net_zima_blue_lab", brokers_effective: ["broker-1.miopunch.local:1883"] },
    state_head: { governance_head_b64: "gov_owner_head_test", decls_head_b64: "decls_owner_head_test" },
    members: [
      { peer_id: PEERS.owner, role: "owner", v4_hint: "easy", v6_hint: "direct" },
      { peer_id: PEERS.admin, role: "admin", v4_hint: "easy", v6_hint: "direct" },
      { peer_id: PEERS.member, role: "member", v4_hint: "portmap", v6_hint: "" },
      { peer_id: PEERS.traveler, role: "member", v4_hint: "hard", v6_hint: "" },
      { peer_id: PEERS.revoked, role: "member", v4_hint: "easy", v6_hint: "", revoked: true },
    ],
    presence: { online_window_sec: 120, hello_interval_sec: 30 },
    bootstrap: { recommendations: [], attempts: [], more_rounds: [] },
    neighbors: {
      target_k: 2,
      selected: [
        { peer_id: PEERS.admin, role: "admin", bucket: "easy", reason: "stable admin", dialable: true },
        { peer_id: PEERS.member, role: "member", bucket: "portmap", reason: "bucket diversity", dialable: true },
      ],
      active: [
        {
          peer_id: PEERS.admin,
          bucket: "easy",
          data_proto: "quic",
          path_family: "udp6",
          healthy: true,
          last_activity_unix_ms: 1777824000000,
        },
        {
          peer_id: PEERS.member,
          bucket: "portmap",
          data_proto: "kcp",
          path_family: "udp4",
          healthy: true,
          last_activity_unix_ms: 1777823980000,
        },
      ],
      unhealthy: [{ peer_id: PEERS.traveler, close_reason: "idle timeout" }],
      degree_distribution: [],
    },
    attempts: [],
    payloads: [],
    recovery: { events: [] },
  };
}

function memberTopology() {
  const top = ownerTopology();
  top.self = { peer_id: PEERS.traveler, role: "member", v4_hint: "hard", v6_hint: "" };
  top.members = [
    { peer_id: PEERS.owner, role: "owner", v4_hint: "easy", v6_hint: "direct" },
    { peer_id: PEERS.admin, role: "admin", v4_hint: "easy", v6_hint: "direct" },
    { peer_id: PEERS.member, role: "member", v4_hint: "portmap", v6_hint: "" },
    { peer_id: PEERS.traveler, role: "member", v4_hint: "hard", v6_hint: "" },
  ];
  return top;
}

function emptyTopology() {
  return {
    format: "miopunch.topology.ui-test",
    observed_at: "2026-05-04T00:00:00Z",
    self: { peer_id: "peer-new-node-0000", role: "unknown", v4_hint: "unknown", v6_hint: "" },
    net: { net_id: "", brokers_effective: [] },
    state_head: {},
    members: [],
    presence: {},
    bootstrap: { recommendations: [], attempts: [], more_rounds: [] },
    neighbors: { target_k: 0, selected: [], active: [], unhealthy: [], degree_distribution: [] },
    attempts: [],
    payloads: [],
    recovery: { events: [] },
  };
}

function fixtureData(name = "owner") {
  if (name === "member") return { connected: true, topology: memberTopology() };
  if (name === "empty") return { connected: true, topology: emptyTopology() };
  if (name === "selected-inactive") {
    const top = ownerTopology();
    top.neighbors.selected = [
      ...top.neighbors.selected,
      { peer_id: PEERS.traveler, role: "member", bucket: "hard", reason: "hard bucket coverage", dialable: true },
    ];
    top.neighbors.failures = [
      {
        peer_id: PEERS.traveler,
        bucket: "hard",
        stage: "peer_contact",
        reason_code: "UNAVAILABLE",
        retry_budget: 0,
        stop_condition: "dial_failed",
      },
    ];
    top.attempts = [
      {
        peer_id: PEERS.traveler,
        attempt_path: "ping",
        data_proto: "quic",
        outcome: "fail",
        stage: "peer_contact",
        reason_code: "UNAVAILABLE",
        stop_condition: "dial_failed",
      },
    ];
    return { connected: true, topology: top };
  }
  if (name === "disconnected") {
    return {
      connected: false,
      topology: emptyTopology(),
      failure: {
        stage: "desktop",
        reason_code: "daemon_not_running",
        exit_code: 70,
        message: "LocalAPI is not reachable",
        suggestions: [{ message: "retry desktop connection" }],
        facts: [{ message: "ui test fixture simulates a disconnected daemon" }],
      },
    };
  }
  if (name === "session-connected") {
    return {
      connected: true,
      selected: "user",
      addr: "unix:/tmp/miopunch-session.sock",
      desktop_managed: false,
      bootstrap_state: "none",
      topology: ownerTopology(),
      diagnostics: [{ message: "selected_endpoint=user" }],
    };
  }
  if (name === "bootstrapping") {
    return {
      connected: false,
      selected: "user",
      addr: "unix:/tmp/miopunch-session.sock",
      desktop_managed: true,
      bootstrap_state: "starting",
      topology: emptyTopology(),
      diagnostics: [{ message: "bootstrap_stage=wait_ready" }],
      bootstrap: {
        attempted: true,
        stage: "wait_ready",
        daemon_path: "/tmp/miopunch",
        pid: 4242,
      },
    };
  }
  if (name === "desktop-managed") {
    return {
      connected: true,
      selected: "user",
      addr: "unix:/tmp/miopunch-session.sock",
      desktop_managed: true,
      bootstrap_state: "ready",
      topology: ownerTopology(),
      diagnostics: [{ message: "selected_endpoint=user" }],
      bootstrap: {
        attempted: true,
        stage: "ready",
        daemon_path: "/tmp/miopunch",
        pid: 4242,
        stderr: "miopunch up: serving LocalAPI (user)",
      },
    };
  }
  if (name === "reused-daemon") {
    return {
      connected: true,
      selected: "user",
      addr: "unix:/tmp/miopunch-session.sock",
      desktop_managed: false,
      bootstrap_state: "none",
      topology: ownerTopology(),
      diagnostics: [{ message: "selected_endpoint=user" }],
    };
  }
  if (name === "bootstrap-failure") {
    return {
      connected: false,
      selected: "",
      topology: emptyTopology(),
      bootstrap_state: "failed",
      bootstrap: {
        attempted: true,
        stage: "timeout",
        daemon_path: "/tmp/miopunch",
        stderr: "permission denied",
        error: "timed out waiting for LocalAPI",
      },
      failure: {
        stage: "desktop",
        reason_code: "unavailable",
        exit_code: 70,
        message: "same-user session daemon bootstrap failed",
        suggestions: [
          { message: "retry desktop connection" },
          { message: "check that ./miopunch is next to ./miopunch-desktop and executable" },
          { message: "export runtime diagnostics" },
        ],
        facts: [
          { message: "bootstrap_stage=timeout" },
          { message: "error=timed out waiting for LocalAPI" },
        ],
      },
    };
  }
  if (name === "reconnect-on-refresh") {
    return {
      connected: false,
      reconnect_after_connects: 2,
      topology: ownerTopology(),
      failure: {
        stage: "desktop",
        reason_code: "daemon_not_running",
        exit_code: 70,
        message: "LocalAPI is not reachable",
        suggestions: [{ message: "retry desktop connection" }],
        facts: [{ message: "ui test fixture simulates reconnect on refresh" }],
      },
    };
  }
  return { connected: true, topology: ownerTopology() };
}

async function installFakeBridge(page, options = {}) {
  const fx = fixtureData(options.fixture || "owner");
  const data = {
    fixture: fx,
    createTaskModes: options.createTaskModes || {},
    getTaskModes: options.getTaskModes || {},
    initialTasks: options.initialTasks || [],
    inviteCodeDelivery: options.inviteCodeDelivery || "immediate",
    timeoutMs: options.timeoutMs || 120,
    inviteCode,
    confirm: options.confirm !== false,
  };

  await page.addInitScript((init) => {
    window.localStorage.clear();
    window.__miopunchBridgeTimeoutMs = init.timeoutMs;
    window.__miopunchCalls = [];
    window.__miopunchRuntimeHandlers = {};
    window.__miopunchWebSockets = [];

    const calls = window.__miopunchCalls;
    const topology = init.fixture.topology;
    const peers = Array.isArray(topology.members)
      ? topology.members.map((m) => ({ peer_id: m.peer_id }))
      : [];
    let taskSeq = 1;
    let connectSeq = 0;
    const tasks = new Map((init.initialTasks || []).map((task) => [String(task.task_id || ""), task]));
    let connection = {
      connected: !!init.fixture.connected,
      selected: init.fixture.selected || (init.fixture.connected ? "user" : ""),
      addr: init.fixture.addr || (init.fixture.connected ? "unix:/tmp/miopunch-ui-test.sock" : ""),
      system_addr: "unix:/run/miopunch/localapi.sock",
      user_addr: "unix:/tmp/miopunch-user.sock",
      bootstrap_state: init.fixture.bootstrap_state || "none",
      desktop_managed: !!init.fixture.desktop_managed,
      diagnostics: init.fixture.diagnostics || [],
      bootstrap: init.fixture.bootstrap || null,
      failure: init.fixture.failure || null,
    };

    const clone = (value) => (value == null ? value : JSON.parse(JSON.stringify(value)));
    const record = (entry) => calls.push(clone(entry));
    const nextTaskID = (kind) => `ui-${kind}-${String(taskSeq++).padStart(3, "0")}`;
    const addInvitePeerFact = (task) => {
      if (task.facts.some((fact) => fact && fact.term_id === "peer_id")) return;
      const peerID = String(topology.self && topology.self.peer_id ? topology.self.peer_id : "peer-ui-test-owner");
      task.facts.push({ term_id: "peer_id", message: `peer_id=${peerID}` });
    };
    const addInviteNetFact = (task) => {
      if (task.facts.some((fact) => fact && fact.term_id === "net_id")) return;
      task.facts.push({ term_id: "net_id", message: "net_id=net-ui-test" });
    };
    const addInviteSuggestion = (task) => {
      if (task.suggestions.some((s) => s && String(s.message || "").includes("miopunch join"))) return;
      task.suggestions.push({ message: "on another machine: miopunch join <invite_code>" });
    };
    const addInviteCodeFact = (task) => {
      if (task.facts.some((fact) => fact && fact.term_id === "invite_code")) return;
      task.facts.push({ term_id: "invite_code", message: init.inviteCode });
    };
    const completeInviteTask = (task, withCode) => {
      task.status = "done";
      task.stage = "invite code ready";
      task.reason_code = "OK";
      task.exit_code = 0;
      task.report_ready = true;
      if (withCode) addInviteCodeFact(task);
    };
    const okTask = (kind, args) => {
      const task = {
        task_id: nextTaskID(kind),
        kind,
        status: "done",
        stage: "complete",
        reason_code: "OK",
        exit_code: 0,
        report_ready: true,
        created_at: new Date().toISOString(),
        facts: [],
        suggestions: [],
      };
      if (kind === "invite") {
        if (init.inviteCodeDelivery === "immediate") {
          completeInviteTask(task, true);
        } else if (init.inviteCodeDelivery === "missing") {
          completeInviteTask(task, false);
        } else if (init.inviteCodeDelivery === "partial-done-fetch") {
          completeInviteTask(task, false);
          task.stage = "SelfDiscovery";
          addInvitePeerFact(task);
          addInviteSuggestion(task);
        } else {
          task.status = "running";
          task.stage = "prepare invite code";
          task.reason_code = "";
          task.exit_code = undefined;
          task.report_ready = false;
        }
      } else if (kind === "join") {
        task.stage = "membership accepted";
        task.facts.push({ message: `invite_code=${String(args && args.code ? args.code : "")}` });
      } else if (kind === "approve") {
        task.status = "running";
        task.stage = "waiting for joiner";
        task.report_ready = false;
      } else if (kind === "ping") {
        task.stage = "payload exchanged";
        task.facts.push({ message: `peer_id=${String(args && args.peer_id ? args.peer_id : "")}` });
        task.facts.push({ message: "rtt_ms=18" });
      } else if (kind === "sh_ls") {
        task.stage = "sessions listed";
        task.facts.push({ message: `peer_id=${String(args && args.peer_id ? args.peer_id : "")}` });
        task.facts.push({ message: "sessions=main, maintenance" });
      } else if (kind === "sh_attach") {
        task.status = "running";
        task.stage = "attached";
        task.report_ready = false;
        task.facts.push({ message: `peer_id=${String(args && args.peer_id ? args.peer_id : "")}` });
        task.facts.push({ message: `session=${String(args && args.session ? args.session : "main")}` });
      } else if (kind === "revoke_member") {
        task.stage = "decl written";
        task.facts.push({ message: `revoked_peer_id=${String(args && args.peer_id ? args.peer_id : "")}` });
      }
      tasks.set(task.task_id, task);
      return task;
    };
    const getTaskReads = new Map();
    const bridgeError = (kind) => ({
      ok: false,
      error: {
        stage: "desktop",
        reason_code: "bridge_failure",
        exit_code: 70,
        message: `${kind} failed in fake bridge`,
      },
    });

    window.confirm = () => init.confirm;
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text) => record({ method: "Clipboard.writeText", text }),
      },
    });

    window.runtime = {
      EventsOn: (name, cb) => {
        record({ method: "EventsOn", name });
        window.__miopunchRuntimeHandlers[name] = cb;
      },
    };
    window.__miopunchEmit = (name, payload) => {
      const handler = window.__miopunchRuntimeHandlers[name];
      if (typeof handler === "function") handler(payload);
    };

    class FakeWebSocket {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url, protocols) {
        this.url = url;
        this.protocols = protocols;
        this.readyState = FakeWebSocket.CONNECTING;
        this.binaryType = "";
        this.sent = [];
        window.__miopunchWebSockets.push(this);
        record({ method: "WebSocket", url, protocols });
        window.setTimeout(() => {
          this.readyState = FakeWebSocket.OPEN;
          if (typeof this.onopen === "function") this.onopen({});
        }, 0);
      }

      send(data) {
        this.sent.push(typeof data === "string" ? data : `[binary:${data.byteLength || 0}]`);
        record({ method: "WebSocket.send", data: this.sent[this.sent.length - 1] });
      }

      close(code = 1000, reason = "") {
        this.readyState = FakeWebSocket.CLOSED;
        record({ method: "WebSocket.close", code, reason });
        if (typeof this.onclose === "function") this.onclose({ code, reason });
      }
    }
    window.WebSocket = FakeWebSocket;

    window.go = {
      main: {
        App: {
          Connect: async () => {
            record({ method: "Connect" });
            connectSeq += 1;
            if (
              init.fixture.reconnect_after_connects &&
              connectSeq >= init.fixture.reconnect_after_connects
            ) {
              connection = {
                ...connection,
                connected: true,
                selected: "user",
                addr: connection.user_addr,
                bootstrap_state: "ready",
                desktop_managed: true,
                failure: null,
              };
            }
            return connection;
          },
          GetStatus: async () => {
            record({ method: "GetStatus" });
            return { ok: true, status: { version: "ui-test", uptime_ms: 1000 } };
          },
          GetPeers: async () => {
            record({ method: "GetPeers" });
            return { ok: true, peers: { peers } };
          },
          GetTopology: async () => {
            record({ method: "GetTopology" });
            return { ok: true, topology };
          },
          GetTasks: async () => {
            record({ method: "GetTasks" });
            return { ok: true, tasks: { tasks: Array.from(tasks.values()).map(clone) } };
          },
          CreateTask: async (kind, args) => {
            record({ method: "CreateTask", kind, args });
            const mode = init.createTaskModes[kind] || "success";
            if (mode === "failure") return bridgeError(kind);
            if (mode === "timeout") return new Promise(() => {});
            return { ok: true, task: clone(okTask(kind, args || {})) };
          },
          GetTask: async (taskID) => {
            record({ method: "GetTask", taskID });
            const mode = init.getTaskModes[taskID] || init.getTaskModes["*"] || "success";
            if (mode === "failure") return bridgeError("GetTask");
            if (mode === "timeout") return new Promise(() => {});
            const key = String(taskID);
            const task = tasks.get(key) || null;
            if (task && task.kind === "invite" && init.inviteCodeDelivery === "delayed-fetch") {
              const reads = (getTaskReads.get(key) || 0) + 1;
              getTaskReads.set(key, reads);
              if (reads >= 2) completeInviteTask(task, true);
            } else if (task && task.kind === "invite" && init.inviteCodeDelivery === "partial-done-fetch") {
              addInviteNetFact(task);
              completeInviteTask(task, true);
            } else if (task && task.kind === "invite" && init.inviteCodeDelivery === "partial-event-fetch") {
              const reads = (getTaskReads.get(key) || 0) + 1;
              getTaskReads.set(key, reads);
              if (reads >= 2) {
                addInvitePeerFact(task);
                addInviteNetFact(task);
                completeInviteTask(task, true);
              }
            }
            return { ok: true, task: clone(tasks.get(String(taskID)) || null) };
          },
          ExportTaskReport: async (taskID) => {
            record({ method: "ExportTaskReport", taskID });
            return { ok: true, path: `/tmp/${String(taskID || "task")}.md` };
          },
          SetLocalAPIOverride: async (addr) => {
            record({ method: "SetLocalAPIOverride", addr });
            connection = { ...connection, connected: true, selected: "override", addr, override_addr: addr };
            return connection;
          },
          ClearLocalAPIOverride: async () => {
            record({ method: "ClearLocalAPIOverride" });
            connection = { ...connection, connected: true, selected: "user", addr: connection.user_addr, override_addr: "" };
            return connection;
          },
          Quit: async () => {
            record({ method: "Quit" });
          },
          TerminalBridgeInfo: async () => {
            record({ method: "TerminalBridgeInfo" });
            return {
              ok: true,
              base_url: "ws://127.0.0.1:9",
              token: "ui-test-token",
              subprotocol: "miopunch.sh.v0",
            };
          },
        },
      },
    };
  }, data);
}

async function openDesktop(page, options = {}) {
  await installFakeBridge(page, options);
  await page.goto(options.path || "/");
  await expect(page.locator("#page-host .page")).toBeVisible();
}

async function calls(page) {
  return page.evaluate(() => window.__miopunchCalls || []);
}

async function createTaskCalls(page, kind) {
  return (await calls(page)).filter((call) => call.method === "CreateTask" && (!kind || call.kind === kind));
}

async function expectCreateTaskCall(page, kind, args) {
  await expect.poll(async () => createTaskCalls(page, kind)).toContainEqual({ method: "CreateTask", kind, args });
}

async function emitRuntime(page, name, payload) {
  await page.evaluate(
    ({ eventName, eventPayload }) => window.__miopunchEmit(eventName, eventPayload),
    { eventName: name, eventPayload: payload }
  );
}

module.exports = {
  PEERS,
  calls,
  clone,
  createTaskCalls,
  emitRuntime,
  expect,
  expectCreateTaskCall,
  inviteCode,
  openDesktop,
  test,
};
