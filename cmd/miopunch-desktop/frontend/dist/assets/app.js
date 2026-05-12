(() => {
  const el = (id) => document.getElementById(id);

  const COPY = {
    tabs: {
      network: { title: "Network", eyebrow: "device network" },
      access: { title: "Access", eyebrow: "join and share" },
      admin: { title: "Admin", eyebrow: "governance" },
      settings: { title: "Settings", eyebrow: "desktop client" },
    },
    empty: {
      devices: "No devices in this network yet",
      tasks: "No tasks yet",
      facts: "No facts",
      suggestions: "No suggestions",
      errors: "No active connection errors",
    },
  };

  const state = {
    rev: 0,
    status: null,
    topology: null,
    tasks: new Map(),
    peerSessions: [],
    shellSessions: [],
    config: { known_peers: [] },
    diagnostics: [],
    approvalRequests: [],
    activeTab: "network",
    view: { type: "overview" },
    previewMode: false,
    previewFixture: "owner",
  };

  const inviteState = { taskID: "", busy: false, message: "", missingTaskID: "" };
  const joinState = { taskID: "", lastExportPath: "" };
  const approveState = { taskID: "", message: "" };
  const approvalDecisionState = new Map();
  const approvalDecisionTasks = new Map();
  const shellView = {
    phase: "idle",
    peerID: "",
    target: "local",
    session: "main",
    targetOptions: [],
    sessionOptions: [],
    sessionTarget: "",
    error: "",
    detail: "",
    taskID: "",
    discoveryTaskID: "",
  };
  const shellSelections = new Map();
  const shellState = {
    ws: null,
    term: null,
    resizeObs: null,
    taskID: "",
    fitTimer: 0,
    expectedClose: false,
    wsError: "",
    remoteDataSeen: false,
  };

  let lastConn = null;
  let runtimeStream = { ready: false, status: "idle", error: null };
  let renderQueued = false;
  let previewTaskSeq = 1;
  let runtimeResyncInFlight = false;
  let runtimeRecoveryInFlight = false;
  let runtimeRecoveryQueue = [];

  const bridgeAvailable = () => !!(window.go && window.go.main && window.go.main.App);

  const getBridge = () => {
    const b = bridgeAvailable() ? window.go.main.App : null;
    if (!b) throw new Error("desktop bridge is not ready");
    return b;
  };

  const esc = (value) => String(value ?? "").replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;",
    "<": "&lt;",
    ">": "&gt;",
    "\"": "&quot;",
    "'": "&#39;",
  }[ch]));

  const canonicalTaskID = (raw) => String(raw || "").trim();
  const clone = (value) => JSON.parse(JSON.stringify(value));

  const toast = (message) => {
    const toastEl = el("toast");
    if (!toastEl) return;
    toastEl.textContent = String(message || "");
    toastEl.classList.add("is-visible");
    window.clearTimeout(toastEl._hideTimer);
    toastEl._hideTimer = window.setTimeout(() => toastEl.classList.remove("is-visible"), 2200);
  };

  const scheduleRender = () => {
    if (renderQueued) return;
    renderQueued = true;
    requestAnimationFrame(() => {
      renderQueued = false;
      renderAll();
    });
  };

  const fmtUptime = (ms) => {
    const s = Math.max(0, Math.floor(Number(ms || 0) / 1000));
    const h = Math.floor(s / 3600);
    const m = Math.floor((s % 3600) / 60);
    const ss = s % 60;
    if (h > 0) return `${h}h ${m}m ${ss}s`;
    if (m > 0) return `${m}m ${ss}s`;
    return `${ss}s`;
  };

  const shortID = (raw) => {
    const s = String(raw || "").trim();
    if (!s) return "-";
    if (s.length <= 18) return s;
    return `${s.slice(0, 12)}...${s.slice(-6)}`;
  };

  const headShort = (raw) => {
    const s = String(raw || "").trim();
    if (!s) return "-";
    return s.length > 14 ? `${s.slice(0, 14)}...` : s;
  };

  const bridgeErrorSummary = (err) => {
    if (!err) return "unknown error";
    const reason = err.reason_code ? String(err.reason_code) : "";
    const msg = err.message ? String(err.message) : "";
    if (reason && msg) return `${reason}: ${msg}`;
    return reason || msg || "unknown error";
  };

  const chipHTML = (text, cls = "") => `<span class="chip ${cls}">${esc(text || "-")}</span>`;
  const metricHTML = (label, value) => `
    <div class="metric">
      <div class="metric-label">${esc(label)}</div>
      <div class="metric-value">${esc(value)}</div>
    </div>`;
  const detailRowHTML = (label, value) => `
    <div class="detail-row"><span>${esc(label)}</span><strong>${esc(value || "-")}</strong></div>`;
  const listItemHTML = (text, cls = "") => `<div class="list-item ${cls}">${esc(text || "")}</div>`;

  const copyToClipboard = async (text) => {
    try {
      await navigator.clipboard.writeText(text);
      toast("Copied");
      return;
    } catch {
      // Fall back to selection-based copy for restricted preview contexts.
    }

    const ta = document.createElement("textarea");
    ta.value = text;
    ta.setAttribute("readonly", "true");
    ta.style.position = "absolute";
    ta.style.left = "-9999px";
    document.body.appendChild(ta);
    ta.select();
    document.execCommand("copy");
    document.body.removeChild(ta);
    toast("Copied");
  };

  const normalizeTaskArgs = (args) => {
    if (!args || typeof args !== "object" || Array.isArray(args)) return {};
    return args;
  };

  const withTimeout = async (promise, label, timeoutMs = 12000) => {
    const overrideMs = Number(window.__miopunchBridgeTimeoutMs || 0);
    const effectiveTimeoutMs = Number.isFinite(overrideMs) && overrideMs > 0 ? overrideMs : timeoutMs;
    let timer = 0;
    try {
      return await Promise.race([
        Promise.resolve(promise),
        new Promise((_, reject) => {
          timer = window.setTimeout(() => reject(new Error(`${label} timed out`)), effectiveTimeoutMs);
        }),
      ]);
    } finally {
      if (timer) window.clearTimeout(timer);
    }
  };

  const sleep = (ms) => new Promise((resolve) => window.setTimeout(resolve, ms));

  const previewFixtures = {
    owner: {
      connection: {
        connected: true,
        selected: "preview",
        addr: "static://owner",
        system_addr: "unix:/run/miopunch/localapi.sock",
        user_addr: "unix:/tmp/miopunch-user.sock",
      },
      status: { version: "preview-zima", uptime_ms: 7343000, mode: "preview" },
      topology: {
        format: "miopunch.topology.preview",
        observed_at: "2026-05-04T00:00:00Z",
        self: { peer_id: "peer-owner-zima-blue-0001", role: "owner", v4_hint: "easy", v6_hint: "direct" },
        net: { net_id: "net_zima_blue_lab", brokers_effective: ["broker-1.miopunch.local:1883"] },
        state_head: { governance_head_b64: "gov_owner_head_preview", decls_head_b64: "decls_owner_head_preview" },
        members: [
          { peer_id: "peer-owner-zima-blue-0001", role: "owner", v4_hint: "easy", v6_hint: "direct" },
          { peer_id: "peer-studio-workstation-02", role: "admin", v4_hint: "easy", v6_hint: "direct" },
          { peer_id: "peer-livingroom-mini-03", role: "member", v4_hint: "portmap", v6_hint: "" },
          { peer_id: "peer-travel-laptop-04", role: "member", v4_hint: "hard", v6_hint: "" },
          { peer_id: "peer-old-phone-05", role: "member", revoked: true },
        ],
        presence: { online_window_sec: 120, hello_interval_sec: 30 },
        bootstrap: { recommendations: [], attempts: [], more_rounds: [] },
        neighbors: {
          target_k: 2,
          selected: [
            { peer_id: "peer-studio-workstation-02", role: "admin", bucket: "easy", reason: "stable admin", dialable: true },
            { peer_id: "peer-livingroom-mini-03", role: "member", bucket: "portmap", reason: "bucket diversity", dialable: true },
          ],
          active: [
            {
              peer_id: "peer-studio-workstation-02",
              bucket: "easy",
              data_proto: "quic",
              path_family: "udp6",
              healthy: true,
              last_activity_unix_ms: 1777824000000,
            },
            {
              peer_id: "peer-livingroom-mini-03",
              bucket: "portmap",
              data_proto: "kcp",
              path_family: "udp4",
              healthy: true,
              last_activity_unix_ms: 1777823980000,
            },
          ],
          unhealthy: [{ peer_id: "peer-travel-laptop-04", close_reason: "idle timeout" }],
          degree_distribution: [],
        },
        attempts: [],
        payloads: [],
        recovery: { events: [] },
      },
      tasks: [
        {
          task_id: "preview-invite-001",
          kind: "invite",
          status: "done",
          stage: "ready",
          reason_code: "OK",
          exit_code: 0,
          report_ready: true,
          created_at: "2026-05-04T00:00:00Z",
          facts: [{ message: "invite_code=mp:v0.preview.zima-blue.owner" }],
          suggestions: [],
        },
      ],
      approval_requests: [
        {
          approve_task_id: "preview-approve-001",
          invite_id: "preview-invite",
          request_msg_id: "JBSWY3DPEHPK3PXPJBSWY3DPAA",
          member_peer_id: "peer-new-tablet-06",
          member_name: "New tablet",
          platform: "linux",
          status: "pending",
          created_at: "2026-05-04T00:02:00Z",
        },
      ],
    },
    member: {
      connection: { connected: true, selected: "preview", addr: "static://member" },
      status: { version: "preview-zima", uptime_ms: 1933000, mode: "preview" },
      topology: {
        format: "miopunch.topology.preview",
        observed_at: "2026-05-04T00:00:00Z",
        self: { peer_id: "peer-travel-laptop-04", role: "member", v4_hint: "hard", v6_hint: "" },
        net: { net_id: "net_zima_blue_lab", brokers_effective: ["broker-1.miopunch.local:1883"] },
        state_head: { governance_head_b64: "gov_member_head_preview", decls_head_b64: "decls_member_head_preview" },
        members: [
          { peer_id: "peer-owner-zima-blue-0001", role: "owner", v4_hint: "easy", v6_hint: "direct" },
          { peer_id: "peer-studio-workstation-02", role: "admin", v4_hint: "easy", v6_hint: "direct" },
          { peer_id: "peer-travel-laptop-04", role: "member", v4_hint: "hard", v6_hint: "" },
        ],
        presence: { online_window_sec: 120, hello_interval_sec: 30 },
        bootstrap: { recommendations: [], attempts: [], more_rounds: [] },
        neighbors: {
          target_k: 2,
          selected: [{ peer_id: "peer-owner-zima-blue-0001", role: "owner", bucket: "easy", dialable: true }],
          active: [
            {
              peer_id: "peer-owner-zima-blue-0001",
              bucket: "easy",
              data_proto: "quic",
              path_family: "udp4",
              healthy: true,
              last_activity_unix_ms: 1777824000000,
            },
          ],
          unhealthy: [],
          degree_distribution: [],
        },
        attempts: [],
        payloads: [],
        recovery: { events: [] },
      },
      tasks: [],
    },
    empty: {
      connection: { connected: true, selected: "preview", addr: "static://empty" },
      status: { version: "preview-zima", uptime_ms: 64000, mode: "preview" },
      topology: {
        format: "miopunch.topology.preview",
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
      },
      tasks: [],
    },
    disconnected: {
      connection: {
        connected: false,
        selected: "",
        addr: "",
        system_addr: "unix:/run/miopunch/localapi.sock",
        user_addr: "unix:/tmp/miopunch-user.sock",
        failure: {
          stage: "desktop",
          reason_code: "daemon_not_running",
          exit_code: 70,
          message: "LocalAPI is not reachable",
          suggestions: [{ message: "retry desktop connection" }],
          facts: [{ message: "preview fixture simulates a disconnected daemon" }],
        },
      },
      status: null,
      topology: null,
      tasks: [
        {
          task_id: "preview-connect-001",
          kind: "connect",
          status: "done",
          stage: "desktop",
          reason_code: "daemon_not_running",
          exit_code: 70,
          report_ready: true,
          created_at: "2026-05-04T00:00:00Z",
          facts: [{ message: "LocalAPI is not reachable" }],
          suggestions: [{ message: "retry desktop connection" }],
        },
      ],
    },
  };

  const self = () => (state.topology && state.topology.self ? state.topology.self : {});
  const selfRole = () => String(self().role || "unknown").toLowerCase();
  const hasText = (value) => String(value || "").trim() !== "";
  const isFirstRunUninitialized = () => {
    if (!state.topology || (!state.previewMode && !(lastConn && lastConn.connected))) return false;
    const top = state.topology;
    const role = String(top.self && top.self.role || "").toLowerCase();
    const netID = top.net && top.net.net_id;
    const stateHead = top.state_head || {};
    const memberList = Array.isArray(top.members) ? top.members : [];
    return (!role || role === "unknown")
      && !hasText(netID)
      && !hasText(stateHead.governance_head_b64)
      && !hasText(stateHead.decls_head_b64)
      && memberList.length === 0;
  };
  const effectiveSelfRole = () => isFirstRunUninitialized() ? "owner" : selfRole();
  const roleKnown = () => !!(state.topology && state.topology.self && effectiveSelfRole());
  const isAdminRole = (role) => ["owner", "admin"].includes(String(role || "").toLowerCase());
  const adminVisible = () => isAdminRole(effectiveSelfRole());
  const approvalRequestKey = (req) => {
    const approveTaskID = String(req && (req.approve_task_id || req.task_id) || "").trim();
    const requestMsgID = String(req && req.request_msg_id || "").trim();
    return approveTaskID && requestMsgID ? `${approveTaskID}/${requestMsgID}` : "";
  };
  const approvalRequestStatus = (req) => String(req && req.status || "").toLowerCase();
  const pendingApprovalRequests = () => (Array.isArray(state.approvalRequests) ? state.approvalRequests : [])
    .filter((req) => approvalRequestKey(req));
  const approvalStatusClass = (status) => {
    const s = String(status || "").toLowerCase();
    if (s === "approved") return "chip-done";
    if (s === "rejected" || s === "expired") return "chip-error";
    if (s === "pending") return "chip-running";
    return "";
  };

  const members = () => {
    const top = state.topology || {};
    const list = Array.isArray(top.members) ? top.members.slice() : [];
    const selfPeerID = String(self().peer_id || "");
    if (selfPeerID && !list.some((m) => m.peer_id === selfPeerID)) {
      list.unshift({
        peer_id: selfPeerID,
        role: effectiveSelfRole(),
        v4_hint: self().v4_hint || "",
        v6_hint: self().v6_hint || "",
      });
    }
    return list;
  };

  const memberByPeerID = (peerID) => members().find((m) => String(m.peer_id || "") === String(peerID || ""));

  const activeNeighbor = (peerID) => {
    const active = state.topology && state.topology.neighbors && Array.isArray(state.topology.neighbors.active)
      ? state.topology.neighbors.active
      : [];
    return active.find((n) => String(n.peer_id || "") === String(peerID || "")) || null;
  };

  const selectedNeighbor = (peerID) => {
    const selected = state.topology && state.topology.neighbors && Array.isArray(state.topology.neighbors.selected)
      ? state.topology.neighbors.selected
      : [];
    return selected.find((n) => String(n.peer_id || "") === String(peerID || "")) || null;
  };

  const recentPeerFailure = (peerID) => {
    const id = String(peerID || "");
    if (!id) return null;
    const top = state.topology || {};
    const failures = top.neighbors && Array.isArray(top.neighbors.failures)
      ? top.neighbors.failures
      : [];
    for (let i = failures.length - 1; i >= 0; i -= 1) {
      const failure = failures[i];
      if (failure && String(failure.peer_id || "") === id) return failure;
    }
    const attempts = Array.isArray(top.attempts) ? top.attempts : [];
    for (let i = attempts.length - 1; i >= 0; i -= 1) {
      const attempt = attempts[i];
      if (!attempt || String(attempt.peer_id || "") !== id) continue;
      const outcome = String(attempt.outcome || "").toLowerCase();
      if ((outcome && outcome !== "ok") || hasText(attempt.reason_code) || hasText(attempt.stop_condition)) {
        return attempt;
      }
    }
    return null;
  };

  const failureSummary = (failure) => {
    if (!failure) return "-";
    const parts = [];
    if (hasText(failure.stage)) parts.push(`stage=${failure.stage}`);
    if (hasText(failure.reason_code)) parts.push(`reason=${failure.reason_code}`);
    if (hasText(failure.stop_condition)) parts.push(`stop=${failure.stop_condition}`);
    if (hasText(failure.outcome)) parts.push(`outcome=${failure.outcome}`);
    if (hasText(failure.bucket)) parts.push(`bucket=${failure.bucket}`);
    return parts.length ? parts.join(" | ") : "-";
  };

  const statusForMember = (mem) => {
    if (!mem) return { label: "none", cls: "chip-muted" };
    if (mem.revoked) return { label: "revoked", cls: "chip-revoked" };
    if (String(mem.peer_id || "") === String(self().peer_id || "")) return { label: "this node", cls: "chip-role" };
    if (activeNeighbor(mem.peer_id)) return { label: "active", cls: "chip-active" };
    if (selectedNeighbor(mem.peer_id)) return { label: "target", cls: "chip-muted" };
    return { label: "known", cls: "chip-muted" };
  };

  const mergeItems = (current, incoming) => {
    const out = [];
    const seen = new Set();
    for (const item of [...current, ...incoming]) {
      if (!item) continue;
      const termID = String(item.term_id || "").trim();
      const message = String(item.message || "").trim();
      const key = `${termID}\n${message}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push(item);
    }
    return out;
  };

  const mergeTask = (current, incoming) => {
    if (!current) return incoming || null;
    if (!incoming) return current;
    const merged = { ...current, ...incoming };
    const currentFacts = Array.isArray(current.facts) ? current.facts : [];
    const incomingFacts = Array.isArray(incoming.facts) ? incoming.facts : [];
    merged.facts = mergeItems(currentFacts, incomingFacts);
    const currentSuggestions = Array.isArray(current.suggestions) ? current.suggestions : [];
    const incomingSuggestions = Array.isArray(incoming.suggestions) ? incoming.suggestions : [];
    merged.suggestions = mergeItems(currentSuggestions, incomingSuggestions);
    if (current.report_ready && !incoming.report_ready) merged.report_ready = true;
    if (current.status === "done" && incoming.status !== "done") {
      merged.status = current.status;
      merged.reason_code = current.reason_code;
      merged.exit_code = current.exit_code;
    }
    if (current.stage && !incoming.stage) merged.stage = current.stage;
    return merged;
  };

  const upsertTask = (taskObj) => {
    const taskID = canonicalTaskID(taskObj && taskObj.task_id);
    if (!taskID) return "";
    const merged = mergeTask(state.tasks.get(taskID), taskObj);
    state.tasks.set(taskID, merged);
    syncApprovalDecisionTask(merged);
    return taskID;
  };

  const syncApprovalDecisionTask = (taskObj) => {
    const taskID = canonicalTaskID(taskObj && taskObj.task_id);
    const key = taskID ? approvalDecisionTasks.get(taskID) : "";
    if (!key) return;
    const current = approvalDecisionState.get(key) || {};
    const status = String(taskObj && taskObj.status || "").toLowerCase();
    const reason = String(taskObj && taskObj.reason_code || "").trim();
    if (status !== "done") {
      approvalDecisionState.set(key, { ...current, busy: true, taskID });
      return;
    }
    if (reason && reason !== "OK") {
      approvalDecisionState.set(key, {
        ...current,
        busy: false,
        taskID,
        failure: taskFailureSummary(taskObj),
      });
      return;
    }
    approvalDecisionState.set(key, { ...current, busy: false, taskID, failure: "", message: "Decision sent" });
  };

  const attachPeerFact = (taskObj, peerID) => {
    const id = String(peerID || "").trim();
    if (!taskObj || !id) return taskObj;
    const facts = Array.isArray(taskObj.facts) ? taskObj.facts.slice() : [];
    if (!facts.some((f) => String(f && f.message ? f.message : "").includes(id))) {
      facts.push({ message: `peer_id=${id}` });
    }
    taskObj.facts = facts;
    return taskObj;
  };

  const taskStatusClass = (taskObj) => {
    const status = String(taskObj && taskObj.status ? taskObj.status : "").toLowerCase();
    const reason = String(taskObj && taskObj.reason_code ? taskObj.reason_code : "").toLowerCase();
    if (reason && reason !== "ok") return "chip-error";
    if (status === "done") return "chip-done";
    if (status === "running" || status === "pending") return "chip-running";
    return "";
  };

  const taskFailureSummary = (taskObj) => {
    if (!taskObj) return "decision task failed";
    const reason = String(taskObj.reason_code || "").trim();
    const facts = Array.isArray(taskObj.facts) ? taskObj.facts : [];
    const messages = facts.map((f) => String(f && f.message || "").trim()).filter(Boolean);
    const detail = messages.find((msg) => /failed|error|invalid|conflict/i.test(msg)) || messages[0] || "";
    return [reason, detail].filter(Boolean).join(": ") || "decision task failed";
  };

  const compactStatusText = (value, max = 160) => {
    const text = String(value || "").replace(/\s+/g, " ").trim();
    if (!text) return "";
    if (text.length <= max) return text;
    return `${text.slice(0, Math.max(0, max - 3)).trimEnd()}...`;
  };

  const latestShellRuntimeSession = (peerID) => {
    const id = String(peerID || "").trim();
    if (!id) return null;
    const sessions = Array.isArray(state.shellSessions) ? state.shellSessions : [];
    return sessions
      .filter((item) => String(item && item.peer_id || "").trim() === id)
      .sort((a, b) => String(b && b.created_at || "").localeCompare(String(a && a.created_at || "")))[0] || null;
  };

  const shellDefaultTarget = (peerID) => {
    const runtime = latestShellRuntimeSession(peerID);
    const target = runtime && runtime.target ? String(runtime.target).trim() : "";
    return target || "local";
  };

  const shellDefaultSession = (peerID) => {
    const runtime = latestShellRuntimeSession(peerID);
    const session = runtime && runtime.session ? String(runtime.session).trim() : "";
    return session || "main";
  };

  const rememberShellSelection = (peerID = shellView.peerID) => {
    const id = String(peerID || "").trim();
    if (!id) return;
    shellSelections.set(id, {
      target: shellView.target,
      session: shellView.session,
      targetOptions: Array.isArray(shellView.targetOptions) ? shellView.targetOptions.slice() : [],
      sessionOptions: Array.isArray(shellView.sessionOptions) ? shellView.sessionOptions.slice() : [],
      sessionTarget: shellView.sessionTarget,
    });
  };

  const shellSeedForPeer = (peerID) => {
    const runtime = latestShellRuntimeSession(peerID);
    const target = shellDefaultTarget(peerID);
    const session = shellDefaultSession(peerID);
    return {
      target,
      session,
      targetOptions: runtime && hasText(runtime.target) ? [String(runtime.target).trim()] : [],
      sessionOptions: runtime && hasText(runtime.session) ? [String(runtime.session).trim()] : [],
      sessionTarget: runtime && hasText(runtime.target) && hasText(runtime.session) ? String(runtime.target).trim() : "",
    };
  };

  const syncShellSelectionForPeer = (peerID) => {
    const id = String(peerID || "").trim();
    if (!id) return;
    if (shellView.peerID && shellView.peerID !== id) rememberShellSelection(shellView.peerID);

    if (shellView.peerID === id) {
      if (!hasText(shellView.target)) shellView.target = shellDefaultTarget(id);
      if (!hasText(shellView.session)) shellView.session = shellDefaultSession(id);
      rememberShellSelection(id);
      return;
    }

    const saved = shellSelections.get(id);
    const seed = saved || shellSeedForPeer(id);
    shellView.peerID = id;
    shellView.target = hasText(seed.target) ? String(seed.target).trim() : shellDefaultTarget(id);
    shellView.session = hasText(seed.session) ? String(seed.session).trim() : shellDefaultSession(id);
    shellView.targetOptions = Array.isArray(seed.targetOptions) ? seed.targetOptions.slice() : [];
    shellView.sessionOptions = Array.isArray(seed.sessionOptions) ? seed.sessionOptions.slice() : [];
    shellView.sessionTarget = hasText(seed.sessionTarget) ? String(seed.sessionTarget).trim() : "";
    shellView.error = "";
    shellView.detail = "";
    shellView.phase = "idle";
    shellView.taskID = "";
    shellView.discoveryTaskID = "";
    rememberShellSelection(id);
  };

  const shellPageActive = () => state.activeTab === "network" && state.view.type === "peer" && state.view.section === "shell";

  const shellOperateEnabled = (peerID = shellView.peerID) => {
    const mem = memberByPeerID(peerID);
    const id = String(peerID || "").trim();
    const selfPeerID = String(self().peer_id || "");
    return !!(mem && id && id !== selfPeerID && !mem.revoked && (state.previewMode || lastConn && lastConn.connected));
  };

  const shellPhaseClass = (phase) => {
    if (phase === "failed") return "chip-error";
    if (phase === "connected") return "chip-done";
    if (phase === "listing" || phase === "connecting") return "chip-running";
    return "chip-muted";
  };

  const shellPhaseDefaultDetail = (phase) => {
    if (phase === "listing") return "Listing shell choices...";
    if (phase === "connecting") return "Connecting shell...";
    if (phase === "connected") return "Shell connected.";
    if (phase === "disconnected") return "Shell disconnected. Reconnect is available.";
    if (phase === "failed") return "Shell action failed. Retry is available.";
    return "Ready to discover or connect.";
  };

  const shellStatusText = () => shellView.detail || shellPhaseDefaultDetail(shellView.phase);

  const shellCanDiscover = (peerID = shellView.peerID) => shellOperateEnabled(peerID)
    && !["listing", "connecting", "connected"].includes(shellView.phase);

  const shellCanConnect = (peerID = shellView.peerID) => shellOperateEnabled(peerID)
    && !["listing", "connecting", "connected"].includes(shellView.phase);

  const shellCanDisconnect = () => !!(shellState.ws || shellState.term);

  const detachShellTerminalDOM = () => {
    if (!shellPageActive() || !shellState.term) return null;
    const container = el("terminal");
    if (!container || !container.firstChild) return null;
    const fragment = document.createDocumentFragment();
    while (container.firstChild) fragment.appendChild(container.firstChild);
    return { fragment };
  };

  const restoreShellTerminalDOM = (preserved) => {
    if (!preserved || !shellPageActive() || !shellState.term) return;
    const container = el("terminal");
    if (!container) return;
    container.textContent = "";
    container.appendChild(preserved.fragment);
    if (shellState.resizeObs) {
      try {
        shellState.resizeObs.disconnect();
        shellState.resizeObs.observe(container);
      } catch {
        // ignore resize observer races while the shell view is rebuilding
      }
    }
    try {
      shellState.term.focus();
    } catch {
      // ignore focus races if xterm has already been disposed
    }
    if (shellState.fitTimer) window.clearTimeout(shellState.fitTimer);
    shellState.fitTimer = window.setTimeout(fitAndSendWinSize, 80);
  };

  const shellTaskValues = (taskObj, termID, prefix) => {
    const values = [];
    const seen = new Set();
    const facts = taskObj && Array.isArray(taskObj.facts) ? taskObj.facts : [];
    for (const fact of facts) {
      const message = String(fact && fact.message || "").trim();
      const factTermID = String(fact && fact.term_id || "").trim();
      let value = "";
      if (factTermID === termID && hasText(message)) {
        value = message.startsWith(prefix) ? message.slice(prefix.length).trim() : message;
      } else if (message.startsWith(prefix)) {
        value = message.slice(prefix.length).trim();
      }
      if (!value || seen.has(value)) continue;
      seen.add(value);
      values.push(value);
    }
    return values;
  };

  const shellTaskValue = (taskObj, termID, prefix) => shellTaskValues(taskObj, termID, prefix)[0] || "";

  const shellTaskFailed = (taskObj) => {
    const status = String(taskObj && taskObj.status || "").toLowerCase();
    const reason = String(taskObj && taskObj.reason_code || "").trim().toUpperCase();
    return status === "done" && !!reason && reason !== "OK";
  };

  const shellTaskDiagnosticSummary = (taskObj) => {
    if (!taskObj) return "";
    const reason = String(taskObj.reason_code || "").trim();
    const layer = shellTaskValue(taskObj, "shell_layer", "shell_layer=");
    let detail = shellTaskValue(taskObj, "shell_close", "shell_close=");
    if (!detail) detail = taskFailureSummary(taskObj);
    const lowerDetail = detail.toLowerCase();
    const lowerLayer = String(layer || "").toLowerCase();
    if (detail && layer && !lowerDetail.startsWith(`${lowerLayer} `) && !lowerDetail.startsWith(`${lowerLayer}:`)) {
      detail = `${layer}: ${detail}`;
    }
    return compactStatusText([reason, detail].filter(Boolean).join(": ") || taskFailureSummary(taskObj));
  };

  const shellSocketCloseReason = (event, wsError = "") => compactStatusText(
    String(wsError || "").trim()
      || (event && event.reason ? String(event.reason) : "")
      || (event && event.code ? `websocket closed (${event.code})` : "terminal websocket closed")
  );

  const closeShellTransport = (closeCode = 1000, reason = "bye") => {
    if (shellState.resizeObs) {
      shellState.resizeObs.disconnect();
      shellState.resizeObs = null;
    }
    if (shellState.fitTimer) {
      window.clearTimeout(shellState.fitTimer);
      shellState.fitTimer = 0;
    }
    const ws = shellState.ws;
    shellState.ws = null;
    if (ws) {
      const readyState = typeof ws.readyState === "number" ? ws.readyState : WebSocket.CLOSED;
      if (readyState < WebSocket.CLOSING) {
        shellState.expectedClose = true;
        try {
          ws.close(closeCode, reason);
        } catch {
          // ignore close race
        }
      } else {
        shellState.expectedClose = false;
      }
    } else {
      shellState.expectedClose = false;
    }
    if (shellState.term) {
      try {
        shellState.term.dispose();
      } catch {
        // ignore dispose race
      }
      shellState.term = null;
    }
    shellState.taskID = "";
    shellState.wsError = "";
    shellState.remoteDataSeen = false;
  };

  const syncShellDOM = () => {
    if (!shellPageActive()) return;
    const phase = el("shell-phase");
    if (phase) {
      phase.textContent = shellView.phase;
      phase.className = `chip ${shellPhaseClass(shellView.phase)}`.trim();
    }
    const status = el("shell-status");
    if (status) status.textContent = shellStatusText();
    const error = el("shell-error");
    if (error) {
      error.textContent = shellView.error || "";
      error.classList.toggle("is-hidden", !shellView.error);
    }
    const discover = el("btn-shell-discover");
    if (discover) discover.disabled = !shellCanDiscover();
    const connect = el("btn-shell-connect");
    if (connect) connect.disabled = !shellCanConnect();
    const disconnect = el("btn-shell-disconnect");
    if (disconnect) disconnect.disabled = !shellCanDisconnect();
  };

  const failShellAction = (message, detail = "") => {
    closeShellTransport();
    shellView.phase = "failed";
    shellView.detail = detail || shellPhaseDefaultDetail("failed");
    shellView.error = String(message || "Shell action failed");
    rememberShellSelection();
    scheduleRender();
    toast(shellView.error);
  };

  const applyEventToTask = (taskObj, ev) => {
    if (!taskObj || !ev) return;
    const kind = String(ev.kind || "");
    if (kind === "stage") {
      if (ev.stage) taskObj.stage = ev.stage;
      return;
    }
    if (kind === "fact") {
      if (ev.fact) {
        taskObj.facts = Array.isArray(taskObj.facts) ? taskObj.facts : [];
        taskObj.facts.push(ev.fact);
      }
      return;
    }
    if (kind === "diagnosis") {
      if (ev.suggestion) {
        taskObj.suggestions = Array.isArray(taskObj.suggestions) ? taskObj.suggestions : [];
        taskObj.suggestions.push(ev.suggestion);
      }
      return;
    }
    if (kind === "report_ready") {
      taskObj.report_ready = true;
      return;
    }
    if (kind === "done") {
      taskObj.status = "done";
      if (ev.reason_code) taskObj.reason_code = ev.reason_code;
      if (typeof ev.exit_code !== "undefined") taskObj.exit_code = ev.exit_code;
    }
  };

  const applyDesktopSnapshot = (snapshot) => {
    if (!snapshot || typeof snapshot !== "object") return;

    state.rev = Number(snapshot.rev || 0);
    state.status = snapshot.status || null;
    state.topology = snapshot.topology || null;
    state.peerSessions = Array.isArray(snapshot.peer_sessions) ? snapshot.peer_sessions : [];
    state.shellSessions = Array.isArray(snapshot.shell_sessions) ? snapshot.shell_sessions : [];
    state.config = snapshot.config || { known_peers: [] };
    state.diagnostics = Array.isArray(snapshot.diagnostics) ? snapshot.diagnostics : [];
    state.approvalRequests = Array.isArray(snapshot.approval_requests) ? snapshot.approval_requests : [];

    const nextTasks = new Map();
    const tasks = Array.isArray(snapshot.tasks) ? snapshot.tasks : [];
    for (const taskObj of tasks) {
      const taskID = canonicalTaskID(taskObj && taskObj.task_id);
      if (!taskID) continue;
      const merged = mergeTask(state.tasks.get(taskID), taskObj);
      nextTasks.set(taskID, merged);
      syncApprovalDecisionTask(merged);
    }
    state.tasks = nextTasks;
  };

  const applyDesktopStateUpdate = (ev) => {
    const kind = String(ev.kind || "");
    if (kind === "task.upsert") {
      if (ev.task) {
        upsertTask({
          ...ev.task,
          task_id: canonicalTaskID(ev.task.task_id),
        });
      }
    } else if (kind === "topology.replace") {
      state.topology = ev.topology || null;
    } else if (kind === "peer_sessions.replace") {
      state.peerSessions = Array.isArray(ev.peer_sessions) ? ev.peer_sessions : [];
    } else if (kind === "shell_sessions.replace") {
      state.shellSessions = Array.isArray(ev.shell_sessions) ? ev.shell_sessions : [];
    } else if (kind === "config.replace") {
      state.config = ev.config || { known_peers: [] };
    } else if (kind === "diagnostics.replace") {
      state.diagnostics = Array.isArray(ev.diagnostics) ? ev.diagnostics : [];
    } else if (kind === "approval_requests.replace") {
      state.approvalRequests = Array.isArray(ev.approval_requests) ? ev.approval_requests : [];
    } else {
      return false;
    }

    state.rev = Number(ev.rev || state.rev);
    return true;
  };

  const replayRuntimeRecoveryQueue = () => {
    const queued = runtimeRecoveryQueue;
    runtimeRecoveryQueue = [];
    for (const queuedEvent of queued) {
      const baseRev = Number(queuedEvent && queuedEvent.base_rev);
      if (!Number.isFinite(baseRev) || baseRev !== state.rev) continue;
      applyDesktopStateUpdate(queuedEvent);
    }
  };

  const markRuntimeStreamReady = () => {
    runtimeStream = { ready: true, status: "live", error: null };
  };

  const markRuntimeStreamFailed = (error, status = "failed") => {
    runtimeStream = { ready: false, status, error: error || null };
  };

  const markRuntimeStreamRetrying = (error) => {
    const wasRetrying = runtimeStream.status === "retrying";
    runtimeStream = { ready: runtimeStream.ready, status: "retrying", error: error || null };
    if (!wasRetrying) toast(`Runtime stream retrying: ${bridgeErrorSummary(error)}`);
  };

  const handleDesktopStateEvent = (ev) => {
    if (!ev || typeof ev !== "object") return;
    if (runtimeRecoveryInFlight) {
      runtimeRecoveryQueue.push(ev);
      return;
    }

    markRuntimeStreamReady();

    const kind = String(ev.kind || "");
    if (kind === "snapshot") {
      if (ev.snapshot) applyDesktopSnapshot(ev.snapshot);
      scheduleRender();
      return;
    }

    const baseRev = Number(ev.base_rev);
    if (Number.isFinite(baseRev) && baseRev !== state.rev) {
      void recoverDesktopRuntimeFromGap({ silent: true });
      return;
    }

    if (!applyDesktopStateUpdate(ev)) return;
    scheduleRender();
  };

  const handleDesktopRuntimeEvent = (ev) => {
    if (!ev || typeof ev !== "object") return;
    if (ev.connection) renderConnection(ev.connection);
    const kind = String(ev.kind || "");
    if (kind === "stream_retrying") {
      markRuntimeStreamRetrying(ev.error || null);
    } else if (kind === "connection" && ev.connection && !ev.connection.connected) {
      markRuntimeStreamFailed(ev.connection.failure || null, "disconnected");
    }
  };

  const renderConnection = (conn) => {
    lastConn = conn || null;
  };

  const loadPreviewFixture = (name) => {
    const selected = previewFixtures[name] ? name : "owner";
    const fx = clone(previewFixtures[selected]);
    state.previewFixture = selected;
    state.rev = 0;
    state.status = fx.status || null;
    state.topology = fx.topology || null;
    state.tasks = new Map((fx.tasks || []).map((t) => [canonicalTaskID(t.task_id), t]));
    state.peerSessions = Array.isArray(fx.peer_sessions) ? fx.peer_sessions : [];
    state.shellSessions = Array.isArray(fx.shell_sessions) ? fx.shell_sessions : [];
    state.config = fx.config || { known_peers: [] };
    state.diagnostics = Array.isArray(fx.diagnostics) ? fx.diagnostics : [];
    state.approvalRequests = Array.isArray(fx.approval_requests) ? fx.approval_requests : [];
    renderConnection(fx.connection || null);
    scheduleRender();
  };

  const renderNav = () => {
    for (const btn of document.querySelectorAll(".nav-tab")) {
      const active = btn.dataset.tab === state.activeTab;
      btn.classList.toggle("is-active", active);
      if (active) btn.setAttribute("aria-current", "page");
      else btn.removeAttribute("aria-current");
    }
    for (const node of document.querySelectorAll("[data-admin-nav]")) {
      node.classList.toggle("is-hidden", !adminVisible());
    }
  };

  const renderTopbar = () => {
    const copy = COPY.tabs[state.activeTab] || COPY.tabs.network;
    const title = el("topbar-title");
    const eyebrow = el("topbar-eyebrow");
    if (title) title.textContent = copy.title;
    if (eyebrow) eyebrow.textContent = copy.eyebrow;
    const fixtureWrap = el("preview-fixture-wrap");
    if (fixtureWrap) fixtureWrap.classList.toggle("is-hidden", !state.previewMode);
  };

  const setActiveTab = (name, opts = {}) => {
    const next = name === "admin" && !adminVisible() ? "network" : (name || "network");
    if (state.activeTab !== next) disconnectShell();
    state.activeTab = next;
    state.view = { type: "overview" };
    if (!opts.skipStore) localStorage.setItem("miopunch_desktop_tab", next);
    scheduleRender();
  };

  const navigate = (view) => {
    const leavingShell = state.activeTab === "network" && state.view.type === "peer" && state.view.section === "shell";
    const enteringShell = state.activeTab === "network" && view.type === "peer" && view.section === "shell";
    const changingShellPeer = leavingShell && enteringShell
      && String(state.view.peerID || "") !== String(view.peerID || "");
    if (leavingShell && (!enteringShell || changingShellPeer)) disconnectShell();
    state.view = view;
    scheduleRender();
  };

  const backToOverview = () => {
    navigate({ type: "overview" });
  };

  const setPage = (html) => {
    const host = el("page-host");
    if (!host) return;
    const preservedTerminal = detachShellTerminalDOM();
    host.innerHTML = html;
    renderPostDOM(preservedTerminal);
  };

  const renderAll = () => {
    if (roleKnown() && !adminVisible() && state.activeTab === "admin") {
      state.activeTab = "network";
      state.view = { type: "overview" };
    }
    renderNav();
    renderTopbar();
    if (state.activeTab === "network") renderNetwork();
    else if (state.activeTab === "access") renderAccess();
    else if (state.activeTab === "admin") renderAdmin();
    else renderSettings();
  };

  const pageHeadingHTML = (eyebrow, title, subtitle = "", actions = "") => `
    <div class="page-heading">
      <div>
        <p class="eyebrow">${esc(eyebrow)}</p>
        <h2 class="page-title">${esc(title)}</h2>
        ${subtitle ? `<p class="page-subtitle">${esc(subtitle)}</p>` : ""}
      </div>
      ${actions ? `<div class="action-row">${actions}</div>` : ""}
    </div>`;

  const moduleSwitchHTML = (items) => `
    <nav class="module-switch" aria-label="Workspace view">
      ${items.map((item) => `
        <button class="module-switch-item ${item.active ? "is-active" : ""}" type="button" ${item.attr || ""}>
          <span>${esc(item.label)}</span>
          ${item.meta ? `<small>${esc(item.meta)}</small>` : ""}
        </button>`).join("")}
    </nav>`;

  const networkSwitchHTML = (activePeerID = "") => {
    const activeID = String(activePeerID || "");
    const items = [{
      label: "Overview",
      meta: "network",
      active: !activeID,
      attr: "data-open-overview",
    }];
    for (const mem of members()) {
      const peerID = String(mem.peer_id || "");
      if (!peerID) continue;
      items.push({
        label: shortID(peerID),
        meta: mem.role || "peer",
        active: peerID === activeID,
        attr: `data-open-peer="${esc(peerID)}"`,
      });
    }
    return moduleSwitchHTML(items);
  };

  const accessFlows = () => [
    { id: "join", title: "Join", meta: "this device", admin: false },
    { id: "invite", title: "Invite", meta: "new device", admin: true },
    { id: "approve", title: "Approve", meta: "request", admin: true },
  ].filter((flow) => !flow.admin || adminVisible());

  const accessSwitchHTML = (activeFlow = "") => moduleSwitchHTML([
    { label: "Overview", meta: "access", active: !activeFlow, attr: "data-open-overview" },
    ...accessFlows().map((flow) => ({
      label: flow.title,
      meta: flow.meta,
      active: flow.id === activeFlow,
      attr: `data-open-flow="${esc(flow.id)}"`,
    })),
  ]);

  const adminSwitchHTML = (activeMemberID = "") => {
    const activeID = String(activeMemberID || "");
    return moduleSwitchHTML([
      { label: "Overview", meta: "governance", active: !activeID, attr: "data-open-overview" },
      ...members().map((mem) => {
        const peerID = String(mem.peer_id || "");
        return {
          label: shortID(peerID),
          meta: mem.role || "member",
          active: peerID === activeID,
          attr: `data-open-member="${esc(peerID)}"`,
        };
      }),
    ]);
  };

  const settingSections = () => [
    { id: "localapi", title: "Local daemon", meta: "connection" },
    { id: "diagnostics", title: "Diagnostics", meta: "status" },
    { id: "preview", title: "Preview", meta: "fixture" },
  ];

  const settingsSwitchHTML = (activeSection = "") => moduleSwitchHTML([
    { label: "Overview", meta: "settings", active: !activeSection, attr: "data-open-overview" },
    ...settingSections().map((item) => ({
      label: item.title,
      meta: item.meta,
      active: item.id === activeSection,
      attr: `data-open-setting="${esc(item.id)}"`,
    })),
  ]);

  const renderNetwork = () => {
    if (state.view.type === "peer") {
      renderPeerDetail(memberByPeerID(state.view.peerID), state.view.section || "overview");
      return;
    }
    renderNetworkOverview();
  };

  const renderNetworkOverview = () => {
    const top = state.topology || {};
    const currentSelf = self();
    const list = members();
    const liveMembers = list.filter((m) => !m.revoked);
    const active = top.neighbors && Array.isArray(top.neighbors.active) ? top.neighbors.active : [];
    const targetK = top.neighbors && typeof top.neighbors.target_k !== "undefined" ? top.neighbors.target_k : "-";
    const stateHead = top.state_head || {};
    const net = top.net || {};
    const peerCards = list.length
      ? list.map((mem) => {
        const status = statusForMember(mem);
        const neighbor = activeNeighbor(mem.peer_id);
        const path = neighbor ? `${neighbor.data_proto || "-"} / ${neighbor.path_family || "-"}` : (mem.v4_hint || "path unknown");
        return `
          <button class="tile" data-open-peer="${esc(mem.peer_id || "")}">
            <div class="card-header">
              <div>
                <div class="tile-title">${esc(shortID(mem.peer_id))}</div>
                <div class="tile-meta">${esc(mem.role || "unknown")} | ${esc(path)}</div>
              </div>
              ${chipHTML(status.label, status.cls)}
            </div>
          </button>`;
      }).join("")
      : `<div class="card">${listItemHTML(COPY.empty.devices, "empty")}</div>`;

    setPage(`
      <section class="page">
        ${networkSwitchHTML()}
        ${pageHeadingHTML("Network overview", "Device network")}
        <section class="card">
          <div class="card-header">
            <div>
              <p class="eyebrow">This node</p>
              <h3 class="card-title">${esc(shortID(currentSelf.peer_id))}</h3>
            </div>
            ${chipHTML(currentSelf.role || "unknown", "chip-role")}
          </div>
          <div class="detail-table">
            ${detailRowHTML("Peer ID", currentSelf.peer_id || "-")}
            ${detailRowHTML("Network", net.net_id || "Not joined")}
            ${detailRowHTML("Version", state.status && state.status.version ? state.status.version : "-")}
            ${detailRowHTML("Uptime", state.status && state.status.uptime_ms ? fmtUptime(state.status.uptime_ms) : "-")}
          </div>
        </section>
        <section class="metric-grid">
          ${metricHTML("Members", liveMembers.length)}
          ${metricHTML("Active paths", active.filter((n) => n.healthy !== false).length)}
          ${metricHTML("Target K", targetK)}
          ${metricHTML("Decls head", headShort(stateHead.decls_head_b64))}
        </section>
        <section class="grid">
          <div class="card-header">
            <div>
              <p class="eyebrow">Peers</p>
              <h3 class="card-title">Members</h3>
            </div>
            ${chipHTML(list.length)}
          </div>
          <div class="peer-grid">${peerCards}</div>
        </section>
      </section>`);
  };

  const renderPeerDetail = (mem, section) => {
    const selected = mem || null;
    const peerID = selected && selected.peer_id ? selected.peer_id : "";
    const role = selected && selected.role ? selected.role : "-";
    const status = statusForMember(selected);
    const neighbor = selected ? activeNeighbor(selected.peer_id) : null;
    const selectedEdge = selected ? selectedNeighbor(selected.peer_id) : null;
    const selectedInactive = !!(selectedEdge && !neighbor);
    const recentFailure = selectedInactive ? recentPeerFailure(peerID) : null;
    const selfPeerID = String(self().peer_id || "");
    const isRemote = !!(selected && selected.peer_id && selected.peer_id !== selfPeerID);
    const canOperate = !!(selected && selected.peer_id && isRemote && !selected.revoked && (state.previewMode || lastConn && lastConn.connected));
    let body = "";

    if (!selected) {
      body = `<section class="card">${listItemHTML("Peer was not found", "empty")}</section>`;
    } else if (section === "shell") {
      syncShellSelectionForPeer(peerID);
      const shellTaskObj = state.tasks.get(shellView.taskID || shellView.discoveryTaskID) || null;
      const targetChoices = shellView.targetOptions.length
        ? `Discovered targets: ${shellView.targetOptions.join(", ")}`
        : "Defaults to target local until discovery returns a richer choice.";
      const sessionChoices = shellView.sessionOptions.length
        ? `Discovered sessions for ${shellView.sessionTarget || shellView.target || "current target"}: ${shellView.sessionOptions.join(", ")}`
        : "Defaults to session main until discovery returns a richer choice.";
      body = `
        <section class="card">
          <div class="card-header">
            <div>
              <p class="eyebrow">Remote session</p>
              <h3 class="card-title">Shell</h3>
            </div>
            <div class="action-row">
              <span class="chip ${shellPhaseClass(shellView.phase)}" id="shell-phase">${esc(shellView.phase)}</span>
              <button class="btn btn-tonal" id="btn-shell-disconnect" type="button" ${shellCanDisconnect() ? "" : "disabled"}>Disconnect</button>
            </div>
          </div>
          <form class="form-grid" id="shell-form">
            <div class="grid grid-3">
              <label>Peer ID<input class="textfield mono" id="shell-peer-id" value="${esc(peerID)}" autocomplete="off" readonly /></label>
              <label>Target<input class="textfield" id="shell-target" value="${esc(shellView.target || shellDefaultTarget(peerID))}" list="shell-target-options" autocomplete="off" /></label>
              <label>Session<input class="textfield" id="shell-session" value="${esc(shellView.session || shellDefaultSession(peerID))}" list="shell-session-options" autocomplete="off" /></label>
            </div>
            <datalist id="shell-target-options">
              ${shellView.targetOptions.map((value) => `<option value="${esc(value)}"></option>`).join("")}
            </datalist>
            <datalist id="shell-session-options">
              ${shellView.sessionOptions.map((value) => `<option value="${esc(value)}"></option>`).join("")}
            </datalist>
            <div class="grid grid-2">
              <div class="helper" id="shell-target-choices">${esc(targetChoices)}</div>
              <div class="helper" id="shell-session-choices">${esc(sessionChoices)}</div>
            </div>
            <div class="action-row">
              <button class="btn btn-tonal" id="btn-shell-discover" type="button" ${shellCanDiscover(peerID) ? "" : "disabled"}>Discover</button>
              <button class="btn btn-primary" id="btn-shell-connect" type="submit" ${shellCanConnect(peerID) ? "" : "disabled"}>Connect</button>
              <div class="helper" id="shell-status">${esc(shellStatusText())}</div>
            </div>
            <div class="helper helper-error ${shellView.error ? "" : "is-hidden"}" id="shell-error">${esc(shellView.error)}</div>
          </form>
          <div class="terminal mt" id="terminal"></div>
        </section>
        ${shellTaskObj ? renderTaskSummary(shellTaskObj) : ""}`;
    } else {
      body = `
        <section class="detail-grid">
          <div class="card">
            <div class="card-header">
              <div>
                <p class="eyebrow">Peer</p>
                <h3 class="card-title">${esc(shortID(peerID))}</h3>
              </div>
              ${chipHTML(status.label, status.cls)}
            </div>
            <div class="detail-table">
              ${detailRowHTML("Peer ID", peerID)}
              ${detailRowHTML("Status", status.label)}
              ${detailRowHTML("Connection", neighbor ? "active edge" : selectedInactive ? "target candidate" : "-")}
              ${detailRowHTML("Role", role)}
              ${detailRowHTML("IPv4", selected.v4_hint || "-")}
              ${detailRowHTML("IPv6", selected.v6_hint || "-")}
              ${detailRowHTML("Path", neighbor ? `${neighbor.data_proto || "-"} / ${neighbor.path_family || "-"}` : "-")}
              ${detailRowHTML("Selection", selectedEdge ? selectedEdge.reason || selectedEdge.bucket || "target" : "-")}
              ${recentFailure ? detailRowHTML("Recent failure", failureSummary(recentFailure)) : ""}
            </div>
          </div>
          <div class="grid">
            <div class="card">
              <div class="card-header">
                <div>
                  <p class="eyebrow">Path</p>
                  <h3 class="card-title">Current edge</h3>
                </div>
              </div>
              <div class="detail-table">
                ${detailRowHTML("Bucket", neighbor ? neighbor.bucket || "-" : selectedEdge ? selectedEdge.bucket || "-" : "-")}
                ${detailRowHTML("Dialable", selectedEdge && typeof selectedEdge.dialable !== "undefined" ? String(selectedEdge.dialable) : "-")}
                ${detailRowHTML("Healthy", neighbor && typeof neighbor.healthy !== "undefined" ? String(neighbor.healthy) : "-")}
              </div>
            </div>
            <div class="card">
              <div class="card-header">
                <div>
                  <p class="eyebrow">Actions</p>
                  <h3 class="card-title">Operate</h3>
                </div>
              </div>
              <div class="action-row">
                <button class="icon-btn" data-copy-peer="${esc(peerID)}" title="Copy Peer ID" aria-label="Copy Peer ID">
                  <svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M8 8h10v12H8z" /><path d="M6 16H4V4h12v2" /></svg>
                </button>
                <button class="btn btn-tonal" data-run-peer-task="ping" ${canOperate ? "" : "disabled"}>Ping</button>
                <button class="btn btn-tonal" data-run-peer-task="sh_ls" ${canOperate ? "" : "disabled"}>List sessions</button>
                <button class="btn btn-primary" data-peer-section="shell" data-peer-id="${esc(peerID)}" ${canOperate ? "" : "disabled"}>Shell</button>
              </div>
            </div>
          </div>
        </section>
        ${renderPeerTaskCard(peerID)}`;
    }

    setPage(`
      <section class="page">
        ${networkSwitchHTML(peerID)}
        ${pageHeadingHTML(section === "shell" ? "Peer shell" : "Peer detail", selected ? shortID(peerID) : "Peer")}
        ${body}
      </section>`);
  };

  const renderAccess = () => {
    if (state.view.type === "flow") {
      renderAccessFlow(state.view.flow || "join");
      return;
    }
    const flowTiles = [
      { id: "join", title: "Join network", meta: "Accept access on this device", admin: false },
      { id: "invite", title: "Create invite", meta: "Add another device", admin: true },
      { id: "approve", title: "Approve request", meta: "Process a join request", admin: true },
    ].filter((flow) => !flow.admin || adminVisible()).map((flow) => `
      <button class="tile" data-open-flow="${flow.id}">
        <div class="tile-title">${esc(flow.title)}</div>
        <div class="tile-meta">${esc(flow.meta)}</div>
      </button>`).join("");
    setPage(`
      <section class="page">
        ${accessSwitchHTML()}
        ${pageHeadingHTML("Access overview", "Access")}
        <div class="grid grid-3">${flowTiles}</div>
        ${renderApprovalRequestsPanel()}
        ${renderTaskCard("Recent tasks")}
      </section>`);
  };

  const renderAccessFlow = (flow) => {
    let body = "";
    if (flow === "invite") body = renderInviteFlow();
    else if (flow === "approve") body = renderApproveFlow();
    else body = renderJoinFlow();
    setPage(`
      <section class="page">
        ${accessSwitchHTML(flow)}
        ${pageHeadingHTML("Access flow", flow === "invite" ? "Create invite" : flow === "approve" ? "Approve request" : "Join network")}
        ${body}
      </section>`);
  };

  const renderJoinFlow = () => {
    const taskObj = joinState.taskID ? state.tasks.get(joinState.taskID) : null;
    return `
      <section class="card">
        <form class="form-grid" id="join-form">
          <label>Invite code or URL<input class="textfield" id="join-code" placeholder="Paste invite code here" autocomplete="off" /></label>
          <div class="action-row">
            <button class="btn btn-primary" type="submit">Join</button>
            <button class="btn btn-tonal" id="join-report-export" type="button" ${taskObj && taskObj.report_ready ? "" : "disabled"}>Export report</button>
          </div>
          <div class="helper" id="join-report-path">${esc(joinState.lastExportPath || "")}</div>
        </form>
      </section>
      ${renderTaskSummary(taskObj)}`;
  };

  const renderInviteFlow = () => {
    const taskObj = inviteState.taskID ? state.tasks.get(inviteState.taskID) : null;
    const code = findInviteCode(taskObj);
    const missingCode = taskObj && inviteState.missingTaskID === taskObj.task_id;
    const hint = inviteState.message || (code ? "Ready" : missingCode ? "Task completed with no invite code." : "");
    return `
      <section class="detail-grid">
        <div class="card">
          <div class="card-header">
            <div>
              <p class="eyebrow">Invite</p>
              <h3 class="card-title">Code</h3>
            </div>
            <button class="btn btn-primary" id="btn-invite" type="button" ${inviteState.busy || (!state.previewMode && !(lastConn && lastConn.connected)) ? "disabled" : ""}>Create</button>
          </div>
          <div class="form-grid">
            <label>Invite code<input class="textfield textfield-code" id="invite-code" readonly value="${esc(code)}" placeholder="Create an invite first" /></label>
            <div class="action-row">
              <button class="btn btn-tonal" data-copy-invite type="button" ${code ? "" : "disabled"}>Copy</button>
              <div class="helper" id="invite-hint">${esc(hint)}</div>
            </div>
          </div>
        </div>
        <div class="qr-wrap" id="invite-qr"></div>
      </section>
      ${renderTaskSummary(taskObj)}`;
  };

  const renderApproveFlow = () => {
    const taskObj = approveState.taskID ? state.tasks.get(approveState.taskID) : null;
    return `
      <section class="card">
        <form class="form-grid" id="approve-form">
          <label>Invite code<input class="textfield" id="approve-code" placeholder="Paste the invite code to approve" autocomplete="off" /></label>
          <div class="action-row">
            <button class="btn btn-primary" type="submit">Start approval</button>
            <div class="helper" id="approve-hint">${esc(approveState.message || "")}</div>
          </div>
        </form>
      </section>
      ${renderApprovalRequestsPanel()}
      ${renderTaskSummary(taskObj)}`;
  };

  const renderApprovalRequestsPanel = () => {
    if (!adminVisible()) return "";
    const requests = pendingApprovalRequests();
    const rows = requests.length ? requests.map((req) => {
      const key = approvalRequestKey(req);
      const approveTaskID = String(req.approve_task_id || req.task_id || "").trim();
      const requestMsgID = String(req.request_msg_id || "").trim();
      const status = approvalRequestStatus(req) || "pending";
      const decision = approvalDecisionState.get(key) || {};
      const busy = !!decision.busy && status === "pending";
      const failure = String(decision.failure || "");
      const displayName = String(req.member_name || "").trim();
      const peerLabel = displayName ? `${displayName} | ${shortID(req.member_peer_id)}` : shortID(req.member_peer_id);
      const hints = [
        req.platform ? `platform=${req.platform}` : "",
        req.v4_hint ? `v4=${req.v4_hint}` : "",
        req.v6_hint ? `v6=${req.v6_hint}` : "",
      ].filter(Boolean).join(" | ");
      const canDecide = adminVisible() && status === "pending";
      return `
        <div class="row-card approval-row">
          <div>
            <div class="row-title">${esc(peerLabel)}</div>
            <div class="row-meta">${esc(req.member_peer_id || "-")} | request=${esc(shortID(requestMsgID))}</div>
            ${hints ? `<div class="helper">${esc(hints)}</div>` : ""}
            ${failure ? `<div class="helper helper-error">${esc(failure)}</div>` : ""}
          </div>
          <div class="approval-actions">
            ${chipHTML(status, approvalStatusClass(status))}
            <button class="btn btn-tonal" type="button" data-approval-decision="reject" data-approve-task-id="${esc(approveTaskID)}" data-request-msg-id="${esc(requestMsgID)}" ${canDecide && !busy ? "" : "disabled"}>Reject</button>
            <button class="btn btn-primary" type="button" data-approval-decision="approve" data-approve-task-id="${esc(approveTaskID)}" data-request-msg-id="${esc(requestMsgID)}" ${canDecide && !busy ? "" : "disabled"}>Approve</button>
          </div>
        </div>`;
    }).join("") : listItemHTML("No pending approval requests", "empty");
    return `
      <section class="card">
        <div class="card-header">
          <div><p class="eyebrow">Review</p><h3 class="card-title">Approval requests</h3></div>
          ${chipHTML(requests.length)}
        </div>
        <div class="row-list">${rows}</div>
      </section>`;
  };

  const renderAdmin = () => {
    if (!adminVisible()) {
      setPage(`<section class="page">${pageHeadingHTML("Admin", "Unavailable")}<section class="card">${listItemHTML("Administrator controls are available only on owner or admin nodes", "empty")}</section></section>`);
      return;
    }
    if (state.view.type === "member") {
      renderMemberDetail(memberByPeerID(state.view.memberID));
      return;
    }
    const list = members();
    const owners = list.filter((m) => m.role === "owner").length;
    const admins = list.filter((m) => m.role === "admin").length;
    const revoked = list.filter((m) => m.revoked).length;
    const stateHead = state.topology && state.topology.state_head ? state.topology.state_head : {};
    const memberRows = list.length ? list.map((mem) => {
      const status = statusForMember(mem);
      return `
        <button class="row-card" data-open-member="${esc(mem.peer_id || "")}">
          <div>
            <div class="row-title">${esc(shortID(mem.peer_id))}</div>
            <div class="row-meta">${esc(mem.peer_id || "-")} | ${esc(mem.v4_hint || "path unknown")}</div>
          </div>
          <div class="action-row">
            ${chipHTML(mem.role || "unknown", mem.role === "owner" || mem.role === "admin" ? "chip-role" : "")}
            ${chipHTML(status.label, status.cls)}
          </div>
        </button>`;
    }).join("") : listItemHTML("No members", "empty");
    setPage(`
      <section class="page">
        ${adminSwitchHTML()}
        ${pageHeadingHTML("Admin overview", "Governance")}
        <section class="metric-grid">
          ${metricHTML("Owners", owners)}
          ${metricHTML("Admins", admins)}
          ${metricHTML("Revoked", revoked)}
          ${metricHTML("Governance head", headShort(stateHead.governance_head_b64))}
        </section>
        <section class="grid">
          <div class="card-header">
            <div>
              <p class="eyebrow">Members</p>
              <h3 class="card-title">Access control</h3>
            </div>
            ${chipHTML(list.length)}
          </div>
          <div class="row-list">${memberRows}</div>
        </section>
      </section>`);
  };

  const renderMemberDetail = (mem) => {
    const selfPeerID = String(self().peer_id || "");
    const canRevoke = !!(mem && !mem.revoked && mem.peer_id !== selfPeerID && mem.role !== "owner" && mem.role !== "admin");
    const status = statusForMember(mem);
    setPage(`
      <section class="page">
        ${adminSwitchHTML(mem && mem.peer_id)}
        ${pageHeadingHTML("Member detail", mem ? shortID(mem.peer_id) : "Member")}
        <section class="detail-grid">
          <div class="card">
            <div class="card-header">
              <div>
                <p class="eyebrow">Member</p>
                <h3 class="card-title">${esc(mem ? shortID(mem.peer_id) : "-")}</h3>
              </div>
              ${chipHTML(status.label, status.cls)}
            </div>
            <div class="detail-table">
              ${detailRowHTML("Peer ID", mem && mem.peer_id)}
              ${detailRowHTML("Role", mem && mem.role)}
              ${detailRowHTML("IPv4", mem && mem.v4_hint)}
              ${detailRowHTML("IPv6", mem && mem.v6_hint)}
              ${detailRowHTML("Status", status.label)}
            </div>
          </div>
          <div class="card">
            <div class="card-header">
              <div>
                <p class="eyebrow">Governance</p>
                <h3 class="card-title">Access</h3>
              </div>
            </div>
            <button class="btn btn-tonal" data-revoke-member="${esc(mem && mem.peer_id ? mem.peer_id : "")}" ${canRevoke ? "" : "disabled"}>Revoke</button>
          </div>
        </section>
        ${renderTaskCard("Recent tasks")}
      </section>`);
  };

  const renderSettings = () => {
    if (state.view.type === "section") {
      renderSettingsSection(state.view.section || "localapi");
      return;
    }
    const sections = settingSections().map((item) => `
      <button class="tile" data-open-setting="${item.id}">
        <div class="tile-title">${esc(item.title)}</div>
        <div class="tile-meta">${esc(item.meta)}</div>
      </button>`).join("");
    setPage(`
      <section class="page">
        ${settingsSwitchHTML()}
        ${pageHeadingHTML("Settings overview", "Settings")}
        <section class="metric-grid">
          ${metricHTML("Version", state.status && state.status.version ? state.status.version : "-")}
          ${metricHTML("Uptime", state.status && state.status.uptime_ms ? fmtUptime(state.status.uptime_ms) : "-")}
          ${metricHTML("Mode", state.previewMode ? "preview" : "desktop")}
          ${metricHTML("Tasks", state.tasks.size)}
        </section>
        <div class="grid grid-3">${sections}</div>
        <section class="card">
          <div class="action-row">
            <button class="btn btn-tonal" id="btn-app-quit" type="button" ${state.previewMode ? "disabled" : ""}>Quit</button>
          </div>
        </section>
      </section>`);
  };

  const renderSettingsSection = (section) => {
    let body = "";
    if (section === "diagnostics") {
      const failure = lastConn && lastConn.failure ? lastConn.failure : null;
      const suggestions = failure && Array.isArray(failure.suggestions) ? failure.suggestions : [];
      const facts = failure && Array.isArray(failure.facts) ? failure.facts : [];
      const diagnostics = lastConn && Array.isArray(lastConn.diagnostics) ? lastConn.diagnostics : [];
      const runtimeDiagnostics = Array.isArray(state.diagnostics) ? state.diagnostics : [];
      const runtimeStreamFacts = state.previewMode
        ? []
        : [
          `desktop_stream=${runtimeStream.status}`,
          runtimeStream.error ? `desktop_stream_error=${runtimeStream.error.reason_code || bridgeErrorSummary(runtimeStream.error)}` : "",
        ].filter(Boolean);
      const bootstrap = lastConn && lastConn.bootstrap ? lastConn.bootstrap : null;
      const daemonOwnership = lastConn && lastConn.desktop_managed ? "desktop-managed" : lastConn && lastConn.connected ? "reused" : "-";
      body = `
        <section class="detail-grid">
          <div class="card">
            <div class="card-header"><div><p class="eyebrow">Suggestions</p><h3 class="card-title">Connection</h3></div></div>
            <div class="list">
              ${failure && failure.message ? listItemHTML(failure.message) : ""}
              ${suggestions.length ? suggestions.map((s) => listItemHTML(s.message || "")).join("") : listItemHTML(COPY.empty.errors, "empty")}
            </div>
          </div>
          <div class="card">
            <div class="card-header"><div><p class="eyebrow">Facts</p><h3 class="card-title">LocalAPI</h3></div></div>
            <div class="list">
              ${listItemHTML(`mode=${state.previewMode ? "static preview" : "connected"}`)}
              ${lastConn ? listItemHTML(`selected=${lastConn.selected || "-"}`) : ""}
              ${lastConn ? listItemHTML(`desktop_managed=${daemonOwnership}`) : ""}
              ${lastConn && lastConn.bootstrap_state ? listItemHTML(`bootstrap_state=${lastConn.bootstrap_state}`) : ""}
              ${failure ? listItemHTML(`reason_code=${failure.reason_code || "-"}`) : ""}
              ${bootstrap && bootstrap.stage ? listItemHTML(`bootstrap_stage=${bootstrap.stage}`) : ""}
              ${bootstrap && bootstrap.daemon_path ? listItemHTML(`daemon_path=${bootstrap.daemon_path}`) : ""}
              ${bootstrap && bootstrap.pid ? listItemHTML(`pid=${bootstrap.pid}`) : ""}
              ${bootstrap && bootstrap.error ? listItemHTML(`bootstrap_error=${bootstrap.error}`) : ""}
              ${runtimeStreamFacts.map((message) => listItemHTML(message)).join("")}
              ${runtimeDiagnostics.map((f) => listItemHTML(f.message || "")).join("")}
              ${diagnostics.map((f) => listItemHTML(f.message || "")).join("")}
              ${facts.map((f) => listItemHTML(f.message || "")).join("")}
            </div>
          </div>
        </section>`;
    } else if (section === "preview") {
      body = `
        <section class="card">
          <div class="detail-table">
            ${detailRowHTML("Preview mode", state.previewMode ? "enabled" : "disabled")}
            ${detailRowHTML("Fixture", state.previewFixture)}
          </div>
        </section>`;
    } else {
      const override = lastConn && typeof lastConn.override_addr === "string" ? lastConn.override_addr : "";
      body = `
        <section class="card">
          <form class="form-grid" id="localapi-form">
            <label>Override address<input class="textfield" id="localapi-override" value="${esc(override)}" placeholder="unix:/path/to/localapi.sock" autocomplete="off" ${state.previewMode ? "disabled" : ""} /></label>
            <div class="action-row">
              <button class="btn btn-primary" id="btn-localapi-apply" type="submit" ${state.previewMode ? "disabled" : ""}>Apply</button>
              <button class="btn btn-tonal" id="btn-localapi-clear" type="button" ${state.previewMode ? "disabled" : ""}>Clear</button>
            </div>
            <div class="helper" id="localapi-known">${esc(localAPIKnownText())}</div>
          </form>
        </section>`;
    }
    setPage(`
      <section class="page">
        ${settingsSwitchHTML(section)}
        ${pageHeadingHTML("Settings detail", section === "diagnostics" ? "Diagnostics" : section === "preview" ? "Preview" : "Local daemon")}
        ${body}
      </section>`);
  };

  const localAPIKnownText = () => {
    if (state.previewMode) return "Static preview mode; LocalAPI controls are disabled.";
    const sys = lastConn && lastConn.system_addr ? String(lastConn.system_addr) : "(unknown)";
    const user = lastConn && lastConn.user_addr ? String(lastConn.user_addr) : "(unknown)";
    const ov = lastConn && lastConn.override_addr ? String(lastConn.override_addr) : "";
    const managed = lastConn && lastConn.desktop_managed ? "desktop-managed" : "reused";
    return ov ? `override=${ov} | system=${sys} | user=${user}` : `user=${user} | system=${sys} | daemon=${managed}`;
  };

  const renderTaskSummary = (taskObj) => `
    <section class="card">
      <div class="card-header">
        <div><p class="eyebrow">Task</p><h3 class="card-title">${esc(taskObj && taskObj.task_id ? taskObj.task_id : "-")}</h3></div>
        ${chipHTML(taskObj && taskObj.status ? taskObj.status : "-", taskStatusClass(taskObj))}
      </div>
      <div class="detail-table">
        ${detailRowHTML("Stage", taskObj && taskObj.stage)}
        ${detailRowHTML("Reason", taskObj && taskObj.reason_code)}
      </div>
      <div class="grid grid-2 mt">
        <div class="list">${renderFactList(taskObj && taskObj.suggestions, COPY.empty.suggestions)}</div>
        <div class="list">${renderFactList(taskObj && taskObj.facts, COPY.empty.facts)}</div>
      </div>
    </section>`;

  const renderFactList = (arr, emptyText) => {
    const list = Array.isArray(arr) ? arr : [];
    if (list.length === 0) return listItemHTML(emptyText, "empty");
    return list.map((x) => listItemHTML(x.message || "")).join("");
  };

  const taskMentionsPeer = (taskObj, peerID) => {
    const id = String(peerID || "").trim();
    if (!taskObj || !id) return false;
    const messages = [
      ...(Array.isArray(taskObj.facts) ? taskObj.facts : []),
      ...(Array.isArray(taskObj.suggestions) ? taskObj.suggestions : []),
    ].map((item) => String(item && item.message ? item.message : ""));
    return messages.some((msg) => msg.includes(id));
  };

  const renderPeerTaskCard = (peerID) => renderTaskCard(
    "Recent peer tasks",
    (taskObj) => taskMentionsPeer(taskObj, peerID)
  );

  const renderTaskCard = (title, predicate = null) => {
    const tasks = [...state.tasks.values()].filter((taskObj) =>
      typeof predicate === "function" ? predicate(taskObj) : true
    ).sort((a, b) =>
      String(b.created_at || "").localeCompare(String(a.created_at || ""))
    );
    const body = tasks.length ? tasks.map((t) => `
      <div class="row-card">
        <div>
          <div class="row-title">${esc(t.kind || "(unknown)")}</div>
          <div class="row-meta">${esc(t.task_id || "")} | stage=${esc(t.stage || "-")}</div>
        </div>
        ${chipHTML(t.status || "-", taskStatusClass(t))}
      </div>`).join("") : listItemHTML(COPY.empty.tasks, "empty");
    return `
      <section class="card">
        <div class="card-header">
          <div><p class="eyebrow">Activity</p><h3 class="card-title">${esc(title)}</h3></div>
          ${chipHTML(tasks.length)}
        </div>
        <div class="row-list">${body}</div>
      </section>`;
  };

  const findInviteCode = (taskObj) => {
    if (!taskObj || !Array.isArray(taskObj.facts)) return "";
    for (const f of taskObj.facts) {
      const msg = String(f && f.message ? f.message : "").trim();
      const termID = String(f && f.term_id ? f.term_id : "").trim();
      const prefix = "invite_code=";
      if (termID === "invite_code") {
        if (msg.startsWith(prefix)) return msg.slice(prefix.length).trim();
        if (msg) return msg;
      }
      if (msg.startsWith(prefix)) return msg.slice(prefix.length).trim();
      const idx = msg.indexOf(prefix);
      if (idx >= 0) return msg.slice(idx + prefix.length).trim();
    }
    return "";
  };

  const inviteTaskMissingCode = (taskObj) => {
    if (!taskObj || String(taskObj.kind || "") !== "invite") return false;
    if (findInviteCode(taskObj)) return false;
    const status = String(taskObj.status || "").toLowerCase();
    const reason = String(taskObj.reason_code || "").toLowerCase();
    return status === "done" && (!reason || reason === "ok");
  };

  const renderPostDOM = (preservedTerminal = null) => {
    const qrEl = el("invite-qr");
    const taskObj = inviteState.taskID ? state.tasks.get(inviteState.taskID) : null;
    const code = findInviteCode(taskObj);
    if (qrEl && code && typeof window.qrcode === "function") {
      try {
        const qr = window.qrcode(0, "M");
        qr.addData(code);
        qr.make();
        qrEl.innerHTML = qr.createSvgTag({ scalable: true, cellSize: 4, margin: 4, alt: "invite code" });
      } catch {
        qrEl.textContent = "(QR failed)";
      }
    }
    restoreShellTerminalDOM(preservedTerminal);
    syncShellDOM();
  };

  const topologyFromPeers = (peers) => ({
    format: "miopunch.topology.fallback",
    observed_at: new Date().toISOString(),
    self: { peer_id: "", role: "unknown" },
    net: {},
    state_head: {},
    members: (peers || []).map((p) => ({ peer_id: p.peer_id, role: "unknown" })),
    presence: {},
    bootstrap: { recommendations: [], attempts: [], more_rounds: [] },
    neighbors: { target_k: 0, selected: [], active: [], unhealthy: [], degree_distribution: [] },
    attempts: [],
    payloads: [],
    recovery: { events: [] },
  });

  const startDesktopRuntime = async (options = {}) => {
    if (state.previewMode) {
      scheduleRender();
      return null;
    }

    let resp = null;
    try {
      runtimeStream = { ready: false, status: "starting", error: null };
      resp = await withTimeout(getBridge().DesktopRuntimeStart(), "Start runtime");
      renderConnection(resp && resp.connection ? resp.connection : null);
      if (!resp || !resp.ok) {
        markRuntimeStreamFailed(resp && resp.error);
        throw new Error(bridgeErrorSummary(resp && resp.error));
      }
      if (resp.state) applyDesktopSnapshot(resp.state);
      markRuntimeStreamReady();
      scheduleRender();
      return resp;
    } catch (err) {
      if (!resp) markRuntimeStreamFailed(err);
      scheduleRender();
      if (!options.silent) toast(`Connect failed: ${String(err)}`);
      return null;
    }
  };

  const resyncDesktopRuntime = async (options = {}) => {
    if (state.previewMode) {
      scheduleRender();
      return null;
    }
    if (runtimeResyncInFlight || runtimeRecoveryInFlight) return null;

    runtimeResyncInFlight = true;
    try {
      const bridge = getBridge();
      const canResync = !!(lastConn && lastConn.connected && runtimeStream.ready);
      const action = canResync
        ? bridge.DesktopRuntimeResync()
        : bridge.DesktopRuntimeStart();
      const label = canResync ? "Refresh runtime" : "Start runtime";
      const resp = await withTimeout(action, label);
      renderConnection(resp && resp.connection ? resp.connection : null);
      if (!resp || !resp.ok) {
        if (!canResync) markRuntimeStreamFailed(resp && resp.error);
        throw new Error(bridgeErrorSummary(resp && resp.error));
      }
      if (resp.state) applyDesktopSnapshot(resp.state);
      if (!canResync) markRuntimeStreamReady();
      scheduleRender();
      return resp;
    } catch (err) {
      scheduleRender();
      if (!options.silent) toast(`Refresh failed: ${String(err)}`);
      return null;
    } finally {
      runtimeResyncInFlight = false;
    }
  };

  const recoverDesktopRuntimeFromGap = async (options = {}) => {
    if (state.previewMode) {
      scheduleRender();
      return null;
    }
    if (runtimeRecoveryInFlight) return null;

    runtimeRecoveryInFlight = true;
    runtimeRecoveryQueue = [];
    try {
      const resp = await withTimeout(getBridge().DesktopRuntimeResync(), "Recover runtime");
      renderConnection(resp && resp.connection ? resp.connection : null);
      if (!resp || !resp.ok) throw new Error(bridgeErrorSummary(resp && resp.error));
      if (resp.state) applyDesktopSnapshot(resp.state);
      runtimeRecoveryInFlight = false;
      replayRuntimeRecoveryQueue();
      scheduleRender();
      return resp;
    } catch (err) {
      if (!options.silent) toast(`Refresh failed: ${String(err)}`);
      return null;
    } finally {
      if (runtimeRecoveryInFlight) {
        runtimeRecoveryInFlight = false;
        runtimeRecoveryQueue = [];
      }
    }
  };

  const createPreviewTask = (kind, args) => {
    const taskID = `preview-${kind}-${String(previewTaskSeq++).padStart(3, "0")}`;
    const taskObj = {
      task_id: taskID,
      kind,
      status: kind === "sh_attach" || kind === "approve" ? "running" : "done",
      stage: "preview",
      reason_code: "OK",
      exit_code: 0,
      report_ready: kind !== "sh_attach" && kind !== "approve",
      created_at: new Date().toISOString(),
      facts: [],
      suggestions: [],
    };

    if (kind === "invite") {
      taskObj.stage = "invite ready";
      taskObj.facts.push({ message: `invite_code=mp:v0.preview.${state.previewFixture}.${taskID}` });
    } else if (kind === "join") {
      taskObj.stage = "membership accepted";
      taskObj.facts.push({ message: `invite_code=${String((args && args.code) || "").slice(0, 40)}` });
    } else if (kind === "approve") {
      taskObj.stage = "waiting for joiner";
    } else if (kind === "approve_decision") {
      taskObj.stage = `${String(args && args.decision || "decision")} submitted`;
      const req = pendingApprovalRequests().find((item) =>
        String(item.approve_task_id || item.task_id || "").trim() === String(args && args.approve_task_id || "").trim() &&
        String(item.request_msg_id || "").trim() === String(args && args.request_msg_id || "").trim()
      );
      if (req) {
        req.status = String(args && args.decision || "") === "reject" ? "rejected" : "approved";
        req.decision = String(args && args.decision || "");
        req.updated_at = new Date().toISOString();
      }
    } else if (kind === "ping") {
      taskObj.stage = "payload exchanged";
      taskObj.facts.push({ message: "path=quic/udp4 rtt_ms=18" });
    } else if (kind === "sh_ls") {
      const target = String(args && args.target || "").trim();
      taskObj.stage = target ? "sessions listed" : "targets listed";
      if (target) {
        taskObj.facts.push({ term_id: "session", message: "session=main" });
        taskObj.facts.push({ term_id: "session", message: "session=maintenance" });
      } else {
        taskObj.facts.push({ term_id: "target", message: "target=local" });
        taskObj.facts.push({ term_id: "target", message: "target=ssh:ops" });
      }
    } else if (kind === "revoke_member") {
      taskObj.stage = "decl written";
      taskObj.facts.push({ message: `revoked_peer_id=${args && args.peer_id ? args.peer_id : "-"}` });
      const mem = memberByPeerID(args && args.peer_id);
      if (mem) mem.revoked = true;
    }

    attachPeerFact(taskObj, args && args.peer_id);
    upsertTask(taskObj);
    scheduleRender();
    return taskObj;
  };

  const createTask = async (kind, args) => {
    const taskArgs = normalizeTaskArgs(args);
    if (state.previewMode) return createPreviewTask(kind, taskArgs);
    const resp = await withTimeout(getBridge().CreateTask(kind, taskArgs), `Create ${kind}`);
    if (!resp || !resp.ok) throw new Error(bridgeErrorSummary(resp && resp.error));
    return resp.task;
  };

  const getTask = async (taskID, timeoutMs = 12000) => {
    if (state.previewMode) return state.tasks.get(taskID) || null;
    const resp = await withTimeout(getBridge().GetTask(taskID), "Get task", timeoutMs);
    if (!resp || !resp.ok) throw new Error(bridgeErrorSummary(resp && resp.error));
    return resp.task;
  };

  const exportReport = async (taskID, onSavedPath) => {
    if (state.previewMode) {
      const fakePath = `/tmp/${taskID || "preview-report"}.md`;
      if (typeof onSavedPath === "function") onSavedPath(fakePath);
      toast(`Preview report: ${fakePath}`);
      return { path: fakePath };
    }
    const resp = await getBridge().ExportTaskReport(taskID);
    if (!resp || !resp.ok) throw new Error(bridgeErrorSummary(resp && resp.error));
    if (resp.cancelled) return { cancelled: true };
    if (resp.path && typeof onSavedPath === "function") onSavedPath(String(resp.path));
    toast(resp.path ? `Saved: ${String(resp.path)}` : "Saved");
    return resp;
  };

  const runPeerTask = async (kind) => {
    const peerID = state.view.type === "peer" ? String(state.view.peerID || "") : "";
    if (!peerID) return;
    const args = kind === "sh_ls" ? { peer_id: peerID, target: "" } : { peer_id: peerID };
    try {
      const created = await createTask(kind, args);
      upsertTask(attachPeerFact(created, peerID));
      toast(`${kind} started`);
      scheduleRender();
    } catch (err) {
      toast(String(err));
    }
  };

  const revokePeer = async (peerID) => {
    const id = String(peerID || "").trim();
    if (!id) return;
    if (!state.previewMode && !window.confirm(`Revoke access for ${id}?`)) return;
    try {
      const created = await createTask("revoke_member", { peer_id: id, dangerous: true });
      upsertTask(attachPeerFact(created, id));
      toast("Revoke task started");
      scheduleRender();
    } catch (err) {
      toast(String(err));
    }
  };

  const measureCell = (fontFamily, fontSizePx) => {
    const span = document.createElement("span");
    span.textContent = "W";
    span.style.fontFamily = fontFamily;
    span.style.fontSize = `${fontSizePx}px`;
    span.style.position = "absolute";
    span.style.visibility = "hidden";
    span.style.top = "-9999px";
    document.body.appendChild(span);
    const rect = span.getBoundingClientRect();
    document.body.removeChild(span);
    return { w: Math.max(6, rect.width), h: Math.max(10, rect.height) };
  };

  const fitAndSendWinSize = () => {
    const container = el("terminal");
    const ws = shellState.ws;
    const term = shellState.term;
    if (!container || !ws || ws.readyState !== WebSocket.OPEN || !term) return;
    const fontFamily = term.options.fontFamily || "monospace";
    const fontSize = term.options.fontSize || 13;
    const cell = measureCell(fontFamily, fontSize);
    const rect = container.getBoundingClientRect();
    const cols = Math.max(20, Math.floor(rect.width / cell.w));
    const rows = Math.max(6, Math.floor(rect.height / cell.h));
    try {
      term.resize(cols, rows);
    } catch {
      // ignore resize failures from xterm internals
    }
    try {
      ws.send(JSON.stringify({ op: "winsize", winsize: { cols, rows } }));
    } catch {
      // ignore closed websocket race
    }
  };

  const disconnectShell = (phase = shellView.peerID ? "disconnected" : "idle", detail = "") => {
    closeShellTransport();
    shellView.phase = phase;
    shellView.detail = detail || shellPhaseDefaultDetail(phase);
    shellView.error = "";
    rememberShellSelection();
    scheduleRender();
  };

  const openTerminal = () => {
    const container = el("terminal");
    if (!container) throw new Error("terminal container is missing");
    container.textContent = "";
    if (typeof window.Terminal !== "function") {
      container.textContent = "xterm.js failed to load";
      throw new Error("xterm.js failed to load");
    }
    const term = new window.Terminal({
      cursorBlink: true,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
      fontSize: 13,
      theme: { background: "#0b1014", foreground: "#e8edf2", cursor: "#82d2ff" },
    });
    shellState.term = term;
    term.open(container);
    term.focus();
    return term;
  };

  const startPreviewShell = async (peerID, target, session) => {
    const created = await createTask("sh_attach", { peer_id: peerID, target, session });
    upsertTask(attachPeerFact(created, peerID));
    shellView.taskID = created.task_id;
    shellView.discoveryTaskID = "";
    shellState.taskID = created.task_id;
    const term = openTerminal();
    term.writeln("miopunch preview shell");
    term.writeln(`peer=${peerID}`);
    term.writeln(`session=${session}`);
    term.writeln("");
    term.write("$ ");
    shellView.phase = "connected";
    shellView.detail = `Preview connected to ${target}/${session}.`;
    shellView.error = "";
    rememberShellSelection();
    syncShellDOM();
  };

  const startLiveShell = async (peerID, target, session) => {
    const encoder = new TextEncoder();
    const decoder = new TextDecoder("utf-8");
    const created = await createTask("sh_attach", { peer_id: peerID, target, session });
    upsertTask(attachPeerFact(created, peerID));
    shellView.taskID = created.task_id;
    shellView.discoveryTaskID = "";
    shellState.taskID = created.task_id;
    shellState.expectedClose = false;
    shellState.wsError = "";
    shellState.remoteDataSeen = false;
    shellView.detail = `Task ${created.task_id} created. Waiting for terminal bridge...`;
    syncShellDOM();
    const container = el("terminal");
    if (!container) throw new Error("terminal container is missing");
    if (typeof window.Terminal !== "function") {
      container.textContent = "xterm.js failed to load";
      throw new Error("xterm.js failed to load");
    }

    const bridgeInfo = await withTimeout(getBridge().TerminalBridgeInfo(), "Load terminal bridge");
    if (!bridgeInfo || !bridgeInfo.ok) throw new Error(bridgeErrorSummary(bridgeInfo && bridgeInfo.error));
    const token = String(bridgeInfo.token || "");
    const baseURL = String(bridgeInfo.base_url || "");
    const subprotocol = String(bridgeInfo.subprotocol || "miopunch.sh.v0");
    if (!baseURL || !token) throw new Error("terminal bridge is not ready");
    shellView.detail = `Connecting to ${target}/${session}...`;
    syncShellDOM();
    const term = openTerminal();
    term.writeln("Connecting...");
    syncShellDOM();

    const wsURL = `${baseURL}/api/v0/tasks/${encodeURIComponent(created.task_id)}/ws?token=${encodeURIComponent(token)}`;
    const ws = new WebSocket(wsURL, [subprotocol]);
    ws.binaryType = "arraybuffer";
    shellState.ws = ws;
    ws.onopen = () => {
      if (shellState.ws !== ws) return;
      shellView.phase = "connecting";
      shellView.detail = `Terminal bridge connected. Waiting for shell output from ${target}/${session}...`;
      shellView.error = "";
      rememberShellSelection();
      syncShellDOM();
      fitAndSendWinSize();
    };
    ws.onmessage = (msg) => {
      if (shellState.ws !== ws || !shellState.term) return;
      let output = "";
      let byteLength = 0;
      if (typeof msg.data === "string") {
        output = msg.data;
        byteLength = output.length;
      } else {
        const bytes = new Uint8Array(msg.data);
        byteLength = bytes.byteLength;
        output = decoder.decode(bytes);
      }
      if (!shellState.remoteDataSeen && byteLength > 0) {
        shellState.remoteDataSeen = true;
        shellView.phase = "connected";
        shellView.detail = `Connected to ${target}/${session}.`;
        shellView.error = "";
        rememberShellSelection();
        syncShellDOM();
      }
      shellState.term.write(output);
    };
    ws.onerror = () => {
      if (shellState.ws !== ws) return;
      shellState.wsError = "terminal websocket error";
      shellView.detail = "Terminal bridge reported an error.";
      syncShellDOM();
    };
    ws.onclose = (event) => {
      const expectedClose = shellState.expectedClose;
      const wasConnected = shellView.phase === "connected";
      const reason = shellSocketCloseReason(event, shellState.wsError);
      const taskID = shellView.taskID;
      shellState.expectedClose = false;
      if (expectedClose) return;
      closeShellTransport();
      shellView.phase = wasConnected ? "disconnected" : "failed";
      shellView.detail = wasConnected
        ? shellPhaseDefaultDetail("disconnected")
        : shellPhaseDefaultDetail("failed");
      shellView.error = wasConnected ? `Disconnected: ${reason}` : `Connect failed: ${reason}`;
      rememberShellSelection();
      scheduleRender();
      if (!taskID) return;
      void (async () => {
        const latest = await waitForShellTaskOutput(taskID);
        if (shellView.taskID !== taskID || !shellTaskFailed(latest)) return;
        const summary = shellTaskDiagnosticSummary(latest);
        if (!summary) return;
        shellView.phase = wasConnected ? "disconnected" : "failed";
        shellView.detail = wasConnected
          ? shellPhaseDefaultDetail("disconnected")
          : shellPhaseDefaultDetail("failed");
        shellView.error = wasConnected ? `Disconnected: ${summary}` : `Connect failed: ${summary}`;
        rememberShellSelection();
        scheduleRender();
      })();
    };

    term.onData((data) => {
      const ws = shellState.ws;
      try {
        if (ws && ws.readyState === WebSocket.OPEN) ws.send(encoder.encode(data));
      } catch {
        // ignore closed websocket race
      }
    });

    const ro = new ResizeObserver(() => {
      window.clearTimeout(shellState.fitTimer);
      shellState.fitTimer = window.setTimeout(fitAndSendWinSize, 80);
    });
    ro.observe(container);
    shellState.resizeObs = ro;
  };

  const wireEvents = () => {
    document.querySelectorAll(".nav-tab").forEach((btn) => {
      btn.addEventListener("click", () => setActiveTab(btn.dataset.tab));
    });

    const refreshBtn = el("btn-refresh");
    if (refreshBtn) refreshBtn.addEventListener("click", (event) => {
      event.preventDefault();
      void resyncDesktopRuntime();
    });

    const select = el("preview-fixture");
    if (select) select.addEventListener("change", () => {
      loadPreviewFixture(select.value);
      localStorage.setItem("miopunch_desktop_fixture", select.value);
    });

    const host = el("page-host");
    if (host) {
      host.addEventListener("click", handlePageClick);
      host.addEventListener("submit", handlePageSubmit);
    }

    if (window.runtime && typeof window.runtime.EventsOn === "function") {
      window.runtime.EventsOn("desktop:startup_error", (payload) => {
        try {
          const component = payload && payload.component ? String(payload.component) : "startup";
          const err = payload && payload.error ? payload.error : null;
          toast(`${component}: ${bridgeErrorSummary(err)}`);
        } catch {
          // ignore malformed runtime payload
        }
      });
      window.runtime.EventsOn("desktop:state", (ev) => {
        try {
          handleDesktopStateEvent(ev);
        } catch {
          // ignore malformed event payload
        }
      });
      window.runtime.EventsOn("desktop:runtime", (ev) => {
        try {
          handleDesktopRuntimeEvent(ev);
          scheduleRender();
        } catch {
          // ignore malformed connection payload
        }
      });
    }
  };

  const handlePageClick = async (event) => {
    const target = event.target.closest("button, a");
    if (!target) return;

    if (target.dataset.openOverview !== undefined) {
      event.preventDefault();
      backToOverview();
      return;
    }
    if (target.matches("[data-back]")) {
      event.preventDefault();
      backToOverview();
      return;
    }
    if (target.dataset.openPeer) {
      event.preventDefault();
      navigate({ type: "peer", peerID: target.dataset.openPeer, section: "overview" });
      return;
    }
    if (target.dataset.peerSection) {
      event.preventDefault();
      const peerID = target.dataset.peerId || (state.view.type === "peer" ? state.view.peerID : "");
      navigate({ type: "peer", peerID, section: target.dataset.peerSection });
      return;
    }
    if (target.dataset.openFlow) {
      event.preventDefault();
      navigate({ type: "flow", flow: target.dataset.openFlow });
      return;
    }
    if (target.dataset.openMember) {
      event.preventDefault();
      navigate({ type: "member", memberID: target.dataset.openMember });
      return;
    }
    if (target.dataset.openSetting) {
      event.preventDefault();
      navigate({ type: "section", section: target.dataset.openSetting });
      return;
    }
    if (target.dataset.copyPeer) {
      event.preventDefault();
      await copyToClipboard(target.dataset.copyPeer);
      return;
    }
    if (target.dataset.runPeerTask) {
      event.preventDefault();
      await runPeerTask(target.dataset.runPeerTask);
      return;
    }
    if (target.dataset.copyInvite !== undefined) {
      event.preventDefault();
      const input = el("invite-code");
      if (input && input.value) await copyToClipboard(input.value);
      return;
    }
    if (target.dataset.approvalDecision) {
      event.preventDefault();
      await submitApprovalDecision(target.dataset.approveTaskId, target.dataset.requestMsgId, target.dataset.approvalDecision);
      return;
    }
    if (target.id === "btn-invite") {
      event.preventDefault();
      await createInvite();
      return;
    }
    if (target.id === "join-report-export") {
      event.preventDefault();
      if (!joinState.taskID) return;
      await exportReport(joinState.taskID, (p) => {
        joinState.lastExportPath = p;
        scheduleRender();
      });
      return;
    }
    if (target.dataset.revokeMember) {
      event.preventDefault();
      await revokePeer(target.dataset.revokeMember);
      return;
    }
    if (target.id === "btn-localapi-clear") {
      event.preventDefault();
      await clearLocalAPIOverride();
      return;
    }
    if (target.id === "btn-app-quit") {
      event.preventDefault();
      if (!state.previewMode) await getBridge().Quit();
      return;
    }
    if (target.id === "btn-shell-discover") {
      event.preventDefault();
      await discoverShell();
      return;
    }
    if (target.id === "btn-shell-disconnect") {
      event.preventDefault();
      disconnectShell();
      return;
    }
  };

  const handlePageSubmit = async (event) => {
    event.preventDefault();
    const form = event.target;
    try {
      if (form.id === "join-form") await submitJoin();
      else if (form.id === "approve-form") await submitApprove();
      else if (form.id === "localapi-form") await applyLocalAPIOverride();
      else if (form.id === "shell-form") await submitShell();
    } catch (err) {
      toast(String(err));
    }
  };

  const waitForInviteTaskOutput = async (taskID) => {
    const delays = [0, 150, 300, 500, 800, 1200, 1600];
    let taskObj = taskID ? state.tasks.get(taskID) : null;
    for (const delay of delays) {
      taskObj = taskID ? state.tasks.get(taskID) || taskObj : taskObj;
      if (findInviteCode(taskObj)) {
        if (inviteState.missingTaskID === taskID) inviteState.missingTaskID = "";
        return taskObj;
      }

      if (delay > 0) await sleep(delay);

      taskObj = taskID ? state.tasks.get(taskID) || taskObj : taskObj;
      if (findInviteCode(taskObj)) {
        if (inviteState.missingTaskID === taskID) inviteState.missingTaskID = "";
        return taskObj;
      }

      const latest = await getTask(taskID, 2500);
      if (latest) {
        upsertTask(latest);
        taskObj = state.tasks.get(taskID) || latest;
        scheduleRender();
        if (findInviteCode(taskObj)) {
          if (inviteState.missingTaskID === taskID) inviteState.missingTaskID = "";
          return taskObj;
        }
        if (inviteTaskMissingCode(latest)) {
          inviteState.missingTaskID = taskID;
          return taskObj;
        }
      }
    }
    return taskObj;
  };

  const waitForShellTaskOutput = async (taskID) => {
    const delays = [0, 120, 240, 400, 650, 900, 1200];
    let taskObj = taskID ? state.tasks.get(taskID) : null;
    for (const delay of delays) {
      taskObj = taskID ? state.tasks.get(taskID) || taskObj : taskObj;
      if (taskObj && (shellTaskFailed(taskObj) || String(taskObj.status || "").toLowerCase() === "done")) return taskObj;

      if (delay > 0) await sleep(delay);

      taskObj = taskID ? state.tasks.get(taskID) || taskObj : taskObj;
      if (taskObj && (shellTaskFailed(taskObj) || String(taskObj.status || "").toLowerCase() === "done")) return taskObj;

      const latest = await getTask(taskID, 2500);
      if (latest) {
        upsertTask(latest);
        taskObj = state.tasks.get(taskID) || latest;
        scheduleRender();
        if (shellTaskFailed(taskObj) || String(taskObj.status || "").toLowerCase() === "done") return taskObj;
      }
    }
    return taskObj;
  };

  const createInvite = async () => {
    inviteState.busy = true;
    inviteState.message = "Creating invite...";
    inviteState.missingTaskID = "";
    scheduleRender();
    try {
      const created = await createTask("invite", {});
      const taskID = upsertTask(created);
      inviteState.taskID = taskID;
      if (taskID) await waitForInviteTaskOutput(taskID);
      inviteState.message = "";
      scheduleRender();
    } catch (err) {
      inviteState.message = `Create failed: ${String(err)}`;
      toast(inviteState.message);
    } finally {
      inviteState.busy = false;
      scheduleRender();
    }
  };

  const submitJoin = async () => {
    const codeInput = el("join-code");
    const code = codeInput ? codeInput.value.trim() : "";
    if (!code) {
      toast("Missing invite code");
      return;
    }
    const created = await createTask("join", { code });
    joinState.taskID = upsertTask(created);
    joinState.lastExportPath = "";
    scheduleRender();
  };

  const submitApprove = async () => {
    const input = el("approve-code");
    const code = input ? input.value.trim() : "";
    if (!code) {
      toast("Missing invite code");
      return;
    }
    approveState.message = "Starting approval listener...";
    scheduleRender();
    const created = await createTask("approve", { code, explicit_review: true });
    approveState.taskID = upsertTask(created);
    approveState.message = "";
    scheduleRender();
  };

  const submitApprovalDecision = async (approveTaskID, requestMsgID, decision) => {
    const req = pendingApprovalRequests().find((item) =>
      String(item.approve_task_id || item.task_id || "").trim() === String(approveTaskID || "").trim() &&
      String(item.request_msg_id || "").trim() === String(requestMsgID || "").trim()
    );
    const key = approvalRequestKey(req || { approve_task_id: approveTaskID, request_msg_id: requestMsgID });
    if (!key) return;
    approvalDecisionState.set(key, { busy: true, decision, failure: "", message: "Submitting decision..." });
    scheduleRender();
    try {
      const created = await createTask("approve_decision", {
        approve_task_id: String(approveTaskID || "").trim(),
        request_msg_id: String(requestMsgID || "").trim(),
        decision,
      });
      const taskID = upsertTask(created);
      if (taskID) approvalDecisionTasks.set(taskID, key);
      syncApprovalDecisionTask(state.tasks.get(taskID));
      toast(decision === "approve" ? "Approval submitted" : "Rejection submitted");
    } catch (err) {
      approvalDecisionState.set(key, { busy: false, decision, failure: String(err), message: "" });
      toast(String(err));
    } finally {
      scheduleRender();
    }
  };

  const applyLocalAPIOverride = async () => {
    if (state.previewMode) return;
    const input = el("localapi-override");
    const value = input ? input.value.trim() : "";
    renderConnection(await getBridge().SetLocalAPIOverride(value));
    await startDesktopRuntime();
  };

  const clearLocalAPIOverride = async () => {
    if (state.previewMode) return;
    renderConnection(await getBridge().ClearLocalAPIOverride());
    await startDesktopRuntime();
  };

  const discoverShell = async () => {
    const peerID = state.view.type === "peer" ? String(state.view.peerID || "").trim() : "";
    const targetInput = el("shell-target");
    const sessionInput = el("shell-session");
    const typedTarget = targetInput ? targetInput.value.trim() : "";
    const typedSession = sessionInput ? sessionInput.value.trim() : "";
    if (!peerID) {
      failShellAction("Discover failed: missing peer_id");
      return;
    }

    syncShellSelectionForPeer(peerID);
    shellView.peerID = peerID;
    shellView.target = typedTarget || shellView.target || shellDefaultTarget(peerID);
    shellView.session = typedSession || shellView.session || shellDefaultSession(peerID);

    const discoverTargets = shellView.targetOptions.length === 0 || !typedTarget;
    const target = discoverTargets ? "" : typedTarget;
    const restingPhase = shellView.phase === "disconnected" ? "disconnected" : "idle";

    shellView.phase = "listing";
    shellView.detail = discoverTargets ? "Listing shell targets..." : `Listing sessions for ${target}...`;
    shellView.error = "";
    shellView.taskID = "";
    shellView.discoveryTaskID = "";
    rememberShellSelection();
    scheduleRender();

    try {
      const created = await createTask("sh_ls", { peer_id: peerID, target });
      const taskID = upsertTask(attachPeerFact(created, peerID));
      shellView.taskID = taskID;
      shellView.discoveryTaskID = taskID;
      scheduleRender();

      const latest = await waitForShellTaskOutput(taskID);
      const taskObj = state.tasks.get(taskID) || latest || created;
      if (shellTaskFailed(taskObj)) throw new Error(taskFailureSummary(taskObj));

      if (discoverTargets) {
        const targets = shellTaskValues(taskObj, "target", "target=");
        shellView.targetOptions = targets;
        shellView.sessionOptions = [];
        shellView.sessionTarget = "";
        if (!typedTarget) {
          shellView.target = targets.includes("local") ? "local" : (targets[0] || shellDefaultTarget(peerID));
        }
        shellView.detail = targets.length ? "Targets discovered." : "No targets discovered.";
      } else {
        const sessions = shellTaskValues(taskObj, "session", "session=");
        shellView.sessionOptions = sessions;
        shellView.sessionTarget = target;
        if (!typedSession) {
          shellView.session = sessions.includes("main") ? "main" : (sessions[0] || shellDefaultSession(peerID));
        }
        shellView.detail = sessions.length ? `Sessions discovered for ${target}.` : `No sessions discovered for ${target}.`;
      }

      shellView.phase = restingPhase;
      shellView.error = "";
      rememberShellSelection();
      scheduleRender();
    } catch (err) {
      shellView.phase = "failed";
      shellView.detail = discoverTargets ? "Target discovery failed. Retry is available." : "Session discovery failed. Retry is available.";
      shellView.error = `Discover failed: ${String(err)}`;
      rememberShellSelection();
      scheduleRender();
      toast(shellView.error);
    }
  };

  const submitShell = async () => {
    const peerID = state.view.type === "peer" ? String(state.view.peerID || "").trim() : "";
    const targetInput = el("shell-target");
    const sessionInput = el("shell-session");
    if (!peerID) {
      failShellAction("Connect failed: missing peer_id");
      return;
    }
    syncShellSelectionForPeer(peerID);
    const target = targetInput && targetInput.value.trim() ? targetInput.value.trim() : shellDefaultTarget(peerID);
    const session = sessionInput && sessionInput.value.trim() ? sessionInput.value.trim() : shellDefaultSession(peerID);

    closeShellTransport();
    shellView.peerID = peerID;
    shellView.target = target;
    shellView.session = session;
    shellView.phase = "connecting";
    shellView.detail = `Connecting to ${target}/${session}...`;
    shellView.error = "";
    shellView.discoveryTaskID = "";
    rememberShellSelection();
    scheduleRender();

    try {
      if (state.previewMode) await startPreviewShell(peerID, target, session);
      else await startLiveShell(peerID, target, session);
    } catch (err) {
      failShellAction(`Connect failed: ${String(err)}`);
    }
  };

  const initFromQuery = () => {
    const query = new URLSearchParams(window.location.search);
    const queryTab = query.get("tab") || "";
    const queryFixture = query.get("fixture") || "";
    state.previewMode = !bridgeAvailable();
    if (state.previewMode) {
      const fixture = queryFixture || localStorage.getItem("miopunch_desktop_fixture") || "owner";
      const select = el("preview-fixture");
      if (select) select.value = previewFixtures[fixture] ? fixture : "owner";
      loadPreviewFixture(select ? select.value : fixture);
    } else {
      const fixtureWrap = el("preview-fixture-wrap");
      if (fixtureWrap) fixtureWrap.classList.add("is-hidden");
      renderConnection(null);
      void startDesktopRuntime({ silent: true });
    }

    const tab = queryTab || localStorage.getItem("miopunch_desktop_tab") || "network";
    state.activeTab = tab === "admin" && roleKnown() && !adminVisible() ? "network" : tab;
    state.view = queryView(query);
    scheduleRender();
  };

  const queryView = (query) => {
    if (query.get("peer")) return { type: "peer", peerID: query.get("peer"), section: query.get("section") || "overview" };
    if (query.get("flow")) return { type: "flow", flow: query.get("flow") };
    if (query.get("member")) return { type: "member", memberID: query.get("member") };
    if (query.get("section")) return { type: "section", section: query.get("section") };
    return { type: "overview" };
  };

  wireEvents();
  initFromQuery();
})();
