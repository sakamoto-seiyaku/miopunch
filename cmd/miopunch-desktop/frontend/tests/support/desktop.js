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
    terminalBridgeInfoModes: options.terminalBridgeInfoModes || options.terminalBridgeInfoMode || ["success"],
    webSocketModes: options.webSocketModes || options.webSocketMode || ["success"],
    shellDiscovery: options.shellDiscovery || {
      defaultTargets: ["local", "ssh:ops"],
      sessionsByTarget: {
        local: ["main", "maintenance"],
        "ssh:ops": ["ops-main"],
      },
    },
    disableTerminal: !!options.disableTerminal,
    initialTasks: options.initialTasks || [],
    approvalRequests: options.approvalRequests || null,
    inviteCodeDelivery: options.inviteCodeDelivery || "immediate",
    runtimeStartDelayMs: options.runtimeStartDelayMs || 0,
    runtimeResyncDelayMs: options.runtimeResyncDelayMs || 0,
    runtimeStartFailures: Number(options.runtimeStartFailures || 0),
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
    const record = (entry) => calls.push(clone(entry));

    if (init.disableTerminal) {
      Object.defineProperty(window, "Terminal", {
        configurable: true,
        get: () => undefined,
        set: () => true,
      });
    } else {
      let wrappedTerminal = null;
      Object.defineProperty(window, "Terminal", {
        configurable: true,
        get: () => wrappedTerminal,
        set: (value) => {
          if (typeof value !== "function") {
            wrappedTerminal = value;
            return true;
          }
          wrappedTerminal = class extends value {
            constructor(...args) {
              super(...args);
              this.__miopunchOutput = "";
            }

            __recordOutput(data) {
              this.__miopunchOutput += String(data ?? "");
              record({ method: "Terminal.write", data: String(data ?? "") });
              if (this.element) this.element.setAttribute("data-test-output", this.__miopunchOutput);
            }

            focus(...args) {
              record({ method: "Terminal.focus" });
              return super.focus(...args);
            }

            write(data, ...args) {
              this.__recordOutput(data);
              return super.write(data, ...args);
            }

            writeln(data, ...args) {
              this.__recordOutput(`${String(data ?? "")}\r\n`);
              return super.writeln(data, ...args);
            }
          };
          return true;
        },
      });
    }

    const clone = (value) => (value == null ? value : JSON.parse(JSON.stringify(value)));
    const nextMode = (value, fallback = "success") => {
      if (Array.isArray(value)) return value.length ? value.shift() : fallback;
      return typeof value === "undefined" ? fallback : value;
    };
    const normalizeWebSocketMode = (value) => {
      if (value && typeof value === "object" && !Array.isArray(value)) return clone(value);
      return { mode: typeof value === "undefined" ? "success" : value };
    };
    const nextMapMode = (map, key, fallback = "success") => nextMode(map && Object.prototype.hasOwnProperty.call(map, key) ? map[key] : undefined, fallback);
    const nextTaskMode = (map, key) => {
      if (map && Object.prototype.hasOwnProperty.call(map, key)) return nextMode(map[key], "success");
      if (map && Object.prototype.hasOwnProperty.call(map, "*")) return nextMode(map["*"], "success");
      return "success";
    };
    const createTaskModes = clone(init.createTaskModes || {});
    const getTaskModes = clone(init.getTaskModes || {});
    const terminalBridgeInfoModes = clone(init.terminalBridgeInfoModes || ["success"]);
    const webSocketModes = clone(init.webSocketModes || ["success"]);
    const shellDiscovery = clone(init.shellDiscovery || {});
    const defaultShellTargets = Array.isArray(shellDiscovery.defaultTargets) && shellDiscovery.defaultTargets.length
      ? shellDiscovery.defaultTargets
      : ["local", "ssh:ops"];
    const shellTargetsByPeer = shellDiscovery.targetsByPeer || {};
    const shellSessionsByTarget = shellDiscovery.sessionsByTarget || {
      local: ["main", "maintenance"],
      "ssh:ops": ["ops-main"],
    };
    const shellTargetsForPeer = (peerID) => {
      const list = shellTargetsByPeer[peerID] || shellTargetsByPeer["*"];
      return Array.isArray(list) && list.length ? list : defaultShellTargets;
    };
    const shellSessionsForTarget = (target) => {
      const list = shellSessionsByTarget[target] || shellSessionsByTarget["*"];
      return Array.isArray(list) && list.length ? list : ["main", "maintenance"];
    };
    let topology = clone(init.fixture.topology);
    const peers = Array.isArray(topology.members)
      ? topology.members.map((m) => ({ peer_id: m.peer_id }))
      : [];
    const defaultRuntimeConfig = {
      known_peers: peers.map((peer) => ({ peer_id: peer.peer_id })),
      desired: {
        runtime: {
          mqtt_brokers: ["broker-1.miopunch.local:1883"],
          p2p_network: "auto",
          p2p_ip_family: "auto",
          data_proto: "quic",
          quic_cc: "bbr",
          stun: ["stun-1.miopunch.local:3478"],
          stun_explicit: true,
          disable_portmap: false,
          disable_assisted_addrs: false,
        },
        preferences: {
          default_shell_target: "local",
          default_shell_session: "main",
          log_level: "info",
        },
      },
      effective: {
        runtime: {
          mqtt_brokers: ["broker-1.miopunch.local:1883"],
          p2p_network: "auto",
          p2p_ip_family: "auto",
          data_proto: "quic",
          quic_cc: "bbr",
          stun: ["stun-1.miopunch.local:3478"],
          stun_explicit: true,
          disable_portmap: false,
          disable_assisted_addrs: false,
        },
        preferences: {
          default_shell_target: "local",
          default_shell_session: "main",
          log_level: "info",
        },
      },
      apply: {
        runtime: "immediate",
        preferences: "immediate",
        active_peer_sessions: 0,
        active_shell_sessions: 0,
        requires_reconnect: false,
        restart_required: false,
      },
    };
    let taskSeq = 1;
    let connectSeq = 0;
    let runtimeStartSeq = 0;
    let runtimeRev = 0;
    let runtimePeerSessions = clone(init.fixture.peer_sessions || []);
    let runtimeShellSessions = clone(init.fixture.shell_sessions || []);
    let runtimeConfig = clone(init.fixture.config || defaultRuntimeConfig);
    let runtimeDiagnostics = clone(init.fixture.runtime_diagnostics || init.fixture.diagnostics || []);
    let runtimeApprovalRequests = clone(init.approvalRequests || init.fixture.approval_requests || []);
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

    const nextTaskID = (kind) => `ui-${kind}-${String(taskSeq++).padStart(3, "0")}`;
    const runtimeSnapshot = () => ({
      rev: runtimeRev,
      status: { version: "ui-test", uptime_ms: 1000, mode: "user" },
      topology: clone(topology),
      tasks: Array.from(tasks.values()).map(clone),
      peer_sessions: clone(runtimePeerSessions),
      shell_sessions: clone(runtimeShellSessions),
      config: clone(runtimeConfig),
      diagnostics: clone(runtimeDiagnostics),
      approval_requests: clone(runtimeApprovalRequests),
    });
    const waitRuntimeStart = async () => {
      if (init.runtimeStartDelayMs > 0) {
        await new Promise((resolve) => window.setTimeout(resolve, init.runtimeStartDelayMs));
      }
    };
    const waitRuntimeResync = async () => {
      if (init.runtimeResyncDelayMs > 0) {
        await new Promise((resolve) => window.setTimeout(resolve, init.runtimeResyncDelayMs));
      }
    };
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
    const applyTaskResult = (task, result) => {
      if (!task || !result || typeof result !== "object") return;
      task.status = String(result.status || "done");
      if (Object.prototype.hasOwnProperty.call(result, "stage")) task.stage = String(result.stage || "");
      if (Object.prototype.hasOwnProperty.call(result, "reason_code")) task.reason_code = String(result.reason_code || "");
      if (Object.prototype.hasOwnProperty.call(result, "exit_code")) task.exit_code = result.exit_code;
      if (Array.isArray(result.facts)) task.facts = task.facts.concat(clone(result.facts));
      if (Array.isArray(result.suggestions)) task.suggestions = clone(result.suggestions);
      task.report_ready = Object.prototype.hasOwnProperty.call(result, "report_ready") ? !!result.report_ready : true;
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
      } else if (kind === "approve_decision") {
        task.stage = `${String(args && args.decision ? args.decision : "decision")} submitted`;
        task.facts.push({ message: `approve_task_id=${String(args && args.approve_task_id ? args.approve_task_id : "")}` });
        task.facts.push({ message: `request_msg_id=${String(args && args.request_msg_id ? args.request_msg_id : "")}` });
        const req = runtimeApprovalRequests.find((item) =>
          String(item.approve_task_id || item.task_id || "") === String(args && args.approve_task_id || "") &&
          String(item.request_msg_id || "") === String(args && args.request_msg_id || "")
        );
        if (req) {
          req.status = String(args && args.decision || "") === "reject" ? "rejected" : "approved";
          req.decision = String(args && args.decision || "");
          req.updated_at = new Date().toISOString();
        }
      } else if (kind === "ping") {
        task.stage = "payload exchanged";
        task.facts.push({ message: `peer_id=${String(args && args.peer_id ? args.peer_id : "")}` });
        task.facts.push({ message: "rtt_ms=18" });
      } else if (kind === "sh_ls") {
        const peerID = String(args && args.peer_id ? args.peer_id : "");
        const target = String(args && args.target ? args.target : "");
        task.stage = target ? "sessions listed" : "targets listed";
        task.facts.push({ term_id: "peer_id", message: `peer_id=${peerID}` });
        if (target) {
          for (const session of shellSessionsForTarget(target)) {
            task.facts.push({ term_id: "session", message: `session=${session}` });
          }
        } else {
          for (const value of shellTargetsForPeer(peerID)) {
            task.facts.push({ term_id: "target", message: `target=${value}` });
          }
        }
      } else if (kind === "sh_attach") {
        task.status = "running";
        task.stage = "attached";
        task.report_ready = false;
        task.facts.push({ term_id: "peer_id", message: `peer_id=${String(args && args.peer_id ? args.peer_id : "")}` });
        task.facts.push({ term_id: "target", message: `target=${String(args && args.target ? args.target : "local")}` });
        task.facts.push({ term_id: "session", message: `session=${String(args && args.session ? args.session : "main")}` });
        runtimeShellSessions = runtimeShellSessions.filter((item) => String(item && item.task_id || "") !== String(task.task_id));
        runtimeShellSessions.push({
          task_id: task.task_id,
          peer_id: String(args && args.peer_id ? args.peer_id : ""),
          target: String(args && args.target ? args.target : "local"),
          session: String(args && args.session ? args.session : "main"),
          status: "running",
          stage: "attached",
          created_at: task.created_at,
          report_ready: false,
        });
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
    window.__miopunchSetRuntimeSnapshot = (snapshot) => {
      if (!snapshot || typeof snapshot !== "object") return;
      if (typeof snapshot.rev !== "undefined") runtimeRev = Number(snapshot.rev || 0);
      if (snapshot.topology) topology = clone(snapshot.topology);
      if (Array.isArray(snapshot.tasks)) {
        tasks.clear();
        for (const task of snapshot.tasks) {
          const taskID = String(task && task.task_id ? task.task_id : "");
          if (taskID) tasks.set(taskID, clone(task));
        }
      }
      if (Array.isArray(snapshot.peer_sessions)) runtimePeerSessions = clone(snapshot.peer_sessions);
      if (Array.isArray(snapshot.shell_sessions)) runtimeShellSessions = clone(snapshot.shell_sessions);
      if (snapshot.config) runtimeConfig = clone(snapshot.config);
      if (Array.isArray(snapshot.diagnostics)) runtimeDiagnostics = clone(snapshot.diagnostics);
      if (Array.isArray(snapshot.approval_requests)) runtimeApprovalRequests = clone(snapshot.approval_requests);
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
        this.taskID = "";
        this.modeConfig = null;
        const taskMatch = /\/tasks\/([^/?]+)\/ws/.exec(url);
        if (taskMatch && taskMatch[1]) this.taskID = decodeURIComponent(taskMatch[1]);
        window.__miopunchWebSockets.push(this);
        record({ method: "WebSocket", url, protocols });
        const modeConfig = normalizeWebSocketMode(nextMode(webSocketModes, "success"));
        this.modeConfig = modeConfig;
        const mode = String(modeConfig.mode || "success");
        const deliverRemoteMessages = () => {
          if (mode === "open_no_data" || modeConfig.remoteMessages === false) return;
          const messages = Array.isArray(modeConfig.remoteMessages)
            ? modeConfig.remoteMessages
            : [Object.prototype.hasOwnProperty.call(modeConfig, "remoteMessage") ? modeConfig.remoteMessage : "__MIO_FAKE_REMOTE_OUTPUT__\r\n"];
          for (const message of messages) {
            const encoded = new TextEncoder().encode(String(message ?? ""));
            if (typeof this.onmessage === "function") this.onmessage({ data: encoded.buffer });
          }
        };
        window.setTimeout(() => {
          if (mode === "close_before_open") {
            this.readyState = FakeWebSocket.CLOSED;
            if (typeof this.onclose === "function") this.onclose({ code: 1011, reason: "connect failed in fake websocket" });
            return;
          }
          if (mode === "error_before_open") {
            if (typeof this.onerror === "function") this.onerror({ message: "fake websocket error" });
            this.readyState = FakeWebSocket.CLOSED;
            if (typeof this.onclose === "function") this.onclose({ code: 1011, reason: "connect failed in fake websocket" });
            return;
          }
          this.readyState = FakeWebSocket.OPEN;
          if (typeof this.onopen === "function") this.onopen({});
          window.setTimeout(deliverRemoteMessages, 0);
          if (mode === "close_after_open") {
            const closeCode = Number(modeConfig.closeCode || 1011);
            const closeReason = String(modeConfig.closeReason || "fake websocket transport lost");
            window.setTimeout(() => this.close(closeCode, closeReason), 30);
          }
        }, 0);
      }

      send(data) {
        this.sent.push(typeof data === "string" ? data : `[binary:${data.byteLength || 0}]`);
        record({ method: "WebSocket.send", data: this.sent[this.sent.length - 1] });
      }

      close(code = 1000, reason = "") {
        this.readyState = FakeWebSocket.CLOSED;
        record({ method: "WebSocket.close", code, reason });
        if (this.taskID) {
          runtimeShellSessions = runtimeShellSessions.filter((item) => String(item && item.task_id || "") !== this.taskID);
          const task = tasks.get(this.taskID);
          if (task && task.kind === "sh_attach") {
            if (this.modeConfig && this.modeConfig.taskResult) {
              applyTaskResult(task, this.modeConfig.taskResult);
            } else {
              task.status = "done";
              task.stage = "disconnected";
              task.reason_code = "OK";
              task.exit_code = 0;
              task.report_ready = true;
            }
          }
        }
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
          DesktopRuntimeStart: async () => {
            record({ method: "DesktopRuntimeStart" });
            connectSeq += 1;
            runtimeStartSeq += 1;
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
            if (!connection.connected) {
              await waitRuntimeStart();
              return { ok: false, error: clone(connection.failure), connection: clone(connection) };
            }
            await waitRuntimeStart();
            if (runtimeStartSeq <= init.runtimeStartFailures) {
              return {
                ok: false,
                error: {
                  stage: "desktop",
                  reason_code: "unavailable",
                  exit_code: 70,
                  message: "desktop event stream did not begin with snapshot",
                  suggestions: [{ message: "retry runtime start" }],
                  facts: [],
                },
                connection: clone(connection),
              };
            }
            return { ok: true, connection: clone(connection), state: runtimeSnapshot() };
          },
          DesktopRuntimeResync: async () => {
            record({ method: "DesktopRuntimeResync" });
            await waitRuntimeResync();
            if (!connection.connected) {
              return { ok: false, error: clone(connection.failure), connection: clone(connection) };
            }
            return { ok: true, connection: clone(connection), state: runtimeSnapshot() };
          },
          SaveDesktopConfig: async (update) => {
            record({ method: "SaveDesktopConfig", update });
            if (!connection.connected) {
              return { ok: false, error: clone(connection.failure), connection: clone(connection) };
            }
            if (update && update.runtime && Array.isArray(update.runtime.mqtt_brokers) && update.runtime.mqtt_brokers.includes("broker-without-port")) {
              return {
                ok: false,
                error: {
                  stage: "localapi",
                  reason_code: "BAD_REQUEST",
                  exit_code: 2,
                  message: "invalid MQTT brokers",
                  facts: [{ message: "invalid MQTT brokers" }],
                  suggestions: [{ message: "use host:port" }],
                },
                connection: clone(connection),
              };
            }
            const desired = runtimeConfig.desired || {};
            const effective = runtimeConfig.effective || {};
            runtimeConfig = {
              ...runtimeConfig,
              desired: {
                runtime: { ...(desired.runtime || {}), ...(update && update.runtime ? update.runtime : {}) },
                preferences: { ...(desired.preferences || {}), ...(update && update.preferences ? update.preferences : {}) },
              },
              effective: {
                runtime: { ...(effective.runtime || {}), ...(update && update.runtime ? update.runtime : {}) },
                preferences: { ...(effective.preferences || {}), ...(update && update.preferences ? update.preferences : {}) },
              },
            };
            return { ok: true, connection: clone(connection), state: runtimeSnapshot() };
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
            const mode = nextMapMode(createTaskModes, kind, "success");
            if (mode === "failure") return bridgeError(kind);
            if (mode === "timeout") return new Promise(() => {});
            const approvalBefore = clone(runtimeApprovalRequests);
            const task = okTask(kind, args || {});
            if (mode === "task_failure") {
              runtimeApprovalRequests = approvalBefore;
              task.status = "done";
              task.stage = "failed";
              task.reason_code = "CONFLICT";
              task.exit_code = 6;
              task.report_ready = true;
              task.facts.push({ message: `${kind} failed in fake task` });
            }
            return { ok: true, task: clone(task) };
          },
          GetTask: async (taskID) => {
            record({ method: "GetTask", taskID });
            const mode = nextTaskMode(getTaskModes, taskID);
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
          ExportDiagnostics: async () => {
            record({ method: "ExportDiagnostics" });
            return { ok: true, path: "/tmp/miopunch-diagnostics.zip" };
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
            const mode = nextMode(terminalBridgeInfoModes, "success");
            if (mode && typeof mode === "object") return clone(mode);
            if (mode === "failure") return bridgeError("TerminalBridgeInfo");
            if (mode === "missing") {
              return {
                ok: true,
                base_url: "",
                token: "",
                subprotocol: "miopunch.sh.v0",
              };
            }
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

async function setRuntimeSnapshot(page, snapshot) {
  await page.evaluate((nextSnapshot) => window.__miopunchSetRuntimeSnapshot(nextSnapshot), snapshot);
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
  setRuntimeSnapshot,
  test,
};
