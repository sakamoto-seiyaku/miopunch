(() => {
  const el = (id) => document.getElementById(id);

  const COPY = {
    tabs: {
      network: { title: "Network", eyebrow: "device network" },
      shell: { title: "Shell", eyebrow: "remote sessions" },
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
    runtimeSnapshot: null,
    diagnostics: [],
    approvalRequests: [],
    activeTab: "network",
    view: { type: "overview" },
    previewMode: false,
    previewFixture: "owner",
    networkMapPeerID: "",
    localAliases: {},
  };

  const inviteState = { taskID: "", busy: false, message: "", missingTaskID: "", approvalCode: "", approvalTaskID: "" };
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
    activeSessionTaskID: "",
    optionsOpen: false,
    leftCollapsed: false,
    rightOpen: true,
    zen: false,
  };
  const shellSelections = new Map();
  let shellTargetContextMenu = null;
  const shellState = {
    ws: null,
    term: null,
    resizeObs: null,
    scrollDispose: null,
    taskID: "",
    fitTimer: 0,
    expectedClose: false,
    wsError: "",
    remoteDataSeen: false,
    remoteExit: null,
    frameBuffer: new Uint8Array(0),
  };
  const settingsState = { saving: false, message: "", failure: null, exportPath: "" };

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
  const governanceConfig = () => (state.config && state.config.governance ? state.config.governance : {});
  const hasText = (value) => String(value || "").trim() !== "";

  const pickField = (obj, ...names) => {
    if (!obj || typeof obj !== "object") return "";
    for (const name of names) {
      if (typeof obj[name] !== "undefined" && obj[name] !== null) {
        const value = String(obj[name]).trim();
        if (value) return value;
      }
    }
    return "";
  };

  const evidenceFacts = (snapshot = state.runtimeSnapshot) => {
    const evidence = snapshot && snapshot.evidence ? snapshot.evidence : {};
    return Array.isArray(evidence.facts) ? evidence.facts : [];
  };

  const evidenceSuggestions = (snapshot = state.runtimeSnapshot) => {
    const evidence = snapshot && snapshot.evidence ? snapshot.evidence : {};
    return Array.isArray(evidence.suggestions) ? evidence.suggestions : [];
  };

  const factValue = (facts, key) => {
    const prefix = `${key}=`;
    for (const fact of Array.isArray(facts) ? facts : []) {
      const message = String(fact && fact.message || "").trim();
      const termID = String(fact && fact.term_id || "").trim();
      if (termID === key) return message.startsWith(prefix) ? message.slice(prefix.length).trim() : message;
      if (message.startsWith(prefix)) return message.slice(prefix.length).trim();
    }
    return "";
  };

  const runtimeNetworkID = (snapshot = state.runtimeSnapshot) => {
    const discover = snapshot && snapshot.discover_view ? snapshot.discover_view : {};
    return pickField(discover, "network_id", "NetworkID") || factValue(evidenceFacts(snapshot), "network_id");
  };

  const runtimeRole = (snapshot = state.runtimeSnapshot) => {
    const explicitRole = factValue(evidenceFacts(snapshot), "role").toLowerCase();
    if (explicitRole) return explicitRole;
    if (!runtimeNetworkID(snapshot)) return "";
    const gov = state.config && state.config.governance && typeof state.config.governance === "object"
      ? state.config.governance
      : {};
    const currentRole = String(
      gov.self_role
      || (state.topology && state.topology.self && state.topology.self.role)
      || "",
    ).toLowerCase();
    return ["owner", "admin", "member"].includes(currentRole) ? currentRole : "";
  };

  const normalizeRuntimeShellSession = (item) => {
    if (!item || typeof item !== "object") return item;
    const taskID = pickField(item, "task_id", "id", "ID");
    return {
      ...item,
      task_id: taskID,
      id: pickField(item, "id", "ID") || taskID,
      peer_id: pickField(item, "peer_id", "PeerID"),
      target: pickField(item, "target", "Target"),
      session: pickField(item, "session", "Session"),
      status: pickField(item, "status", "Status") || "pending",
    };
  };

  const topologyFromRuntimeSnapshot = (snapshot) => {
    if (!snapshot || typeof snapshot !== "object") return null;
    if (snapshot.topology) return snapshot.topology;

    const discover = snapshot.discover_view || {};
    const facts = [
      ...evidenceFacts(snapshot),
      ...((lastConn && lastConn.failure && Array.isArray(lastConn.failure.facts)) ? lastConn.failure.facts : []),
    ];
    const networkID = runtimeNetworkID(snapshot);
    const selfPeerID = pickField(discover, "self_peer_id", "SelfPeerID") || factValue(facts, "peer_id");
    const role = runtimeRole(snapshot) || (networkID ? "member" : "unknown");
    const rawPeers = Array.isArray(discover.peers) ? discover.peers : Array.isArray(discover.Peers) ? discover.Peers : [];
    if (!networkID && !selfPeerID && rawPeers.length === 0) return null;

    const members = [];
    if (selfPeerID) {
      members.push({
        peer_id: selfPeerID,
        role,
        member_name: "This device",
        platform: "",
      });
    }
    for (const peer of rawPeers) {
      const peerID = pickField(peer, "peer_id", "PeerID");
      if (!peerID || peerID === selfPeerID) continue;
      members.push({
        peer_id: peerID,
        role: "member",
        member_name: pickField(peer, "device_name", "DeviceName"),
        platform: pickField(peer, "platform", "Platform"),
        app_ver: pickField(peer, "app_ver", "AppVer"),
        online_state: pickField(peer, "online_state", "OnlineState"),
        last_activity_unix_ms: Number(peer.last_observed_unix_ms || peer.LastObservedUnixMs || 0),
      });
    }

    const peerSessions = Array.isArray(snapshot.peer_sessions) ? snapshot.peer_sessions : [];
    const active = peerSessions
      .map((session) => ({
        peer_id: pickField(session, "peer_id", "remote_peer_id", "PeerID", "RemotePeerID"),
        healthy: session.healthy !== false,
        path_family: pickField(session, "path_family", "PathFamily"),
        data_proto: pickField(session, "protocol", "Protocol"),
        selected_path: pickField(session, "selected_path", "SelectedPath"),
        local_endpoint: pickField(session, "local_endpoint", "LocalEndpoint"),
        remote_endpoint: pickField(session, "remote_endpoint", "RemoteEndpoint"),
        last_activity_unix_ms: Number(session.last_activity_unix_ms || session.LastActivityUnixMs || session.last_proven_unix_ms || session.LastProvenUnixMs || 0),
      }))
      .filter((session) => session.peer_id && session.healthy);
    const activeIDs = new Set(active.map((session) => session.peer_id));
    const selected = members
      .filter((mem) => mem.peer_id && mem.peer_id !== selfPeerID && !activeIDs.has(mem.peer_id))
      .map((mem) => ({
        peer_id: mem.peer_id,
        healthy: String(mem.online_state || "").toLowerCase() === "online",
        path_family: "presence",
        data_proto: "discover",
        last_activity_unix_ms: Number(mem.last_activity_unix_ms || 0),
      }));

    return {
      format: "miopunch.runtime.v1.projection",
      observed_at: new Date().toISOString(),
      self: { peer_id: selfPeerID, role },
      net: { net_id: networkID },
      state_head: {},
      members,
      neighbors: { target_k: selected.length + active.length, selected, active, unhealthy: [], failures: [], degree_distribution: [] },
      attempts: [],
      payloads: [],
      recovery: { events: [] },
    };
  };

  const configFromRuntimeSnapshot = (snapshot) => {
    const base = snapshot && snapshot.config && typeof snapshot.config === "object" ? snapshot.config : state.config || {};
    const networkID = runtimeNetworkID(snapshot);
    const role = runtimeRole(snapshot);
    const admin = ["owner", "admin"].includes(role);
    const governance = base.governance && typeof base.governance === "object" ? base.governance : {};
    const derived = networkID
      ? {
        state: admin ? "admin_network" : "member_network",
        self_role: role || "member",
        can_invite: admin,
        can_approve: admin,
        can_init_owner: false,
        can_create_new_network: true,
      }
      : {
        state: "no_network",
        self_role: "",
        can_invite: false,
        can_approve: false,
        can_init_owner: true,
        can_create_new_network: true,
      };
    return { ...base, governance: { ...governance, ...derived } };
  };
  const isFirstRunUninitialized = () => {
    const gov = governanceConfig();
    if (String(gov.state || "") === "no_network") return true;
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
  const effectiveSelfRole = () => String(governanceConfig().self_role || "").trim() || selfRole();
  const roleKnown = () => !!(state.topology && state.topology.self && effectiveSelfRole());
  const isAdminRole = (role) => ["owner", "admin"].includes(String(role || "").toLowerCase());
  const networkJoined = () => {
    if (!state.topology || isFirstRunUninitialized()) return false;
    const top = state.topology;
    const role = String(top.self && top.self.role || "").toLowerCase();
    const netID = top.net && top.net.net_id;
    const memberList = Array.isArray(top.members) ? top.members : [];
    return !!(hasText(netID) || memberList.length > 0 || (role && role !== "unknown"));
  };
  const canInvite = () => {
    const gov = governanceConfig();
    if (typeof gov.can_invite === "boolean") return gov.can_invite;
    return networkJoined() && isAdminRole(effectiveSelfRole());
  };
  const canApprove = () => {
    const gov = governanceConfig();
    if (typeof gov.can_approve === "boolean") return gov.can_approve;
    return networkJoined() && isAdminRole(effectiveSelfRole());
  };
  const canInitOwner = () => governanceConfig().can_init_owner === true;
  const canCreateNewNetwork = () => governanceConfig().can_create_new_network === true;
  const adminVisible = () => canInvite() || canApprove();
  const shellVisible = () => networkJoined();
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

  const peerSessionForPeer = (peerID) => {
    const id = String(peerID || "").trim();
    if (!id) return null;
    const sessions = Array.isArray(state.peerSessions) ? state.peerSessions : [];
    return sessions.find((item) => String(item && item.remote_peer_id || "").trim() === id) ||
      sessions.find((item) => String(item && item.peer_id || "").trim() === id) ||
      null;
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
    if (String(mem.peer_id || "") === String(self().peer_id || "")) return { label: "this device", cls: "chip-role" };
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
    return shellSessionsForPeer(id)[0] || null;
  };

  const shellSessionsForPeer = (peerID) => {
    const id = String(peerID || "").trim();
    if (!id) return [];
    const sessions = Array.isArray(state.shellSessions) ? state.shellSessions : [];
    const out = sessions
      .filter((item) => String(item && item.peer_id || "").trim() === id)
      .filter((item) => String(item && item.status || "").toLowerCase() !== "done");
    const seen = new Set(out.map((item) => String(item && item.task_id || "").trim()).filter(Boolean));
    for (const taskObj of state.tasks.values()) {
      const taskID = String(taskObj && taskObj.task_id || taskObj && taskObj.id || "").trim();
      if (!taskID || seen.has(taskID)) continue;
      const kind = String(taskObj && taskObj.kind || "").toLowerCase();
      if (kind !== "sh_attach" && kind !== "sh") continue;
      if (String(taskObj && taskObj.status || "").toLowerCase() === "done") continue;
      if (shellTaskValue(taskObj, "peer_id", "peer_id=") !== id) continue;
      out.push({
        task_id: taskID,
        peer_id: id,
        target: shellTaskValue(taskObj, "target", "target=") || "local",
        session: shellTaskValue(taskObj, "session", "session=") || "main",
        status: taskObj.status || "running",
        stage: taskObj.stage || "",
        created_at: taskObj.created_at || "",
        report_ready: !!taskObj.report_ready,
        attachable: true,
      });
      seen.add(taskID);
    }
    return out.sort((a, b) => String(b && b.created_at || "").localeCompare(String(a && a.created_at || "")));
  };

  const activeShellSessionForPeer = (peerID) => {
    const sessions = shellSessionsForPeer(peerID);
    const activeTaskID = String(shellView.activeSessionTaskID || shellView.taskID || "").trim();
    const target = String(shellView.target || "").trim();
    if (activeTaskID) {
      return sessions.find((item) => String(item && item.task_id || "").trim() === activeTaskID) || null;
    }
    if (target) {
      return sessions.find((item) => String(item && item.target || "local").trim() === target) || null;
    }
    return sessions[0] || null;
  };

  const shellSessionLabel = (item) => {
    if (!item) return "local / main";
    const target = String(item.target || "local").trim() || "local";
    const session = String(item.session || "main").trim() || "main";
    return `${target} / ${session}`;
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
      leftCollapsed: !!shellView.leftCollapsed,
      rightOpen: !!shellView.rightOpen,
      zen: !!shellView.zen,
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
      leftCollapsed: false,
      rightOpen: true,
      zen: false,
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
    const activeSession = activeShellSessionForPeer(id);
    shellView.peerID = id;
    shellView.target = activeSession && hasText(activeSession.target) ? String(activeSession.target).trim() : (hasText(seed.target) ? String(seed.target).trim() : shellDefaultTarget(id));
    shellView.session = activeSession && hasText(activeSession.session) ? String(activeSession.session).trim() : (hasText(seed.session) ? String(seed.session).trim() : shellDefaultSession(id));
    shellView.targetOptions = Array.isArray(seed.targetOptions) ? seed.targetOptions.slice() : [];
    shellView.sessionOptions = Array.isArray(seed.sessionOptions) ? seed.sessionOptions.slice() : [];
    shellView.sessionTarget = hasText(seed.sessionTarget) ? String(seed.sessionTarget).trim() : "";
    shellView.leftCollapsed = !!seed.leftCollapsed;
    shellView.rightOpen = !!seed.rightOpen;
    shellView.zen = !!seed.zen;
    shellView.error = "";
    shellView.detail = "";
    shellView.phase = "idle";
    shellView.taskID = activeSession ? String(activeSession.task_id || "") : "";
    shellView.activeSessionTaskID = shellView.taskID;
    shellView.discoveryTaskID = "";
    rememberShellSelection(id);
  };

  const shellPageActive = () => state.activeTab === "shell";

  const shellOperateEnabled = (peerID = shellView.peerID) => {
    const mem = memberByPeerID(peerID);
    const id = String(peerID || "").trim();
    const selfPeerID = String(self().peer_id || "");
    return !!(mem && id && id !== selfPeerID && !mem.revoked && (state.previewMode || lastConn && lastConn.connected));
  };

  const shellGateSatisfied = (peerID = shellView.peerID) => {
    const session = peerSessionForPeer(peerID);
    return !!(session && (session.ping_gate_satisfied === true || session.PingGateSatisfied === true || Number(session.shell_ready_unix_ms || session.ShellReadyUnixMs || 0) > 0));
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
    if (phase === "disconnected") return "Disconnected. Resume or start a new shell.";
    if (phase === "failed") return "Shell action failed. Retry is available.";
    return "Ready to open a shell.";
  };

  const shellStatusText = () => shellView.detail || shellPhaseDefaultDetail(shellView.phase);

  const shellCanDiscover = (peerID = shellView.peerID) => shellOperateEnabled(peerID)
    && !["listing", "connecting", "connected"].includes(shellView.phase);

  const shellCanConnect = (peerID = shellView.peerID) => shellOperateEnabled(peerID)
    && shellGateSatisfied(peerID)
    && !["listing", "connecting", "connected"].includes(shellView.phase);

  const shellNeedsPing = (peerID = shellView.peerID) =>
    !!(String(peerID || "").trim() && shellOperateEnabled(peerID) && !shellGateSatisfied(peerID));

  const shellSessionAttachable = (session) => {
    if (!session) return false;
    if (typeof session.attachable === "boolean") return session.attachable;
    const status = String(session.status || "").toLowerCase();
    return status === "pending" || status === "running" || status === "available";
  };
  const shellCanResume = (session, peerID = shellView.peerID) => shellCanConnect(peerID) && shellSessionAttachable(session);

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
    installTerminalScroll(container, shellState.term);
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

  const disposeShellTerminal = (term) => {
    if (!term) return;
    window.setTimeout(() => {
      try {
        term.dispose();
      } catch {
        // ignore dispose races from xterm internals
      }
    }, 0);
  };

  const closeShellTransport = (closeCode = 1000, reason = "bye", options = {}) => {
    const keepTerminal = !!(options && options.keepTerminal);
    if (shellState.resizeObs) {
      shellState.resizeObs.disconnect();
      shellState.resizeObs = null;
    }
    if (shellState.fitTimer) {
      window.clearTimeout(shellState.fitTimer);
      shellState.fitTimer = 0;
    }
    if (shellState.scrollDispose) {
      shellState.scrollDispose();
      shellState.scrollDispose = null;
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
    if (!keepTerminal && shellState.term) {
      const term = shellState.term;
      shellState.term = null;
      disposeShellTerminal(term);
    }
    shellState.taskID = "";
    shellState.wsError = "";
    shellState.remoteDataSeen = false;
    shellState.remoteExit = null;
    shellState.frameBuffer = new Uint8Array(0);
  };

  const syncShellDOM = () => {
    if (!shellPageActive()) return;
    const peerID = state.view.type === "shell-peer" ? state.view.peerID : shellView.peerID;
    const phase = el("shell-phase");
    if (phase) {
      phase.textContent = shellView.phase;
      phase.className = shellView.phase === "idle"
        ? "shell-phase-placeholder"
        : `chip ${shellPhaseClass(shellView.phase)}`.trim();
    }
    const status = el("shell-status");
    if (status) status.textContent = shellNeedsPing(peerID)
      ? "Run Ping first to prove the secure session before opening shell."
      : shellStatusText();
    const error = el("shell-error");
    if (error) {
      error.textContent = shellView.error || "";
      error.classList.toggle("is-hidden", !shellView.error);
    }
    const discover = el("btn-shell-discover");
    if (discover) discover.disabled = !shellCanDiscover();
    const findSessions = el("btn-shell-find-sessions");
    if (findSessions) findSessions.disabled = !shellCanDiscover();
    const connect = el("btn-shell-connect");
    if (connect) connect.disabled = !shellCanConnect(peerID);
    for (const disconnect of document.querySelectorAll("[data-shell-disconnect]")) {
      disconnect.disabled = !shellCanDisconnect();
    }
    const resume = el("btn-shell-resume");
    if (resume) {
      const taskID = String(resume.dataset.shellSessionTask || "").trim();
      const session = shellSessionsForPeer(peerID)
        .find((item) => String(item && item.task_id || "").trim() === taskID);
      resume.disabled = !(taskID && shellCanResume(session, peerID));
    }
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

  const installTerminalScroll = (container, term) => {
    if (shellState.scrollDispose) {
      shellState.scrollDispose();
      shellState.scrollDispose = null;
    }
    const onWheel = (event) => {
      if (event.defaultPrevented) return;
      if (!term || !term.buffer || !term.buffer.active || term.buffer.active.baseY <= 0) return;
      const unit = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 1 : 32;
      const lines = Math.max(1, Math.ceil(Math.abs(event.deltaY) / unit));
      term.scrollLines(event.deltaY > 0 ? lines : -lines);
      event.preventDefault();
    };
    container.addEventListener("wheel", onWheel, { passive: false });
    shellState.scrollDispose = () => container.removeEventListener("wheel", onWheel);
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
    state.runtimeSnapshot = snapshot;
    state.status = snapshot.status || null;
    state.topology = topologyFromRuntimeSnapshot(snapshot);
    state.peerSessions = Array.isArray(snapshot.peer_sessions) ? snapshot.peer_sessions : [];
    state.shellSessions = Array.isArray(snapshot.shell_sessions) ? snapshot.shell_sessions.map(normalizeRuntimeShellSession) : [];
    state.config = configFromRuntimeSnapshot(snapshot);
    state.diagnostics = Array.isArray(snapshot.diagnostics) ? snapshot.diagnostics : [];
    state.approvalRequests = Array.isArray(snapshot.approval_requests) ? snapshot.approval_requests : [];

    const tasks = Array.isArray(snapshot.tasks) ? snapshot.tasks : [];
    if (tasks.length) {
      const nextTasks = new Map();
      for (const taskObj of tasks) {
        const taskID = canonicalTaskID(taskObj && taskObj.task_id);
        if (!taskID) continue;
        const merged = mergeTask(state.tasks.get(taskID), taskObj);
        nextTasks.set(taskID, merged);
        syncApprovalDecisionTask(merged);
      }
      state.tasks = nextTasks;
    }
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
      state.shellSessions = Array.isArray(ev.shell_sessions) ? ev.shell_sessions.map(normalizeRuntimeShellSession) : [];
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
    if (kind === "snapshot" || kind === "snapshot.updated" || ev.snapshot) {
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
    state.shellSessions = Array.isArray(fx.shell_sessions) ? fx.shell_sessions.map(normalizeRuntimeShellSession) : [];
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
    for (const node of document.querySelectorAll("[data-shell-nav]")) {
      node.classList.toggle("is-hidden", !shellVisible());
    }
  };

  const renderTopbar = () => {
    const copy = COPY.tabs[state.activeTab] || COPY.tabs.network;
    const title = el("topbar-title");
    const eyebrow = el("topbar-eyebrow");
    if (title) title.textContent = copy.title;
    if (eyebrow) eyebrow.textContent = copy.eyebrow;
    const actions = document.querySelector(".topbar-actions");
    if (actions) {
      let chip = el("connection-chip");
      if (!chip) {
        chip = document.createElement("span");
        chip.id = "connection-chip";
        chip.className = "chip chip-muted";
        actions.prepend(chip);
      }
      let addr = el("connection-addr");
      if (!addr) {
        addr = document.createElement("span");
        addr.id = "connection-addr";
        addr.className = "helper";
        actions.insertBefore(addr, chip.nextSibling);
      }
      if (lastConn && lastConn.connected) {
        const selected = lastConn.selected || (lastConn.override_addr ? "override" : "user");
        chip.textContent = `Connected via ${selected}`;
        chip.className = "chip chip-done";
        addr.textContent = String(lastConn.override_addr || lastConn.addr || lastConn.user_addr || "");
      } else {
        chip.textContent = "Disconnected";
        chip.className = "chip chip-error";
        addr.textContent = lastConn && lastConn.failure ? String(lastConn.failure.message || "") : "";
      }
    }
    const fixtureWrap = el("preview-fixture-wrap");
    if (fixtureWrap) fixtureWrap.classList.toggle("is-hidden", !state.previewMode);
  };

  const setActiveTab = (name, opts = {}) => {
    let next = name || "network";
    if (next === "access") next = "network";
    if (next === "admin" && !adminVisible()) next = "network";
    if (next === "shell" && !shellVisible()) next = "network";
    state.activeTab = next;
    state.view = { type: "overview" };
    if (!opts.skipStore) localStorage.setItem("miopunch_desktop_tab", next);
    scheduleRender();
  };

  const navigate = (view) => {
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
    if (state.activeTab === "access") {
      state.activeTab = "network";
      state.view = { type: "overview" };
    }
    if (roleKnown() && !adminVisible() && state.activeTab === "admin") {
      state.activeTab = "network";
      state.view = { type: "overview" };
    }
    if (!shellVisible() && state.activeTab === "shell") {
      state.activeTab = "network";
      state.view = { type: "overview" };
    }
    renderNav();
    renderTopbar();
    if (state.activeTab === "network") renderNetwork();
    else if (state.activeTab === "shell") renderShell();
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
    <nav class="workspace-tabs" aria-label="Workspace view">
      ${items.map((item) => `
        <button class="workspace-tab ${item.active ? "is-active" : ""}" type="button" ${item.attr || ""}>
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
        label: deviceName(mem),
        meta: [mem.role || "peer", healthTextForPeer(mem)].filter(Boolean).join(" · "),
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
    { id: "runtime", title: "Connectivity", meta: "transport" },
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
    if (!networkJoined()) {
      renderNetworkJoin();
      return;
    }
    if (state.view.type === "peer") {
      if ((state.view.section || "overview") === "shell") {
        state.activeTab = "shell";
        syncShellSelectionForPeer(state.view.peerID);
        state.view = { type: "overview" };
        renderShell();
        return;
      }
      renderPeerDetail(memberByPeerID(state.view.peerID), state.view.section || "overview");
      return;
    }
    renderNetworkOverview();
  };

  const renderNetworkJoin = () => {
    setPage(`
      <section class="page">
        ${pageHeadingHTML("Network setup", "Join a network", "Paste an invite code to connect this device.")}
        ${runtimeStatusHTML()}
        ${renderJoinFlow()}
      </section>`);
  };

  const previewDeviceNames = {
    "peer-owner-zima-blue-0001": "This MacBook",
    "peer-studio-workstation-02": "Studio PC",
    "peer-livingroom-mini-03": "Living Room Mini",
    "peer-travel-laptop-04": "Travel Laptop",
    "peer-old-phone-05": "Old Phone",
    "peer-new-node-0000": "This Device",
  };

  const remoteDeviceName = (mem) => {
    const peerID = String(mem && mem.peer_id || "");
    if (state.previewMode) {
      return String(
        (mem && (mem.display_name || mem.device_name || mem.member_name || mem.name)) ||
        previewDeviceNames[peerID] ||
        shortID(peerID)
      );
    }
    return String((mem && mem.member_name) || peerID || "Unknown peer");
  };

  const localAliasForPeer = (peerID) => {
    const id = String(peerID || "");
    const prefs = settingsDesired().preferences || {};
    const aliases = prefs.peer_aliases && typeof prefs.peer_aliases === "object" ? prefs.peer_aliases : {};
    const alias = String(aliases[id] || "").trim();
    if (alias || !state.previewMode) return alias;
    return String(state.localAliases && state.localAliases[id] || "").trim();
  };

  const deviceName = (mem) => {
    const peerID = String(mem && mem.peer_id || "");
    return localAliasForPeer(peerID) || remoteDeviceName(mem);
  };

  const peerTitle = (peerID) => deviceName(memberByPeerID(peerID) || { peer_id: peerID });

  const edgeLabel = (peerID) => {
    const edge = activeNeighbor(peerID);
    if (!edge) return selectedNeighbor(peerID) ? "target" : "known";
    return [edge.data_proto, edge.path_family].filter(Boolean).join(" / ") || "connected";
  };

  const lastSeenText = (unixMs) => {
    const n = Number(unixMs || 0);
    if (!Number.isFinite(n) || n <= 0) return "-";
    const age = Math.max(0, Date.now() - n);
    if (age < 60000) return "just now";
    if (age < 3600000) return `${Math.floor(age / 60000)}m ago`;
    if (age < 86400000) return `${Math.floor(age / 3600000)}h ago`;
    return `${Math.floor(age / 86400000)}d ago`;
  };

  const healthTextForPeer = (mem) => {
    if (!mem) return "Unknown";
    if (mem.revoked) return "Access revoked";
    if (String(mem.peer_id || "") === String(self().peer_id || "")) return "This device";
    const edge = activeNeighbor(mem.peer_id);
    if (edge && edge.healthy !== false) return "Connected";
    if (selectedNeighbor(mem.peer_id)) return "Ready to connect";
    if (recentPeerFailure(mem.peer_id)) return "Needs attention";
    return "Known device";
  };

  const mapNodeClass = (mem, selectedID) => {
    const peerID = String(mem && mem.peer_id || "");
    const classes = ["network-node"];
    if (peerID === selectedID) classes.push("is-selected");
    if (peerID === String(self().peer_id || "")) classes.push("is-self");
    else if (mem && mem.revoked) classes.push("is-revoked");
    else if (activeNeighbor(peerID)) classes.push("is-active");
    else if (selectedNeighbor(peerID)) classes.push("is-target");
    else classes.push("is-known");
    return classes.join(" ");
  };

  const userTaskMessage = (taskObj, fallback = "Ready.") => {
    if (!taskObj) return fallback;
    const kind = String(taskObj.kind || "").toLowerCase();
    const status = String(taskObj.status || "").toLowerCase();
    const stage = String(taskObj.stage || "").trim();
    const reason = String(taskObj.reason_code || "").trim();
    if (reason && reason.toUpperCase() !== "OK") return taskFailureSummary(taskObj);
    if (kind === "join" && status === "done") return stage || "This device joined the network.";
    if (kind === "invite" && findInviteCode(taskObj)) return "Invite is ready. Copy it or scan the QR code on the new device.";
    if (kind === "invite" && inviteTaskMissingCode(taskObj)) return "Invite finished, but no invite code was returned.";
    if (kind === "approve" && status !== "done") return stage || "Waiting for a device to request access.";
    if (kind === "approve_decision") return stage || "Access decision was sent.";
    if (kind === "ping" && status === "done") return stage || "Ping succeeded.";
    if (kind === "sh_ls" && status === "done") return stage || "Remote shell choices are ready.";
    if (kind === "revoke_member" && status === "done") return stage || "Access change was written.";
    if (kind === "sh_attach" && status !== "done") return "Opening remote shell.";
    if (stage) return stage;
    if (status === "running" || status === "pending") return "Working...";
    if (status === "done") return "Done.";
    return fallback;
  };

  const operationStatusHTML = (taskObj, title, idleText) => {
    const cls = taskObj ? taskStatusClass(taskObj) : "chip-muted";
    const chip = taskObj && taskObj.status ? taskObj.status : "ready";
    return `
      <section class="operation-status">
        <div>
          <p class="eyebrow">${esc(title || "Status")}</p>
          <strong>${esc(userTaskMessage(taskObj, idleText || "Ready."))}</strong>
        </div>
        ${chipHTML(chip, cls)}
      </section>`;
  };

  const technicalLogHTML = (taskObj, label = "Diagnostics") => {
    if (!taskObj) return "";
    return `
      <details class="technical-log">
        <summary>${esc(label)}</summary>
        <div class="detail-table">
          ${detailRowHTML("Task", taskObj.task_id || "-")}
          ${detailRowHTML("Kind", taskObj.kind || "-")}
          ${detailRowHTML("Stage", taskObj.stage || "-")}
          ${detailRowHTML("Reason", taskObj.reason_code || "-")}
        </div>
        <div class="grid grid-2 mt">
          <div class="list">${renderFactList(taskObj.suggestions, COPY.empty.suggestions)}</div>
          <div class="list">${renderFactList(taskObj.facts, COPY.empty.facts)}</div>
        </div>
      </details>`;
  };

  const runtimeStatusHTML = () => {
    const snapshot = state.runtimeSnapshot || {};
    const stage = String(snapshot.stage || "Network");
    const summary = snapshot.summary && snapshot.summary.text ? String(snapshot.summary.text) : "Runtime state is loading.";
    const failure = lastConn && lastConn.failure ? lastConn.failure : null;
    const facts = [
      ...evidenceFacts(snapshot),
      ...((failure && Array.isArray(failure.facts)) ? failure.facts : []),
    ];
    const suggestions = [
      ...evidenceSuggestions(snapshot),
      ...((failure && Array.isArray(failure.suggestions)) ? failure.suggestions : []),
    ];
    const networkID = runtimeNetworkID(snapshot);
    const failureText = failure ? `${failure.reason_code || "ERROR"}: ${failure.message || bridgeErrorSummary(failure)}` : "";
    const inviteCode = latestInviteCode();
    return `
      <section class="surface-panel runtime-status-panel">
        <div class="card-header">
          <div>
            <p class="eyebrow">Runtime</p>
            <h3 class="card-title">${esc(summary)}</h3>
          </div>
          <span class="chip chip-role" id="stage-chip">${esc(`Stage ${stage}`)}</span>
        </div>
        ${networkID ? `<div class="helper">Network ${esc(networkID)}</div>` : ""}
        ${failureText ? `<div class="helper helper-error">${esc(failureText)}</div>` : ""}
        ${inviteCode ? `<label class="recent-invite">Recent invite<input class="textfield textfield-code" id="recent-invite-code" readonly value="${esc(inviteCode)}" /></label>` : ""}
        <details class="technical-log" open>
          <summary>Evidence</summary>
          <div class="grid grid-2 mt">
            <div class="list">${renderFactList(facts, COPY.empty.facts)}</div>
            <div class="list">${renderFactList(suggestions, COPY.empty.suggestions)}</div>
          </div>
        </details>
      </section>`;
  };

  const topologyMapHTML = (list, currentSelf) => {
    const view = { w: 720, h: 396 };
    const selfID = String(currentSelf.peer_id || "");
    const selectedID = state.networkMapPeerID && memberByPeerID(state.networkMapPeerID)
      ? state.networkMapPeerID
      : selfID;
    const selected = memberByPeerID(selectedID) || currentSelf;
    const peers = list.filter((mem) => String(mem.peer_id || "") !== selfID);
    const selfBase = peers.length <= 1 ? { x: 360, y: 282 } : { x: 360, y: 250 };
    const layout = [
      { peerID: selfID, x: selfBase.x, y: selfBase.y },
      { peerID: "peer-studio-workstation-02", x: 500, y: 112 },
      { peerID: "peer-livingroom-mini-03", x: 570, y: 285 },
      { peerID: "peer-travel-laptop-04", x: 175, y: 118 },
      { peerID: "peer-old-phone-05", x: 145, y: 325 },
    ];
    const fallbackPos = (idx, total) => {
      if (total <= 1) return { x: 360, y: 118 };
      const angle = (-Math.PI / 2) + (idx * 2 * Math.PI / Math.max(1, total));
      return { x: selfBase.x + Math.cos(angle) * 230, y: selfBase.y + Math.sin(angle) * 142 };
    };
    const positionFor = (mem, idx) => {
      const peerID = String(mem && mem.peer_id || "");
      return layout.find((item) => item.peerID === peerID) || fallbackPos(idx, peers.length);
    };
    const selfPos = positionFor({ peer_id: selfID }, 0);
    const peerPositions = new Map(peers.map((mem, idx) => [String(mem.peer_id || ""), positionFor(mem, idx)]));
    const edgeClass = (peerID) => {
      if (activeNeighbor(peerID)) return "is-active";
      if (selectedNeighbor(peerID)) return "is-target";
      return "is-muted";
    };
    const edges = peers.map((mem, idx) => {
      const pos = peerPositions.get(String(mem.peer_id || "")) || positionFor(mem, idx);
      const labelX = ((selfPos.x + pos.x) / 2).toFixed(1);
      const labelY = (((selfPos.y + pos.y) / 2) - 8).toFixed(1);
      const label = activeNeighbor(mem.peer_id) ? edgeLabel(mem.peer_id).toUpperCase() : "";
      return `
        <g class="network-edge ${edgeClass(mem.peer_id)}" data-edge-peer="${esc(mem.peer_id || "")}">
          <line x1="${selfPos.x}" y1="${selfPos.y}" x2="${pos.x.toFixed(1)}" y2="${pos.y.toFixed(1)}" />
          ${label ? `<text x="${labelX}" y="${labelY}">${esc(label)}</text>` : ""}
        </g>`;
    }).join("");
    const nodes = list.map((mem, idx) => {
      const peerID = String(mem.peer_id || "");
      const pos = peerID === selfID ? selfPos : (peerPositions.get(peerID) || positionFor(mem, idx));
      const label = deviceName(mem);
      const health = healthTextForPeer(mem);
      const x = ((pos.x / view.w) * 100).toFixed(3);
      const y = ((pos.y / view.h) * 100).toFixed(3);
      return `
        <button class="${mapNodeClass(mem, selectedID).replace("network-node", "map-device-node")}" type="button" data-map-peer="${esc(mem.peer_id || "")}" style="--x:${x}%;--y:${y}%;" aria-label="${esc(label)}">
          <span class="map-device-dot"></span>
          <span class="map-device-label">
            <strong>${esc(label)}</strong>
            <small>${esc(health)}</small>
          </span>
        </button>`;
    }).join("");
    const edge = activeNeighbor(selectedID);
    const selectedFailure = recentPeerFailure(selectedID);
    const selectedIsSelf = selectedID === selfID;
    const selectedCanShell = !selectedIsSelf && selected && !selected.revoked && (state.previewMode || lastConn && lastConn.connected);
    const selectedLastSeen = edge ? lastSeenText(edge.last_activity_unix_ms) : "-";
    const selectedStatus = statusForMember(selected);
    const quickActions = selectedIsSelf
      ? `
        <button class="btn btn-tonal" type="button" disabled>Rename</button>
        <button class="btn btn-tonal" type="button" disabled>Ping</button>`
      : `
        <button class="btn btn-primary" type="button" data-peer-section="shell" data-peer-id="${esc(selectedID)}" ${selectedCanShell ? "" : "disabled"}>Open shell</button>
        <button class="btn btn-tonal" type="button" data-open-peer="${esc(selectedID)}">Details</button>`;
    const deviceRows = list.map((mem) => {
      const peerID = String(mem.peer_id || "");
      const status = statusForMember(mem);
      return `
        <button class="device-row ${peerID === selectedID ? "is-selected" : ""}" type="button" data-map-peer="${esc(peerID)}">
          <span>
            <strong>${esc(deviceName(mem))}</strong>
            <small>${esc(edgeLabel(peerID))}</small>
          </span>
          ${chipHTML(status.label, status.cls)}
        </button>`;
    }).join("");
    return `
      <section class="network-console" aria-label="Network map prototype">
        <div class="network-map-panel">
          <div class="network-map-head">
            <div>
              <p class="eyebrow">Network map</p>
              <h3 class="card-title">Devices and direct links</h3>
            </div>
            <div class="network-map-legend">
              <span class="legend-dot active">Direct</span>
              <span class="legend-dot target">Ready</span>
              <span class="legend-dot muted">Known</span>
              <span class="legend-dot revoked">Revoked</span>
            </div>
          </div>
          <div class="network-map-board">
            <svg class="network-map-svg" viewBox="0 0 ${view.w} ${view.h}" role="img" aria-label="Device topology links">
              <rect class="network-map-plane" x="14" y="14" width="${view.w - 28}" height="${view.h - 28}" rx="18" />
              ${edges}
            </svg>
            <div class="network-map-nodes">${nodes}</div>
          </div>
          <div class="device-strip">${deviceRows}</div>
        </div>
        <aside class="network-device-panel">
          <div>
            <p class="eyebrow">${selectedIsSelf ? "This device" : "Selected device"}</p>
            <h3 class="card-title">${esc(deviceName(selected))}</h3>
          </div>
          <div class="device-status-line">
            ${chipHTML(selectedStatus.label, selectedStatus.cls)}
            <span>${esc(healthTextForPeer(selected))}</span>
          </div>
          <div class="detail-table">
            ${detailRowHTML("Role", selected.role || "-")}
            ${detailRowHTML("Connection", selectedIsSelf ? "local" : edge ? "direct" : selectedNeighbor(selectedID) ? "ready" : "not connected")}
            ${detailRowHTML("Path", selectedIsSelf ? "this device" : edgeLabel(selectedID))}
            ${detailRowHTML("Last activity", selectedLastSeen)}
            ${selectedFailure ? detailRowHTML("Recent issue", failureSummary(selectedFailure)) : ""}
          </div>
          <div class="action-row">${quickActions}</div>
          <div class="helper">Peer ID: ${esc(shortID(selectedID))}</div>
        </aside>
      </section>`;
  };

  const renderNetworkOverview = () => {
    const currentSelf = self();
    const list = members();
    if (!state.networkMapPeerID || !memberByPeerID(state.networkMapPeerID)) {
      state.networkMapPeerID = currentSelf.peer_id || "";
    }

    setPage(`
      <section class="page">
        ${networkSwitchHTML()}
        ${runtimeStatusHTML()}
        ${topologyMapHTML(list, currentSelf)}
      </section>`);
  };

  const recentPeerTask = (peerID, kinds = []) => {
    const kindSet = new Set(kinds.map((kind) => String(kind || "").toLowerCase()));
    return [...state.tasks.values()].filter((taskObj) => {
      if (kindSet.size && !kindSet.has(String(taskObj.kind || "").toLowerCase())) return false;
      return taskMentionsPeer(taskObj, peerID);
    }).sort((a, b) => String(b.created_at || "").localeCompare(String(a.created_at || "")))[0] || null;
  };

  const deviceConnectionKind = (mem) => {
    const peerID = String(mem && mem.peer_id || "");
    if (!mem) return "unknown";
    if (mem.revoked) return "revoked";
    if (peerID === String(self().peer_id || "")) return "local";
    if (activeNeighbor(peerID)) return "direct";
    if (selectedNeighbor(peerID)) return "ready";
    if (recentPeerFailure(peerID)) return "issue";
    return "known";
  };

  const deviceConnectionCopy = (mem) => {
    const kind = deviceConnectionKind(mem);
    if (kind === "local") return { title: "This device", detail: "Local machine in this network.", chip: "this device", cls: "chip-role" };
    if (kind === "direct") return { title: "Direct connection ready", detail: `Connected through ${edgeLabel(mem.peer_id)}.`, chip: "direct", cls: "chip-active" };
    if (kind === "ready") return { title: "Ready to try", detail: "This device is selected for connectivity but is not currently active.", chip: "ready", cls: "chip-running" };
    if (kind === "revoked") return { title: "Access revoked", detail: "This device cannot be used until access is restored by an administrator.", chip: "revoked", cls: "chip-revoked" };
    if (kind === "issue") return { title: "Needs attention", detail: "Recent connection attempts reported a problem.", chip: "issue", cls: "chip-error" };
    return { title: "Known device", detail: "This device is in the network, but there is no active direct path right now.", chip: "known", cls: "chip-muted" };
  };

  const peerMetadataHTML = (peerID) => `
    <div class="peer-metadata">
      <span>Peer ID ${esc(shortID(peerID))}</span>
      <button class="icon-btn compact-icon-btn" data-copy-peer="${esc(peerID)}" title="Copy Peer ID" aria-label="Copy Peer ID">
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><path d="M8 8h10v12H8z" /><path d="M6 16H4V4h12v2" /></svg>
      </button>
    </div>`;

  const previewPeerPaths = {
    "peer-owner-zima-blue-0001": {
      directIPv4: "100.92.0.10",
      directIPv6: "fd7a:115c:a1e0::10",
      localEndpoint: "192.168.31.42:49320",
      remoteEndpoint: "192.168.31.42:49320",
      publicTuple: "203.0.113.21:49320 -> 203.0.113.21:49320",
      punch: "local loopback",
      port: "49320/udp",
    },
    "peer-studio-workstation-02": {
      directIPv4: "100.92.0.21",
      directIPv6: "fd7a:115c:a1e0::21",
      localEndpoint: "192.168.31.42:49320",
      remoteEndpoint: "192.168.31.88:41877",
      publicTuple: "203.0.113.21:49320 -> 198.51.100.44:41877",
      punch: "udp6 direct",
      port: "41877/udp",
    },
    "peer-livingroom-mini-03": {
      directIPv4: "100.92.0.34",
      directIPv6: "not advertised",
      localEndpoint: "192.168.31.42:49320",
      remoteEndpoint: "10.0.0.12:55391",
      publicTuple: "203.0.113.21:49320 -> 198.51.100.91:55391",
      punch: "portmap assisted",
      port: "55391/udp",
    },
    "peer-travel-laptop-04": {
      directIPv4: "pending",
      directIPv6: "not advertised",
      localEndpoint: "192.168.31.42:49320",
      remoteEndpoint: "unknown",
      publicTuple: "waiting for peer reflexive address",
      punch: "last attempt timed out",
      port: "unknown",
    },
  };

  const firstText = (...values) => {
    for (const value of values) {
      const text = String(value || "").trim();
      if (text) return text;
    }
    return "";
  };

  const peerPathFacts = (peerID, selected, neighbor, selectedEdge) => {
    const preview = state.previewMode ? (previewPeerPaths[peerID] || {}) : {};
    const session = peerSessionForPeer(peerID) || {};
    return {
      directIPv4: firstText(selected && selected.direct_ipv4, neighbor && neighbor.direct_ipv4, session.direct_ipv4, selected && selected.ipv4, preview.directIPv4, state.previewMode && selected && selected.v4_hint, "unknown"),
      directIPv6: firstText(selected && selected.direct_ipv6, neighbor && neighbor.direct_ipv6, session.direct_ipv6, selected && selected.ipv6, preview.directIPv6, state.previewMode && selected && selected.v6_hint, "unknown"),
      localEndpoint: firstText(neighbor && neighbor.local_endpoint, neighbor && neighbor.local_addr, selectedEdge && selectedEdge.local_endpoint, session.local_endpoint, preview.localEndpoint, "unknown"),
      remoteEndpoint: firstText(neighbor && neighbor.remote_endpoint, neighbor && neighbor.remote_addr, selectedEdge && selectedEdge.remote_endpoint, session.remote_endpoint, preview.remoteEndpoint, "unknown"),
      publicTuple: firstText(neighbor && neighbor.public_tuple, neighbor && neighbor.tuple, selectedEdge && selectedEdge.public_tuple, session.public_tuple, preview.publicTuple, "unknown"),
      punch: firstText(neighbor && neighbor.punch_status, neighbor && neighbor.result, selectedEdge && selectedEdge.punch_status, session.punch_status, preview.punch, "unknown"),
      port: firstText(neighbor && neighbor.port, selectedEdge && selectedEdge.port, session.port, preview.port, "unknown"),
    };
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
      body = `<section class="surface-panel">${listItemHTML("Device was not found", "empty")}</section>`;
    } else if (section === "shell") {
      syncShellSelectionForPeer(peerID);
      const shellTaskObj = state.tasks.get(shellView.taskID || shellView.discoveryTaskID) || null;
      const activeSession = activeShellSessionForPeer(peerID);
      if (activeSession && !shellView.activeSessionTaskID) {
        shellView.activeSessionTaskID = String(activeSession.task_id || "");
        shellView.taskID = shellView.activeSessionTaskID;
      }
      const sessions = shellSessionsForPeer(peerID);
      const targetChoices = shellView.targetOptions.length
        ? shellView.targetOptions.join(", ")
        : "local";
      const sessionChoices = shellView.sessionOptions.length
        ? shellView.sessionOptions.join(", ")
        : "main";
      const currentTarget = activeSession && activeSession.target ? String(activeSession.target) : (shellView.target || shellDefaultTarget(peerID));
      const currentSession = activeSession && activeSession.session ? String(activeSession.session) : (shellView.session || shellDefaultSession(peerID));
      const sessionCards = sessions.length ? sessions.map((item) => {
        const taskID = String(item.task_id || "");
        const isActive = taskID && taskID === String(shellView.activeSessionTaskID || shellView.taskID || "");
        const attachable = shellSessionAttachable(item);
        return `
          <button class="shell-session-card ${isActive ? "is-active" : ""}" type="button" data-shell-session-task="${esc(taskID)}">
            <span>
              <strong>${esc(shellSessionLabel(item))}</strong>
              <small>${esc(item.stage || item.status || "running")}</small>
            </span>
            ${chipHTML(isActive ? "foreground" : attachable ? "available" : "unavailable", isActive ? "chip-role" : "chip-muted")}
          </button>`;
      }).join("") : `<div class="shell-session-empty">No active shell sessions for this device.</div>`;
      body = `
        <section class="shell-focus-workspace">
          <div class="shell-focus-header surface-panel">
            <div>
              <p class="eyebrow">Shell</p>
              <h3 class="device-hero-title">${esc(deviceName(selected))}</h3>
              ${peerMetadataHTML(peerID)}
            </div>
            <div class="shell-status-cluster">
              <span class="chip ${shellPhaseClass(shellView.phase)}" id="shell-phase">${esc(shellView.phase)}</span>
              <button class="btn btn-tonal" id="btn-shell-disconnect" type="button" ${shellCanDisconnect() ? "" : "disabled"}>Disconnect</button>
            </div>
          </div>
          <div class="shell-focus-grid">
            <aside class="shell-session-panel surface-panel">
              <div>
                <p class="eyebrow">Sessions</p>
                <h3 class="card-title">Resume or start fresh</h3>
              </div>
              <div class="shell-session-list">${sessionCards}</div>
              <form class="form-grid shell-options-form" id="shell-form">
                <input class="textfield mono is-hidden" id="shell-peer-id" value="${esc(peerID)}" autocomplete="off" readonly />
                <div class="shell-primary-actions">
                  <button class="btn btn-primary" id="btn-shell-resume" type="button" data-shell-session-task="${esc(activeSession && activeSession.task_id ? activeSession.task_id : "")}" ${shellCanResume(activeSession, peerID) ? "" : "disabled"}>Resume session</button>
                  <button class="btn btn-tonal" id="btn-shell-connect" type="submit" ${shellCanConnect(peerID) ? "" : "disabled"}>${activeSession ? "New shell" : "Open shell"}</button>
                </div>
                <div class="helper" id="shell-status">${esc(shellStatusText())}</div>
                <div class="helper helper-error ${shellView.error ? "" : "is-hidden"}" id="shell-error">${esc(shellView.error)}</div>
                <details class="advanced-panel shell-options-panel" ${shellView.optionsOpen ? "open" : ""}>
                  <summary>Shell options</summary>
                  <div class="shell-target-grid">
                    <label>Target<input class="textfield" id="shell-target" value="${esc(currentTarget)}" list="shell-target-options" autocomplete="off" /></label>
                    <label>Session<input class="textfield" id="shell-session" value="${esc(currentSession)}" list="shell-session-options" autocomplete="off" /></label>
                  </div>
                  <datalist id="shell-target-options">
                    ${shellView.targetOptions.map((value) => `<option value="${esc(value)}"></option>`).join("")}
                  </datalist>
                  <datalist id="shell-session-options">
                    ${shellView.sessionOptions.map((value) => `<option value="${esc(value)}"></option>`).join("")}
                  </datalist>
                  <div class="choice-row">
                    <span id="shell-target-choices">Targets: ${esc(targetChoices)}</span>
                    <span id="shell-session-choices">Sessions: ${esc(sessionChoices)}</span>
                  </div>
                  <button class="btn btn-tonal" id="btn-shell-discover" type="button" ${shellCanDiscover(peerID) ? "" : "disabled"}>Find available shells</button>
                </details>
              </form>
            </aside>
            <div class="terminal-shell shell-terminal-panel">
              <div class="terminal" id="terminal"></div>
            </div>
          </div>
          ${technicalLogHTML(shellTaskObj, "Diagnostics")}
        </section>`;
    } else {
      const pathText = neighbor ? `${neighbor.data_proto || "-"} / ${neighbor.path_family || "-"}` : edgeLabel(peerID);
      const recentTask = recentPeerTask(peerID);
      const shellSessions = shellSessionsForPeer(peerID);
      const connection = deviceConnectionCopy(selected);
      const remoteName = remoteDeviceName(selected);
      const localAlias = localAliasForPeer(peerID);
      const visibleName = deviceName(selected);
      const remoteNameMeta = remoteName && remoteName !== visibleName && remoteName !== peerID
        ? `<span>Remote name: ${esc(remoteName)}</span>`
        : "";
      const bucket = neighbor && neighbor.bucket || selectedEdge && selectedEdge.bucket || "-";
      const lastActivity = neighbor ? lastSeenText(neighbor.last_activity_unix_ms) : "-";
      const pathFacts = peerPathFacts(peerID, selected, neighbor, selectedEdge);
      body = `
        <section class="device-workspace redesigned-device-workspace">
          <section class="network-identity-panel">
            <div class="device-identity-card surface-panel">
              <p class="eyebrow">${isRemote ? "Remote device" : "This device"}</p>
              <h3 class="device-hero-title">${esc(visibleName)}</h3>
              <div class="identity-meta-row">
                ${remoteNameMeta}
                <span>Role: ${esc(role)}</span>
              </div>
              ${peerMetadataHTML(peerID)}
            </div>
            <div class="device-action-card surface-panel">
              <form class="device-name-editor" id="alias-form">
                <input type="hidden" id="alias-peer-id" value="${esc(peerID)}" />
                <label>Local alias<input class="textfield" id="alias-name" value="${esc(localAlias)}" placeholder="${esc(remoteName)}" autocomplete="off" /></label>
                <button class="btn btn-tonal" type="submit">Save alias</button>
              </form>
              <div class="device-action-stack">
                <div class="device-chip-row">
                  ${chipHTML(status.label, status.cls)}
                  ${chipHTML(connection.chip, connection.cls)}
                </div>
                <div class="device-command-row">
                  <button class="btn btn-primary" data-peer-section="shell" data-peer-id="${esc(peerID)}" ${canOperate ? "" : "disabled"}>Open shell</button>
                  <button class="btn btn-tonal" data-run-peer-task="ping" ${canOperate ? "" : "disabled"}>Ping</button>
                </div>
                <div class="identity-meta-row device-action-meta">
                  <span>${esc(pathText)}</span>
                  <span>${esc(lastActivity)}</span>
                </div>
              </div>
            </div>
          </section>
          <section class="node-insight-layout">
            <div class="surface-panel node-path-panel">
              <div class="card-header">
                <div>
                  <p class="eyebrow">Path</p>
                  <h3 class="card-title">Reachability facts</h3>
                </div>
              </div>
              <div class="network-fact-grid">
                <div><span>Path</span><strong>${esc(pathText)}</strong></div>
                <div><span>Bucket</span><strong>${esc(bucket)}</strong></div>
                <div><span>Last activity</span><strong>${esc(lastActivity)}</strong></div>
                <div><span>Shell sessions</span><strong>${esc(shellSessions.length ? `${shellSessions.length} live` : "none")}</strong></div>
              </div>
              <div class="path-detail-grid" aria-label="Addresses and path">
                <div><span>Direct IPv4</span><strong>${esc(pathFacts.directIPv4)}</strong></div>
                <div><span>Direct IPv6</span><strong>${esc(pathFacts.directIPv6)}</strong></div>
                <div><span>Local endpoint</span><strong>${esc(pathFacts.localEndpoint)}</strong></div>
                <div><span>Remote endpoint</span><strong>${esc(pathFacts.remoteEndpoint)}</strong></div>
                <div><span>Public tuple</span><strong>${esc(pathFacts.publicTuple)}</strong></div>
                <div><span>Port</span><strong>${esc(pathFacts.port)}</strong></div>
                <div><span>Punch result</span><strong>${esc(pathFacts.punch)}</strong></div>
                <div><span>Peer ID</span><strong>${esc(shortID(peerID))}</strong></div>
              </div>
            </div>
            <div class="surface-panel node-facts-panel">
              <div class="card-header">
                <div>
                  <p class="eyebrow">Node facts</p>
                  <h3 class="card-title">Reported metadata</h3>
                </div>
              </div>
              <div class="connection-stat-grid">
                <div><span>Role</span><strong>${esc(role)}</strong></div>
                <div><span>IPv4 hint</span><strong>${esc(selected.v4_hint || "-")}</strong></div>
                <div><span>IPv6 hint</span><strong>${esc(selected.v6_hint || "-")}</strong></div>
                <div><span>Peer ID</span><strong>${esc(shortID(peerID))}</strong></div>
                <div><span>Selected</span><strong>${esc(selectedEdge ? "yes" : "no")}</strong></div>
                <div><span>Active</span><strong>${esc(neighbor ? "yes" : "no")}</strong></div>
              </div>
              ${recentFailure ? `<div class="connection-issue">${esc(failureSummary(recentFailure))}</div>` : ""}
            </div>
            ${recentTask ? `<div class="surface-panel node-status-panel">
              ${operationStatusHTML(recentTask, "Progress", "Ready.")}
              ${technicalLogHTML(recentTask, "Diagnostics")}
            </div>` : ""}
          </section>
        </section>`;
    }

    setPage(`
      <section class="page">
        ${networkSwitchHTML(peerID)}
        ${body}
      </section>`);
  };

  const renderShell = () => {
    if (!networkJoined()) {
      setPage(`
        <section class="page">
          ${pageHeadingHTML("Shell", "Join a network first", "Remote shell sessions become available after this device joins a network.")}
          ${runtimeStatusHTML()}
          ${renderJoinFlow()}
        </section>`);
      return;
    }
    const selfPeerID = String(self().peer_id || "");
    const candidates = members().filter((mem) => String(mem.peer_id || "") && String(mem.peer_id || "") !== selfPeerID && !mem.revoked);
    const shellPeerMode = state.view.type === "shell-peer";
    const selected = shellPeerMode ? (memberByPeerID(state.view.peerID) || memberByPeerID(shellView.peerID) || candidates[0] || null) : null;
    const peerID = selected && selected.peer_id ? String(selected.peer_id) : "";
    if (peerID) syncShellSelectionForPeer(peerID);
    if (shellTargetContextMenu && shellTargetContextMenu.peerID !== peerID) shellTargetContextMenu = null;
    const shellSwitch = moduleSwitchHTML([
      { label: "Overview", meta: "sessions", active: !shellPeerMode, attr: "data-open-overview" },
      ...candidates.map((mem) => {
        const id = String(mem.peer_id || "");
        const count = shellSessionsForPeer(id).length;
        return {
          label: deviceName(mem),
          meta: count ? `${count} live` : edgeLabel(id),
          active: shellPeerMode && id === peerID,
          attr: `data-shell-peer="${esc(id)}"`,
        };
      }),
    ]);

    if (!shellPeerMode) {
      const liveSessions = candidates.flatMap((mem) => shellSessionsForPeer(mem.peer_id)
        .map((session) => ({ mem, session })));
      const liveItems = liveSessions.length ? liveSessions.map(({ mem, session }) => {
        const target = String(session.target || "local").trim() || "local";
        return `
          <button class="shell-overview-card" type="button" data-shell-peer="${esc(mem.peer_id)}" data-shell-session-task="${esc(session.task_id || "")}">
            <span>
              <strong>${esc(deviceName(mem))}</strong>
              <small>${esc(shellSessionLabel(session))}</small>
            </span>
            ${chipHTML(shellSessionAttachable(session) ? session.stage || session.status || "running" : "unavailable", shellSessionAttachable(session) ? "chip-running" : "chip-muted")}
          </button>`;
      }).join("") : `<div class="shell-session-empty">No live shell sessions are available.</div>`;
      const recentTargets = candidates.slice(0, 4).map((mem) => {
        const id = String(mem.peer_id || "");
        const session = shellSessionsForPeer(id)[0] || null;
        const target = session && session.target ? String(session.target) : shellDefaultTarget(id);
        return `
          <button class="shell-overview-card" type="button" data-shell-peer="${esc(id)}" data-shell-target="${esc(target)}">
            <span>
              <strong>${esc(deviceName(mem))}</strong>
              <small>${esc(target)}</small>
            </span>
            ${chipHTML(edgeLabel(id), activeNeighbor(id) ? "chip-active" : "chip-muted")}
          </button>`;
      }).join("");
      setPage(`
        <section class="page shell-page">
          ${shellSwitch}
          ${runtimeStatusHTML()}
          <section class="shell-overview-grid">
            <section class="surface-panel shell-overview-panel">
              <div class="shell-section-title">
                <span>Live sessions</span>
                <small>live sessions</small>
              </div>
              <div class="shell-overview-list">${liveItems}</div>
            </section>
            <section class="surface-panel shell-overview-panel">
              <div class="shell-section-title">
                <span>Targets</span>
                <small>recent and ready</small>
              </div>
              <div class="shell-overview-list">${recentTargets || `<div class="shell-session-empty">No shell targets are available.</div>`}</div>
            </section>
          </section>
        </section>`);
      return;
    }

    const shellTaskObj = state.tasks.get(shellView.taskID || shellView.discoveryTaskID) || null;
    const sessions = peerID ? shellSessionsForPeer(peerID) : [];
    const activeSession = peerID ? activeShellSessionForPeer(peerID) : null;
    if (activeSession && !shellView.activeSessionTaskID) {
      shellView.activeSessionTaskID = String(activeSession.task_id || "");
      shellView.taskID = shellView.activeSessionTaskID;
    }
    const currentTarget = shellView.target || (activeSession && activeSession.target ? String(activeSession.target) : shellDefaultTarget(peerID));
    const currentSession = shellView.session || (activeSession && activeSession.session ? String(activeSession.session) : shellDefaultSession(peerID));
    const targetOptions = [...new Set([
      currentTarget,
      "local",
      ...shellView.targetOptions,
      ...sessions.map((item) => String(item.target || "local").trim() || "local"),
    ].filter(Boolean))];
    const sessionOptionsForTarget = shellView.sessionTarget === currentTarget ? shellView.sessionOptions : [];
    const sessionOptions = [...new Set([
      currentSession,
      "main",
      ...sessionOptionsForTarget,
      ...sessions
        .filter((item) => String(item.target || "local").trim() === currentTarget)
        .map((item) => String(item.session || "main").trim() || "main"),
    ].filter(Boolean))];
    const matchingSession = sessions.find((item) =>
      String(item.target || "local").trim() === currentTarget &&
      String(item.session || "main").trim() === currentSession
    ) || null;
    const canResumeMatch = shellCanResume(matchingSession, peerID);
    const showDisconnect = ["connecting", "connected"].includes(shellView.phase) && shellCanDisconnect();
    const showPhase = shellView.phase && shellView.phase !== "idle";
    const liveSessionStrip = sessions.length ? `
      <section class="shell-live-panel" aria-label="Live shell sessions">
        <div class="shell-section-title shell-live-title">
          <span>Live sessions</span>
          <small>${esc(`${sessions.length} ${sessions.length === 1 ? "session" : "sessions"}`)}</small>
        </div>
        <div class="shell-live-strip">
          ${sessions.map((item) => {
          const taskID = String(item.task_id || "");
          const isActive = taskID && taskID === String(shellView.activeSessionTaskID || shellView.taskID || "");
          const attachable = shellSessionAttachable(item);
          return `
            <button class="shell-live-chip ${isActive ? "is-active" : ""}" type="button" data-shell-session-task="${esc(taskID)}">
              <span>${esc(shellSessionLabel(item))}</span>
              ${chipHTML(isActive ? "open" : attachable ? "resume" : "unavailable", isActive ? "chip-role" : attachable ? "chip-running" : "chip-muted")}
            </button>`;
          }).join("")}
        </div>
      </section>` : "";
    const needsPing = shellNeedsPing(peerID);
    const primaryAction = needsPing
      ? `<button class="btn btn-primary shell-connect-primary" id="btn-ping" type="button" data-run-peer-task="ping">Ping first</button>
         <button class="btn btn-tonal shell-connect-primary" type="submit" disabled>Open</button>`
      : showDisconnect
      ? `<button class="btn btn-tonal shell-connect-primary" type="button" data-shell-disconnect>Disconnect</button>`
      : canResumeMatch
        ? `<button class="btn btn-primary shell-connect-primary" type="button" data-shell-session-task="${esc(matchingSession.task_id || "")}">Resume</button>`
        : `<button class="btn btn-primary shell-connect-primary" id="btn-shell-connect" type="submit" ${shellCanConnect(peerID) ? "" : "disabled"}>Open</button>`;

    setPage(`
      <section class="page shell-page">
        ${shellSwitch}
        ${runtimeStatusHTML()}
        <section class="shell-focus-layout ${shellView.zen ? "is-zen" : ""}">
          <form class="surface-panel shell-connect-panel" id="shell-form" data-shell-session-form="true">
            <input class="textfield mono is-hidden" id="shell-peer-id" value="${esc(peerID)}" autocomplete="off" readonly />
            <div class="shell-connect-identity">
              <strong>${esc(selected ? deviceName(selected) : "No peer selected")}</strong>
              <span>${esc(currentTarget)} / ${esc(currentSession)}</span>
            </div>
            <label>Target<input class="textfield" id="shell-target" value="${esc(currentTarget)}" list="shell-target-options" autocomplete="off" /></label>
            <button class="btn btn-tonal btn-compact" id="btn-shell-discover" type="button" ${shellCanDiscover(peerID) ? "" : "disabled"}>Find targets</button>
            <label>Session<input class="textfield" id="shell-session" value="${esc(currentSession)}" list="shell-session-options" autocomplete="off" /></label>
            <button class="btn btn-tonal btn-compact" id="btn-shell-find-sessions" type="button" ${shellCanDiscover(peerID) ? "" : "disabled"}>Find sessions</button>
            ${primaryAction}
            <button class="btn btn-tonal btn-compact" type="button" data-shell-toggle="zen">${shellView.zen ? "Exit Zen" : "Zen"}</button>
            ${showPhase ? `<span class="chip ${shellPhaseClass(shellView.phase)}" id="shell-phase">${esc(shellView.phase)}</span>` : `<span class="shell-phase-placeholder" id="shell-phase">idle</span>`}
            <datalist id="shell-target-options">
              ${targetOptions.map((value) => `<option value="${esc(value)}"></option>`).join("")}
            </datalist>
            <datalist id="shell-session-options">
              ${sessionOptions.map((value) => `<option value="${esc(value)}"></option>`).join("")}
            </datalist>
            <div class="helper shell-connect-status" id="shell-status">${esc(needsPing ? "Run Ping first to prove the secure session before opening shell." : shellStatusText())}</div>
            <div class="helper helper-error ${shellView.error ? "" : "is-hidden"}" id="shell-error">${esc(shellView.error)}</div>
          </form>
          ${liveSessionStrip}
          <section class="shell-terminal-workspace">
            <div class="terminal-shell shell-terminal-panel">
              <div class="terminal" id="terminal"></div>
            </div>
            ${technicalLogHTML(shellTaskObj, "Diagnostics")}
          </section>
        </section>
      </section>`);
  };

  const renderAccess = () => {
    if (state.view.type === "flow") {
      renderAccessFlow(state.view.flow || "join");
      return;
    }
    const cards = [
      { id: "join", title: "Join network", meta: "Paste an invite and connect this computer to a network.", admin: false, tone: "primary" },
      { id: "invite", title: "Create invite", meta: "Generate a short code for another computer or phone.", admin: true, tone: "success" },
      { id: "approve", title: "Approve request", meta: "Approve devices that are waiting to enter this network.", admin: true, tone: "warning" },
    ].filter((flow) => !flow.admin || adminVisible()).map((flow) => `
      <button class="action-card ${flow.tone}" data-open-flow="${flow.id}" type="button">
        <span class="action-card-mark"></span>
        <span>
          <strong>${esc(flow.title)}</strong>
          <small>${esc(flow.meta)}</small>
        </span>
      </button>`).join("");
    setPage(`
      <section class="page">
        ${accessSwitchHTML()}
        ${pageHeadingHTML("Access overview", "Access")}
        <div class="action-card-grid grid">${cards}</div>
        ${renderApprovalRequestsPanel()}
        ${renderTaskCard("Recent activity")}
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
    const disabled = !state.previewMode && !(lastConn && lastConn.connected);
    return `
      <section class="flow-layout">
        <div class="surface-panel flow-primary">
          <div class="card-header">
            <div>
              <p class="eyebrow">Join</p>
              <h3 class="card-title">Connect this device</h3>
            </div>
          </div>
          <form class="form-grid" id="join-form">
            <label>Invite code or URL<input class="textfield textfield-code" id="join-code" placeholder="mp:v0..." autocomplete="off" ${disabled ? "disabled" : ""} /></label>
            <div class="action-row">
              <button class="btn btn-primary" type="submit" ${disabled ? "disabled" : ""}>Join</button>
              <button class="btn btn-tonal" id="join-report-export" type="button" ${!disabled && taskObj && taskObj.report_ready ? "" : "disabled"}>Export report</button>
            </div>
            <div class="helper" id="join-report-path">${esc(joinState.lastExportPath || "")}</div>
          </form>
        </div>
        <aside class="surface-panel">
          ${operationStatusHTML(taskObj, "Progress", "Paste an invite code to join.")}
          ${technicalLogHTML(taskObj)}
        </aside>
      </section>`;
  };

  const renderInviteFlow = () => {
    const taskObj = inviteState.taskID ? state.tasks.get(inviteState.taskID) : null;
    const code = findInviteCode(taskObj);
    const missingCode = taskObj && inviteState.missingTaskID === taskObj.task_id;
    const hint = inviteState.message || (code ? "Invite ready" : missingCode ? "no invite code was returned." : "Create an invite when the new device is nearby.");
    return `
      <section class="flow-layout invite-layout">
        <div class="surface-panel flow-primary">
          <div class="card-header">
            <div>
              <p class="eyebrow">Invite</p>
              <h3 class="card-title">Add another device</h3>
            </div>
            <button class="btn btn-primary" id="btn-invite" type="button" ${inviteState.busy || (!state.previewMode && !(lastConn && lastConn.connected)) ? "disabled" : ""}>Create</button>
          </div>
          <div class="form-grid">
            <label>Invite code<input class="textfield textfield-code invite-code-field" id="invite-code" readonly value="${esc(code)}" placeholder="Create an invite first" /></label>
            <div class="action-row">
              <button class="btn btn-tonal" data-copy-invite type="button" ${code ? "" : "disabled"}>Copy</button>
              <div class="helper" id="invite-hint">${esc(hint)}</div>
            </div>
          </div>
        </div>
        <aside class="surface-panel invite-side">
          <div class="qr-wrap" id="invite-qr"></div>
          ${operationStatusHTML(taskObj, "Progress", "No invite has been created yet.")}
          ${technicalLogHTML(taskObj)}
        </aside>
      </section>`;
  };

  const renderApproveFlow = () => {
    const taskObj = approveState.taskID ? state.tasks.get(approveState.taskID) : null;
    const disabled = !state.previewMode && !(lastConn && lastConn.connected);
    return `
      ${renderApprovalRequestsPanel()}
      <section class="surface-panel approval-listener">
        <div class="card-header">
          <div>
            <p class="eyebrow">Approval</p>
            <h3 class="card-title">Approval listener</h3>
          </div>
        </div>
        <p class="page-subtitle">Visible requests can be approved above. Manual code approval is only for recovery or older join flows.</p>
        ${operationStatusHTML(taskObj, "Listener", "Ready to receive incoming requests.")}
        ${technicalLogHTML(taskObj)}
        <details class="advanced-panel">
          <summary>Advanced manual approval</summary>
          <form class="form-grid compact-form" id="approve-form">
            <label>Invite code<input class="textfield textfield-code" id="approve-code" placeholder="Optional manual approval code" autocomplete="off" ${disabled ? "disabled" : ""} /></label>
            <button class="btn btn-tonal" type="submit" ${disabled ? "disabled" : ""}>Start approval</button>
            <div class="helper" id="approve-hint">${esc(approveState.message || "Use this only when a request is not visible in the review list.")}</div>
          </form>
        </details>
      </section>
      `;
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
      const peerLabel = displayName || shortID(req.member_peer_id);
      const hints = [
        req.platform ? req.platform : "",
        req.v4_hint ? `IPv4 ${req.v4_hint}` : "",
        req.v6_hint ? `IPv6 ${req.v6_hint}` : "",
      ].filter(Boolean).join(" · ");
      const canDecide = adminVisible() && status === "pending";
      return `
        <div class="approval-request-card approval-row">
          <div>
            <div class="row-title">${esc(peerLabel)}</div>
            <div class="row-meta">${esc(req.member_peer_id || "-")}</div>
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
      <section class="surface-panel">
        <div class="card-header">
          <div><p class="eyebrow">Review</p><h3 class="card-title">Devices waiting to join</h3></div>
          ${chipHTML(requests.length)}
        </div>
        <div class="row-list">${rows}</div>
      </section>`;
  };

  const renderAdmin = () => {
    if (!adminVisible()) {
      setPage(`<section class="page">${pageHeadingHTML("Admin", "Unavailable")}${runtimeStatusHTML()}<section class="card">${listItemHTML("Administrator controls are available only on owner or admin nodes", "empty")}</section></section>`);
      return;
    }
    if (state.view.type === "flow") {
      renderAdminFlow(state.view.flow || "invite");
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
        <button class="member-card" data-open-member="${esc(mem.peer_id || "")}">
          <div>
            <div class="row-title">${esc(deviceName(mem))}</div>
            <div class="row-meta">${esc(mem.peer_id || "-")} | ${esc(mem.v4_hint || mem.v6_hint || "path unknown")}</div>
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
        ${runtimeStatusHTML()}
        <section class="admin-summary">
          ${metricHTML("Owners", owners)}
          ${metricHTML("Admins", admins)}
          ${metricHTML("Revoked", revoked)}
          ${metricHTML("Governance", headShort(stateHead.governance_head_b64))}
        </section>
        <section class="action-card-grid grid">
          <button class="action-card success" data-open-flow="invite" type="button">
            <span class="action-card-mark"></span>
            <span><strong>Create invite</strong><small>Generate a join code for another device.</small></span>
          </button>
          <button class="action-card warning" data-open-flow="approve" type="button">
            <span class="action-card-mark"></span>
            <span><strong>Approve request</strong><small>Review devices waiting to enter this network.</small></span>
          </button>
        </section>
        ${renderApprovalRequestsPanel()}
        <section class="surface-panel">
          <div class="card-header">
            <div>
              <p class="eyebrow">Members</p>
              <h3 class="card-title">Who can access this network</h3>
            </div>
            ${chipHTML(list.length)}
          </div>
          <div class="row-list">${memberRows}</div>
        </section>
      </section>`);
  };

  const renderAdminFlow = (flow) => {
    const title = flow === "approve" ? "Approve request" : "Create invite";
    const body = flow === "approve" ? renderApproveFlow() : renderInviteFlow();
    setPage(`
      <section class="page">
        ${adminSwitchHTML()}
        ${pageHeadingHTML("Admin", title)}
        ${runtimeStatusHTML()}
        ${body}
      </section>`);
  };

  const renderMemberDetail = (mem) => {
    const selfPeerID = String(self().peer_id || "");
    const canRevoke = !!(mem && !mem.revoked && mem.peer_id !== selfPeerID && mem.role !== "owner" && mem.role !== "admin");
    const status = statusForMember(mem);
    setPage(`
      <section class="page">
        ${adminSwitchHTML(mem && mem.peer_id)}
        ${pageHeadingHTML("Member detail", mem ? deviceName(mem) : "Member")}
        <section class="workspace-grid">
          <div class="surface-panel">
            <div class="card-header">
              <div>
                <p class="eyebrow">Member</p>
                <h3 class="card-title">${esc(mem ? deviceName(mem) : "-")}</h3>
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
          <div class="danger-panel">
            <div class="card-header">
              <div>
                <p class="eyebrow">Danger zone</p>
                <h3 class="card-title">Remove access</h3>
              </div>
            </div>
            <button class="btn btn-tonal" data-revoke-member="${esc(mem && mem.peer_id ? mem.peer_id : "")}" ${canRevoke ? "" : "disabled"}>Revoke</button>
          </div>
        </section>
        ${renderTaskCard("Recent activity")}
      </section>`);
  };

  const settingsDesired = () => {
    const cfg = state.config || {};
    const desired = cfg.desired || {};
    const local = cfg.local || {};
    const runtime = desired.runtime || {
      mqtt_brokers: local.mqtt_brokers || [],
      p2p_network: local.p2p_network || "auto",
      p2p_ip_family: local.p2p_ip_family || "auto",
      data_proto: local.data_proto || "quic",
      quic_cc: local.quic_cc || "bbr",
      stun: local.stun || [],
      stun_explicit: !!local.stun_explicit,
      disable_portmap: !!local.disable_portmap,
      disable_assisted_addrs: !!local.disable_assisted_addrs,
    };
    const preferences = desired.preferences || { log_level: "info" };
    return { runtime, preferences };
  };

  const settingsEffective = () => {
    const cfg = state.config || {};
    const effective = cfg.effective || settingsDesired();
    return {
      runtime: effective.runtime || settingsDesired().runtime,
      preferences: effective.preferences || settingsDesired().preferences,
    };
  };

  const csvValue = (value) => Array.isArray(value) ? value.join(", ") : "";

  const selectHTML = (id, value, options, disabled = false) => `
    <select class="textfield" id="${esc(id)}" ${disabled ? "disabled" : ""}>
      ${options.map((opt) => `<option value="${esc(opt)}" ${String(value || "") === opt ? "selected" : ""}>${esc(opt)}</option>`).join("")}
    </select>`;

  const renderSettings = () => {
    if (state.view.type === "section") {
      renderSettingsSection(state.view.section || "localapi");
      return;
    }
    const sections = settingSections().map((item) => `
      <button class="action-card" data-open-setting="${item.id}" type="button">
        <span class="action-card-mark"></span>
        <span>
          <strong>${esc(item.title)}</strong>
          <small>${esc(item.meta)}</small>
        </span>
      </button>`).join("");
    const gov = governanceConfig();
    const governancePanel = (canInitOwner() || canCreateNewNetwork()) ? `
        <section class="surface-panel">
          <div class="card-header">
            <div>
              <p class="eyebrow">Governance</p>
              <h3 class="card-title">${canInitOwner() ? "Owner/Admin mode" : "Create new network"}</h3>
            </div>
            ${chipHTML(gov.state || "setup")}
          </div>
          <p class="page-subtitle">${esc(gov.reason || (canInitOwner() ? "Initialize this blank node before creating invites." : "Create a distinct local network for this node."))}</p>
          <div class="action-row">
            ${canInitOwner() ? `<button class="btn btn-primary" id="btn-init-network-bootstrap" type="button">Enable Owner/Admin mode</button>` : ""}
            ${canCreateNewNetwork() ? `<button class="btn btn-tonal" id="btn-init-network-new" type="button">Create new network</button>` : ""}
          </div>
        </section>` : "";
    setPage(`
      <section class="page">
        ${settingsSwitchHTML()}
        ${pageHeadingHTML("Settings overview", "Settings")}
        <section class="admin-summary">
          ${metricHTML("Version", state.status && state.status.version ? state.status.version : "-")}
          ${metricHTML("Uptime", state.status && state.status.uptime_ms ? fmtUptime(state.status.uptime_ms) : "-")}
          ${metricHTML("Mode", state.previewMode ? "preview" : "desktop")}
          ${metricHTML("Activity", state.tasks.size)}
        </section>
        ${governancePanel}
        <div class="action-card-grid grid">${sections}</div>
        <section class="surface-panel">
          <div class="action-row">
            <button class="btn btn-tonal" id="btn-app-quit" type="button" ${state.previewMode ? "disabled" : ""}>Quit</button>
          </div>
        </section>
      </section>`);
  };

  const renderSettingsSection = (section) => {
    let body = "";
    if (section === "runtime") {
      const desired = settingsDesired();
      const effective = settingsEffective();
      const runtime = desired.runtime;
      const prefs = desired.preferences;
      const apply = state.config && state.config.apply ? state.config.apply : {};
      const disabled = state.previewMode || settingsState.saving;
      const pendingSupportDisabled = true;
      const failure = settingsState.failure;
      const failureSuggestions = failure && Array.isArray(failure.suggestions) ? failure.suggestions : [];
      body = `
        <section class="settings-layout runtime-layout">
          <form class="surface-panel settings-form" id="runtime-config-form">
            <div class="card-header">
              <div><p class="eyebrow">Connectivity</p><h3 class="card-title">How this device connects</h3></div>
              <button class="btn btn-primary" type="submit" ${disabled ? "disabled" : ""}>Save</button>
            </div>
            <div class="settings-group settings-group-wide">
              <div>
                <p class="setting-group-title">Discovery and relay</p>
                <p class="helper">Use defaults when possible. Add brokers or STUN servers only when your network needs them.</p>
              </div>
              <label>MQTT brokers<input class="textfield" id="settings-mqtt-brokers" value="${esc(csvValue(runtime.mqtt_brokers))}" placeholder="host:port, host:port" autocomplete="off" ${disabled || pendingSupportDisabled ? "disabled" : ""} /></label>
              <label>STUN<input class="textfield" id="settings-stun" value="${esc(csvValue(runtime.stun))}" placeholder="stun.example.net:3478" autocomplete="off" ${disabled || pendingSupportDisabled ? "disabled" : ""} /></label>
            </div>
            <div class="settings-form-grid">
              <div class="settings-group">
                <p class="setting-group-title">Path preference</p>
                <label>P2P network${selectHTML("settings-p2p-network", runtime.p2p_network || "auto", ["auto", "udp_only", "tcp_only"], disabled || pendingSupportDisabled)}</label>
                <label>IP family${selectHTML("settings-p2p-ip-family", runtime.p2p_ip_family || "auto", ["auto", "v4", "v6"], disabled || pendingSupportDisabled)}</label>
              </div>
              <div class="settings-group">
                <p class="setting-group-title">Transport</p>
                <label>Data protocol${selectHTML("settings-data-proto", runtime.data_proto || "quic", ["quic", "kcp"], disabled || pendingSupportDisabled)}</label>
                <label>QUIC CC${selectHTML("settings-quic-cc", runtime.quic_cc || "bbr", ["bbr", "brutal"], disabled || pendingSupportDisabled)}</label>
              </div>
              <div class="settings-group">
                <p class="setting-group-title">Remote shell defaults</p>
                <label>Default shell target<input class="textfield" id="settings-shell-target" value="${esc(prefs.default_shell_target || "")}" autocomplete="off" ${disabled || pendingSupportDisabled ? "disabled" : ""} /></label>
                <label>Default shell session<input class="textfield" id="settings-shell-session" value="${esc(prefs.default_shell_session || "")}" autocomplete="off" ${disabled || pendingSupportDisabled ? "disabled" : ""} /></label>
              </div>
              <div class="settings-group">
                <p class="setting-group-title">Advanced behavior</p>
                <label>Log level${selectHTML("settings-log-level", prefs.log_level || "info", ["trace", "debug", "info", "warn", "error"], disabled)}</label>
                <label class="checkbox-row"><input type="checkbox" id="settings-disable-portmap" ${runtime.disable_portmap ? "checked" : ""} ${disabled || pendingSupportDisabled ? "disabled" : ""} /> Disable portmap</label>
                <label class="checkbox-row"><input type="checkbox" id="settings-disable-assisted" ${runtime.disable_assisted_addrs ? "checked" : ""} ${disabled || pendingSupportDisabled ? "disabled" : ""} /> Disable assisted addresses</label>
              </div>
            </div>
            ${settingsState.message ? `<div class="helper">${esc(settingsState.message)}</div>` : ""}
            ${failure ? `<div class="helper helper-error">${esc(`Save failed: ${bridgeErrorSummary(failure)}`)}</div>` : ""}
            ${failureSuggestions.length ? `<div class="list">${failureSuggestions.map((s) => listItemHTML(s.message || "")).join("")}</div>` : ""}
          </form>
          <aside class="surface-panel">
            <div class="card-header"><div><p class="eyebrow">Current</p><h3 class="card-title">Active settings</h3></div></div>
            <div class="detail-table">
              ${detailRowHTML("MQTT", csvValue(effective.runtime.mqtt_brokers))}
              ${detailRowHTML("P2P", effective.runtime.p2p_network)}
              ${detailRowHTML("IP family", effective.runtime.p2p_ip_family)}
              ${detailRowHTML("Data", effective.runtime.data_proto)}
              ${detailRowHTML("QUIC CC", effective.runtime.quic_cc)}
              ${detailRowHTML("STUN", csvValue(effective.runtime.stun))}
              ${detailRowHTML("Log level", effective.preferences.log_level)}
              ${detailRowHTML("Runtime apply", apply.runtime || "-")}
              ${detailRowHTML("Preferences apply", apply.preferences || "-")}
              ${detailRowHTML("Reconnect", apply.requires_reconnect ? "required for active sessions" : "not required")}
            </div>
          </aside>
        </section>`;
    } else if (section === "diagnostics") {
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
        <section class="settings-layout">
          <div class="surface-panel">
            <div class="card-header"><div><p class="eyebrow">Suggestions</p><h3 class="card-title">Connection guidance</h3></div></div>
            <div class="list">
              ${failure && failure.message ? listItemHTML(failure.message) : ""}
              ${suggestions.length ? suggestions.map((s) => listItemHTML(s.message || "")).join("") : listItemHTML(COPY.empty.errors, "empty")}
            </div>
          </div>
          <aside class="surface-panel">
            <div class="card-header"><div><p class="eyebrow">Diagnostics</p><h3 class="card-title">Local daemon</h3></div></div>
            <details class="technical-log" open>
              <summary>Technical details</summary>
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
            </details>
          </aside>
          <div class="surface-panel">
            <div class="card-header"><div><p class="eyebrow">Export</p><h3 class="card-title">Diagnostics archive</h3></div></div>
            <div class="action-row">
              <button class="btn btn-primary" id="btn-export-diagnostics" type="button" ${state.previewMode ? "disabled" : ""}>Export diagnostics</button>
            </div>
            <div class="helper">${esc(settingsState.exportPath || "")}</div>
          </div>
        </section>`;
    } else if (section === "preview") {
      body = `
        <section class="surface-panel">
          <div class="detail-table">
            ${detailRowHTML("Preview mode", state.previewMode ? "enabled" : "disabled")}
            ${detailRowHTML("Fixture", state.previewFixture)}
          </div>
        </section>`;
    } else {
      const override = lastConn && typeof lastConn.override_addr === "string" ? lastConn.override_addr : "";
      body = `
        <section class="surface-panel">
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
        ${pageHeadingHTML("Settings detail", section === "diagnostics" ? "Diagnostics" : section === "preview" ? "Preview" : section === "runtime" ? "Runtime config" : "Local daemon")}
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

  const renderTaskSummary = (taskObj) => taskObj
    ? `${operationStatusHTML(taskObj, "Progress", "Ready.")}${technicalLogHTML(taskObj)}`
    : operationStatusHTML(null, "Progress", "Ready.");

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
      <div class="activity-row">
        <div>
          <div class="row-title">${esc(t.kind || "(unknown)")}</div>
          <div class="row-meta">${esc(userTaskMessage(t, "Ready."))}</div>
        </div>
        ${chipHTML(t.status || "-", taskStatusClass(t))}
      </div>`).join("") : listItemHTML(COPY.empty.tasks, "empty");
    return `
      <section class="surface-panel">
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

  const latestInviteCode = () => {
    if (inviteState.taskID) {
      const code = findInviteCode(state.tasks.get(inviteState.taskID));
      if (code) return code;
    }
    const invites = [...state.tasks.values()]
      .filter((taskObj) => String(taskObj && taskObj.kind || "") === "invite")
      .sort((a, b) => String(b.created_at || "").localeCompare(String(a.created_at || "")));
    for (const taskObj of invites) {
      const code = findInviteCode(taskObj);
      if (code) return code;
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
      const peerID = String(args && args.peer_id || "").trim();
      if (peerID) {
        const current = Array.isArray(state.peerSessions) ? state.peerSessions.slice() : [];
        const index = current.findIndex((item) => String(item && (item.peer_id || item.remote_peer_id) || "").trim() === peerID);
        const next = {
          ...(index >= 0 ? current[index] : {}),
          peer_id: peerID,
          healthy: true,
          path_family: "udp4",
          protocol: "quic",
          ping_gate_satisfied: true,
          shell_ready_unix_ms: Date.now(),
        };
        if (index >= 0) current[index] = next;
        else current.push(next);
        state.peerSessions = current;
      }
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

  const runtimeActionName = (kind) => ({
    init_network: "init-network",
    sh_attach: "sh",
    revoke_member: "revoke",
  }[kind] || kind);

  const runtimeActionTimeout = (action) => {
    if (action === "approve" || action === "join") return 185000;
    if (action === "sh") return 125000;
    return 30000;
  };

  const runtimeActionArgs = (kind, args) => {
    const taskArgs = normalizeTaskArgs(args);
    if (kind === "init_network") {
      return taskArgs.create_new ? { create_new: true, confirm: taskArgs.confirm || "create-new-network" } : {};
    }
    return taskArgs;
  };

  const parseActionData = (value) => {
    if (!value) return {};
    if (typeof value === "object" && !Array.isArray(value)) return value;
    if (Array.isArray(value)) {
      try {
        return JSON.parse(new TextDecoder("utf-8").decode(new Uint8Array(value)));
      } catch {
        return {};
      }
    }
    if (typeof value === "string") {
      try {
        return JSON.parse(value);
      } catch {
        return {};
      }
    }
    return {};
  };

  const taskFromActionResult = (kind, args, result) => {
    const actionResult = result || {};
    const evidence = actionResult.evidence || {};
    const data = parseActionData(actionResult.data);
    const shellSessionID = String(actionResult.shell_session_id || data.shell_session_id || "").trim();
    const taskID = shellSessionID || `${kind}-${Date.now()}-${String(previewTaskSeq++).padStart(3, "0")}`;
    const facts = Array.isArray(evidence.facts) ? evidence.facts.slice() : [];
    const suggestions = Array.isArray(evidence.suggestions) ? evidence.suggestions.slice() : [];
    if (shellSessionID && !facts.some((f) => String(f && f.message || "").startsWith("shell_session_id="))) {
      facts.push({ message: `shell_session_id=${shellSessionID}` });
    }
    const taskObj = {
      task_id: taskID,
      kind,
      status: "done",
      stage: actionResult.stage || "",
      reason_code: actionResult.reason_code || "OK",
      exit_code: typeof actionResult.exit_code === "number" ? actionResult.exit_code : 0,
      report_ready: hasText(actionResult.report_markdown),
      report_markdown: actionResult.report_markdown || "",
      created_at: new Date().toISOString(),
      facts,
      suggestions,
      data,
    };
    attachPeerFact(taskObj, args && args.peer_id);
    return taskObj;
  };

  const createRunningTask = (kind, args) => {
    const taskObj = {
      task_id: `${kind}-${Date.now()}-${String(previewTaskSeq++).padStart(3, "0")}`,
      kind,
      status: "running",
      stage: "running",
      reason_code: "",
      exit_code: 0,
      report_ready: false,
      created_at: new Date().toISOString(),
      facts: [],
      suggestions: [],
    };
    attachPeerFact(taskObj, args && args.peer_id);
    upsertTask(taskObj);
    return taskObj;
  };

  const updateRunningTaskFromResult = (taskID, kind, args, result) => {
    const taskObj = taskFromActionResult(kind, args, result);
    taskObj.task_id = taskID || taskObj.task_id;
    taskObj.status = "done";
    upsertTask(taskObj);
    if (result && result.snapshot) applyDesktopSnapshot(result.snapshot);
    scheduleRender();
    return taskObj;
  };

  const updateRunningTaskFromError = (taskID, kind, message) => {
    const taskObj = state.tasks.get(taskID) || { task_id: taskID, kind, facts: [], suggestions: [] };
    taskObj.status = "done";
    taskObj.reason_code = "ERROR";
    taskObj.exit_code = 1;
    taskObj.stage = "failed";
    taskObj.facts = mergeItems(Array.isArray(taskObj.facts) ? taskObj.facts : [], [{ message: String(message || "action failed") }]);
    upsertTask(taskObj);
    scheduleRender();
    return taskObj;
  };

  const createTask = async (kind, args) => {
    const taskArgs = normalizeTaskArgs(args);
    if (state.previewMode) return createPreviewTask(kind, taskArgs);
    const bridge = getBridge();
    if (typeof bridge.RuntimeAction !== "function") throw new Error("desktop bridge does not expose RuntimeAction");
    const action = runtimeActionName(kind);
    const resp = await withTimeout(bridge.RuntimeAction(action, runtimeActionArgs(kind, taskArgs)), `Run ${action}`, runtimeActionTimeout(action));
    if (!resp || !resp.ok) throw new Error(bridgeErrorSummary(resp && resp.error));
    if (resp.result && resp.result.snapshot) applyDesktopSnapshot(resp.result.snapshot);
    return taskFromActionResult(kind, taskArgs, resp.result);
  };

  const startBackgroundTask = (kind, args, callbacks = {}) => {
    const taskArgs = normalizeTaskArgs(args);
    if (state.previewMode) {
      const taskObj = createPreviewTask(kind, taskArgs);
      if (typeof callbacks.done === "function") callbacks.done(taskObj);
      return taskObj;
    }
    const taskObj = createRunningTask(kind, taskArgs);
    const action = runtimeActionName(kind);
    void (async () => {
      try {
        const resp = await withTimeout(getBridge().RuntimeAction(action, runtimeActionArgs(kind, taskArgs)), `Run ${action}`, runtimeActionTimeout(action));
        if (!resp || !resp.ok) throw new Error(bridgeErrorSummary(resp && resp.error));
        const done = updateRunningTaskFromResult(taskObj.task_id, kind, taskArgs, resp.result);
        if (typeof callbacks.done === "function") callbacks.done(done);
      } catch (err) {
        const failed = updateRunningTaskFromError(taskObj.task_id, kind, String(err));
        if (typeof callbacks.error === "function") callbacks.error(err, failed);
      }
    })();
    return taskObj;
  };

  const getTask = async (taskID, timeoutMs = 12000) => {
    if (state.previewMode) return state.tasks.get(taskID) || null;
    return state.tasks.get(taskID) || null;
  };

  const exportReport = async (taskID, onSavedPath) => {
    if (state.previewMode) {
      const fakePath = `/tmp/${taskID || "preview-report"}.md`;
      if (typeof onSavedPath === "function") onSavedPath(fakePath);
      toast(`Preview report: ${fakePath}`);
      return { path: fakePath };
    }
    const taskObj = state.tasks.get(taskID);
    if (!taskObj || !hasText(taskObj.report_markdown)) throw new Error("report is not available for this action");
    const blob = new Blob([taskObj.report_markdown], { type: "text/markdown" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `${taskID}.md`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
    const path = `download:${taskID}.md`;
    if (typeof onSavedPath === "function") onSavedPath(path);
    toast("Report downloaded");
    return { path };
  };

  const runPeerTask = async (kind) => {
    const peerID = state.view.type === "peer" || state.view.type === "shell-peer"
      ? String(state.view.peerID || "")
      : String(shellView.peerID || "");
    if (!peerID) return;
    const args = kind === "sh_ls" ? { peer_id: peerID, target: "" } : { peer_id: peerID };
    try {
      const created = await createTask(kind, args);
      upsertTask(attachPeerFact(created, peerID));
      toast(kind === "ping" ? "Ping started" : "Task started");
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

  const concatBytes = (a, b) => {
    const left = a instanceof Uint8Array ? a : new Uint8Array(0);
    const right = b instanceof Uint8Array ? b : new Uint8Array(0);
    const out = new Uint8Array(left.length + right.length);
    out.set(left, 0);
    out.set(right, left.length);
    return out;
  };

  const encodeShellFrame = (kind, payload) => {
    const body = payload instanceof Uint8Array ? payload : new Uint8Array(payload || []);
    const out = new Uint8Array(5 + body.length);
    out[0] = kind & 0xff;
    const view = new DataView(out.buffer);
    view.setUint32(1, body.length, false);
    out.set(body, 5);
    return out;
  };

  const encodeShellJSON = (value) => encodeShellFrame(1, new TextEncoder().encode(JSON.stringify(value || {})));

  const consumeShellFrames = (incoming, onFrame) => {
    shellState.frameBuffer = concatBytes(shellState.frameBuffer, incoming);
    const maxFrame = 4 << 20;
    for (;;) {
      const buf = shellState.frameBuffer;
      if (buf.length < 5) return;
      const kind = buf[0];
      const view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
      const length = view.getUint32(1, false);
      if ((kind !== 0 && kind !== 1) || length > maxFrame) {
        shellState.frameBuffer = new Uint8Array(0);
        onFrame(0, buf);
        return;
      }
      if (buf.length < 5 + length) return;
      onFrame(kind, buf.slice(5, 5 + length));
      shellState.frameBuffer = buf.slice(5 + length);
    }
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
      ws.send(encodeShellJSON({ op: "winsize", winsize: { cols, rows } }));
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
    if (shellState.term) {
      const term = shellState.term;
      shellState.term = null;
      disposeShellTerminal(term);
    }
    container.textContent = "";
    if (typeof window.Terminal !== "function") {
      container.textContent = "xterm.js failed to load";
      throw new Error("xterm.js failed to load");
    }
    const term = new window.Terminal({
      cursorBlink: true,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
      fontSize: 13,
      scrollback: 5000,
      theme: { background: "#0b1014", foreground: "#e8edf2", cursor: "#82d2ff" },
    });
    shellState.term = term;
    term.open(container);
    installTerminalScroll(container, term);
    term.focus();
    return term;
  };

  const openPreviewShellTask = (taskID, peerID, target, session, mode = "connected") => {
    shellView.taskID = taskID;
    shellView.activeSessionTaskID = taskID;
    shellView.discoveryTaskID = "";
    shellState.taskID = taskID;
    const term = openTerminal();
    term.writeln(mode === "resumed" ? "miopunch preview shell resumed" : "miopunch preview shell");
    term.writeln(`device=${peerTitle(peerID)}`);
    term.writeln(`session=${session}`);
    term.writeln("");
    term.write("$ ");
    shellView.phase = "connected";
    shellView.detail = mode === "resumed"
      ? `Resumed ${target}/${session}.`
      : `Connected to ${target}/${session}.`;
    shellView.error = "";
    rememberShellSelection();
    syncShellDOM();
  };

  const startPreviewShell = async (peerID, target, session) => {
    const created = await createTask("sh_attach", { peer_id: peerID, target, session });
    upsertTask(attachPeerFact(created, peerID));
    openPreviewShellTask(created.task_id, peerID, target, session);
  };

  const attachLiveShellTask = async (taskID, peerID, target, session, mode = "new") => {
    const encoder = new TextEncoder();
    const decoder = new TextDecoder("utf-8");
    shellView.taskID = taskID;
    shellView.activeSessionTaskID = taskID;
    shellView.discoveryTaskID = "";
    shellState.taskID = taskID;
    shellState.expectedClose = false;
    shellState.wsError = "";
    shellState.remoteDataSeen = false;
    shellState.remoteExit = null;
    shellView.detail = mode === "resume" ? "Resuming shell session..." : "Opening shell...";
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
    shellView.detail = mode === "resume" ? `Resuming ${target}/${session}...` : `Connecting to ${target}/${session}...`;
    syncShellDOM();
    const term = openTerminal();
    term.writeln(mode === "resume" ? "Resuming..." : "Connecting...");
    syncShellDOM();

    const wsURL = `${baseURL}/api/v1/shell/${encodeURIComponent(taskID)}/ws?token=${encodeURIComponent(token)}`;
    const ws = new WebSocket(wsURL, [subprotocol]);
    ws.binaryType = "arraybuffer";
    shellState.ws = ws;
    ws.onopen = () => {
      if (shellState.ws !== ws) return;
      shellView.phase = "connecting";
      shellView.detail = `Waiting for remote shell output from ${target}/${session}...`;
      shellView.error = "";
      rememberShellSelection();
      syncShellDOM();
      fitAndSendWinSize();
    };
    ws.onmessage = (msg) => {
      if (shellState.ws !== ws || !shellState.term) return;
      let incoming = new Uint8Array(0);
      if (typeof msg.data === "string") {
        incoming = new TextEncoder().encode(msg.data);
      } else {
        incoming = new Uint8Array(msg.data);
      }
      consumeShellFrames(incoming, (kind, payload) => {
        if (kind === 1) {
          try {
            const control = JSON.parse(decoder.decode(payload));
            if (control && control.op === "shell_exit") {
              const controlError = control.error && (control.error.message || control.error.reason_code)
                ? compactStatusText([control.error.reason_code, control.error.message].filter(Boolean).join(": "))
                : "";
              const ok = control.ok !== false && !controlError;
              const detail = ok
                ? "Remote shell exited."
                : compactStatusText(controlError || "Remote shell exited with an error.");
              shellState.remoteExit = { ok, detail };
              shellView.phase = ok ? "disconnected" : "failed";
              shellView.detail = detail;
              shellView.error = ok ? "" : detail;
              syncShellDOM();
            }
          } catch {
            // ignore malformed shell control frames
          }
          return;
        }
        const output = decoder.decode(payload);
        if (!shellState.remoteDataSeen && payload.byteLength > 0) {
          shellState.remoteDataSeen = true;
          shellView.phase = "connected";
          shellView.detail = mode === "resume" ? `Resumed ${target}/${session}.` : `Connected to ${target}/${session}.`;
          shellView.error = "";
          rememberShellSelection();
          syncShellDOM();
        }
        shellState.term.write(output);
      });
    };
    ws.onerror = () => {
      if (shellState.ws !== ws) return;
      shellState.wsError = "terminal websocket error";
      shellView.detail = "Terminal bridge reported an error.";
      syncShellDOM();
    };
    ws.onclose = (event) => {
      const expectedClose = shellState.expectedClose;
      const remoteExit = shellState.remoteExit;
      const normalSocketClose = event && event.code === 1000 && !shellState.wsError;
      const wasConnected = shellView.phase === "connected" || !!remoteExit || normalSocketClose;
      const reason = shellSocketCloseReason(event, shellState.wsError);
      const taskID = shellView.taskID;
      shellState.expectedClose = false;
      if (expectedClose) return;
      if (remoteExit && remoteExit.ok) {
        closeShellTransport(1000, "shell exited", { keepTerminal: true });
        shellView.phase = "disconnected";
        shellView.detail = remoteExit.detail || "Remote shell exited.";
        shellView.error = "";
        rememberShellSelection();
        scheduleRender();
        return;
      }
      if (remoteExit && !remoteExit.ok) {
        closeShellTransport();
        shellView.phase = "failed";
        shellView.detail = shellPhaseDefaultDetail("failed");
        shellView.error = remoteExit.detail || "Remote shell exited with an error.";
        rememberShellSelection();
        scheduleRender();
        return;
      }
      if (normalSocketClose) {
        closeShellTransport(1000, "shell closed", { keepTerminal: true });
        shellView.phase = "disconnected";
        shellView.detail = event && event.reason ? compactStatusText(event.reason) : "Terminal connection closed.";
        shellView.error = "";
        rememberShellSelection();
        scheduleRender();
        return;
      }
      closeShellTransport(1000, "shell closed", { keepTerminal: wasConnected });
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
        if (ws && ws.readyState === WebSocket.OPEN) ws.send(encodeShellFrame(0, encoder.encode(data)));
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

  const startLiveShell = async (peerID, target, session) => {
    const created = await createTask("sh_attach", { peer_id: peerID, target, session });
    upsertTask(attachPeerFact(created, peerID));
    await attachLiveShellTask(created.task_id, peerID, target, session, "new");
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
      host.addEventListener("contextmenu", handlePageContextMenu);
      host.addEventListener("keydown", handlePageKeydown);
      host.addEventListener("submit", handlePageSubmit);
      host.addEventListener("toggle", handlePageToggle, true);
    }
    document.addEventListener("keydown", handleGlobalKeydown);

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
      window.runtime.EventsOn("localapi:connection", (conn) => {
        try {
          renderConnection(conn);
          scheduleRender();
        } catch {
          // ignore malformed connection payload
        }
      });
    }
  };

  const handlePageKeydown = (event) => {
    if (event.key === "Escape" && shellTargetContextMenu) {
      shellTargetContextMenu = null;
      scheduleRender();
      return;
    }
    if (event.key !== "Enter" && event.key !== " ") return;
    const target = event.target.closest("[data-map-peer]");
    if (!target) return;
    event.preventDefault();
    state.networkMapPeerID = target.dataset.mapPeer;
    scheduleRender();
  };

  const handleGlobalKeydown = (event) => {
    if (event.key !== "Escape" || !shellTargetContextMenu) return;
    shellTargetContextMenu = null;
    scheduleRender();
  };

  const handlePageToggle = (event) => {
    const details = event.target && event.target.closest ? event.target.closest(".shell-options-panel") : null;
    if (!details) return;
    shellView.optionsOpen = !!details.open;
  };

  const handlePageContextMenu = (event) => {
    const target = event.target && event.target.closest ? event.target.closest("[data-shell-target]") : null;
    if (!target) return;
    event.preventDefault();
    const peerID = target.dataset.shellPeer || shellView.peerID;
    const targetName = String(target.dataset.shellTarget || "").trim();
    if (!peerID || !targetName) return;
    shellTargetContextMenu = { peerID, target: targetName, x: event.clientX, y: event.clientY };
    scheduleRender();
  };

  const handlePageClick = async (event) => {
    const target = event.target.closest("button, a, [data-map-peer]");
    if (!target) {
      if (shellTargetContextMenu) {
        shellTargetContextMenu = null;
        scheduleRender();
      }
      return;
    }
    if (!target.dataset.shellTarget && !target.dataset.shellDeleteTarget) shellTargetContextMenu = null;

    if (target.dataset.mapPeer) {
      event.preventDefault();
      state.networkMapPeerID = target.dataset.mapPeer;
      scheduleRender();
      return;
    }
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
      if (target.dataset.peerSection === "shell") {
        if (peerID) syncShellSelectionForPeer(peerID);
        state.activeTab = "shell";
        state.view = { type: "shell-peer", peerID };
        localStorage.setItem("miopunch_desktop_tab", "shell");
        scheduleRender();
        return;
      }
      navigate({ type: "peer", peerID, section: target.dataset.peerSection });
      return;
    }
    if (target.dataset.shellTarget !== undefined) {
      event.preventDefault();
      const peerID = target.dataset.shellPeer || shellView.peerID;
      const targetName = String(target.dataset.shellTarget || "").trim() || shellDefaultTarget(peerID);
      syncShellSelectionForPeer(peerID);
      state.view = { type: "shell-peer", peerID };
      await openShellTarget(peerID, targetName, shellDefaultSession(peerID));
      return;
    }
    if (target.dataset.shellSessionName !== undefined) {
      event.preventDefault();
      const value = String(target.dataset.shellSessionName || "").trim();
      if (!value) return;
      const peerID = state.view.type === "shell-peer" ? state.view.peerID : shellView.peerID;
      syncShellSelectionForPeer(peerID);
      await openShellTarget(peerID, shellView.target || shellDefaultTarget(peerID), value);
      return;
    }
    if (target.dataset.shellPeer) {
      event.preventDefault();
      syncShellSelectionForPeer(target.dataset.shellPeer);
      state.view = { type: "shell-peer", peerID: target.dataset.shellPeer };
      scheduleRender();
      return;
    }
    if (target.dataset.shellSessionTask && target.id !== "btn-shell-resume") {
      event.preventDefault();
      const taskID = String(target.dataset.shellSessionTask || "").trim();
      if (!taskID) return;
      const peerID = target.dataset.shellPeer || (state.view.type === "shell-peer" ? state.view.peerID : shellView.peerID);
      const session = shellSessionsForPeer(peerID)
        .find((item) => String(item && item.task_id || "") === taskID);
      if (session) {
        if (!shellSessionAttachable(session)) {
          failShellAction("Resume failed: shell session is not attachable. Create another session instead.");
        } else {
          await resumeShellSession(taskID);
        }
      }
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
    if (target.id === "btn-init-network-bootstrap") {
      event.preventDefault();
      if (!canInitOwner()) return;
      await initNetwork("bootstrap");
      return;
    }
    if (target.id === "btn-init-network-new") {
      event.preventDefault();
      if (!canCreateNewNetwork()) return;
      await initNetwork("create_new");
      return;
    }
    if (target.id === "btn-export-diagnostics") {
      event.preventDefault();
      await exportDiagnostics();
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
    if (target.id === "btn-shell-find-sessions") {
      event.preventDefault();
      await discoverShell("sessions");
      return;
    }
    if (target.id === "btn-shell-add-target") {
      event.preventDefault();
      addShellTargetFromInput();
      return;
    }
    if (target.dataset.shellDeleteTarget !== undefined) {
      event.preventDefault();
      deleteShellTarget(target.dataset.shellDeleteTarget);
      return;
    }
    if (target.dataset.shellToggle) {
      event.preventDefault();
      const pane = String(target.dataset.shellToggle || "");
      if (pane === "left") {
        if (shellView.zen) shellView.zen = false;
        shellView.leftCollapsed = !shellView.leftCollapsed;
      } else if (pane === "right") {
        if (shellView.zen) shellView.zen = false;
        shellView.rightOpen = !shellView.rightOpen;
      } else if (pane === "zen") {
        shellView.zen = !shellView.zen;
      }
      rememberShellSelection();
      scheduleRender();
      return;
    }
    if (target.id === "btn-shell-resume") {
      event.preventDefault();
      await resumeShellSession(target.dataset.shellSessionTask || shellView.activeSessionTaskID || shellView.taskID);
      return;
    }
    if (target.id === "btn-shell-disconnect" || target.dataset.shellDisconnect !== undefined) {
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
      else if (form.id === "runtime-config-form") await submitRuntimeConfig();
      else if (form.id === "shell-form") await submitShell();
      else if (form.id === "shell-target-form") await submitShell();
      else if (form.id === "alias-form") await submitAlias();
    } catch (err) {
      toast(String(err));
    }
  };

  const submitAlias = async () => {
    const peerID = el("alias-peer-id") ? el("alias-peer-id").value.trim() : "";
    const value = el("alias-name") ? el("alias-name").value.trim() : "";
    if (!peerID) return;
    state.localAliases = { ...(state.localAliases || {}), [peerID]: value };
    toast(value ? "Alias saved locally" : "Alias cleared locally");
    scheduleRender();
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

  const startInviteApprovalListener = async (code) => {
    const inviteCode = String(code || "").trim();
    if (!inviteCode) return;
    if (inviteState.approvalCode === inviteCode && inviteState.approvalTaskID) return;

    inviteState.approvalCode = inviteCode;
    inviteState.message = "Invite ready. Listening for join requests...";
    scheduleRender();
    try {
      const created = startBackgroundTask("approve", { code: inviteCode }, {
        done: () => {
          inviteState.message = "Invite ready. Approval completed.";
          approveState.message = "";
          scheduleRender();
        },
        error: (err) => {
          inviteState.approvalTaskID = "";
          inviteState.message = `Invite ready, but approval listener failed: ${String(err)}`;
          toast(inviteState.message);
          scheduleRender();
        },
      });
      const taskID = upsertTask(created);
      inviteState.approvalTaskID = taskID;
      approveState.taskID = taskID;
      approveState.message = "";
      inviteState.message = "Invite ready. Approval listener is running.";
    } catch (err) {
      inviteState.approvalTaskID = "";
      inviteState.message = `Invite ready, but approval listener failed: ${String(err)}`;
      toast(inviteState.message);
    } finally {
      scheduleRender();
    }
  };

  const initNetwork = async (mode) => {
    const createNew = mode === "create_new";
    if (createNew && !window.confirm("Create a new local network for this node? Existing members must be invited again.")) {
      return;
    }
    const args = createNew
      ? { create_new: true, confirm: "create-new-network" }
      : {};
    try {
      const created = await createTask("init_network", args);
      const taskID = upsertTask(created);
      if (taskID) await getTask(taskID, 2500);
      const resp = await getBridge().DesktopRuntimeResync();
      if (resp && resp.state) applyDesktopSnapshot(resp.state);
      if (adminVisible()) setActiveTab("admin");
      toast(createNew ? "New network created" : "Owner/Admin mode enabled");
    } catch (err) {
      toast(`Network setup failed: ${String(err)}`);
    } finally {
      scheduleRender();
    }
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
      const finalTask = taskID ? await waitForInviteTaskOutput(taskID) : null;
      const code = findInviteCode(finalTask || (taskID ? state.tasks.get(taskID) : null));
      if (code) void startInviteApprovalListener(code);
      else inviteState.message = "";
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
    const created = startBackgroundTask("approve", { code }, {
      done: () => {
        approveState.message = "Approval completed.";
        scheduleRender();
      },
      error: (err) => {
        approveState.message = `Approval failed: ${String(err)}`;
        toast(approveState.message);
        scheduleRender();
      },
    });
    approveState.taskID = upsertTask(created);
    approveState.message = "Approval listener is running.";
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

  const submitRuntimeConfig = async () => {
    if (state.previewMode || settingsState.saving) return;
    settingsState.saving = true;
    settingsState.message = "Saving log level...";
    settingsState.failure = null;
    scheduleRender();
    try {
      if (typeof getBridge().SaveDesktopConfig !== "function") {
        settingsState.message = "";
        settingsState.failure = { message: "Log level save is not supported by this LocalAPI bridge." };
        toast("Log level save is not supported by this bridge");
        return;
      }
      const update = {
        preferences: {
          log_level: el("settings-log-level") ? el("settings-log-level").value : "info",
        },
      };
      const resp = await withTimeout(getBridge().SaveDesktopConfig(update), "Save log level");
      renderConnection(resp && resp.connection ? resp.connection : null);
      if (!resp || !resp.ok) {
        const failure = resp && resp.error ? resp.error : { message: "unknown error" };
        settingsState.message = "";
        settingsState.failure = failure;
        toast(`Save failed: ${bridgeErrorSummary(failure)}`);
        return;
      }
      if (resp.state) applyDesktopSnapshot(resp.state);
      settingsState.message = "Log level saved";
      toast("Log level saved");
    } catch (err) {
      const failure = { message: String(err) };
      settingsState.message = "";
      settingsState.failure = failure;
      toast(`Save failed: ${bridgeErrorSummary(failure)}`);
    } finally {
      settingsState.saving = false;
      scheduleRender();
    }
  };

  const exportDiagnostics = async () => {
    if (state.previewMode) return;
    try {
      const resp = await withTimeout(getBridge().ExportDiagnostics(), "Export diagnostics");
      if (!resp || !resp.ok) throw new Error(bridgeErrorSummary(resp && resp.error));
      if (resp.cancelled) return;
      settingsState.exportPath = resp.path || "";
      toast("Diagnostics exported");
      scheduleRender();
    } catch (err) {
      toast(`Export failed: ${String(err)}`);
    }
  };

  const discoverShell = async (mode = "targets") => {
    const peerID = state.view.type === "shell-peer" ? String(state.view.peerID || "").trim() : String(shellView.peerID || "").trim();
    const targetInput = el("shell-target");
    const typedTarget = targetInput ? targetInput.value.trim() : "";
    const discoverSessions = mode === "sessions";
    if (!peerID) {
      failShellAction("Shell lookup failed: missing device");
      return;
    }

    syncShellSelectionForPeer(peerID);
    state.view = { type: "shell-peer", peerID };
    shellView.peerID = peerID;
    shellView.target = typedTarget || shellView.target || shellDefaultTarget(peerID);
    shellView.session = shellView.session || shellDefaultSession(peerID);

    const target = discoverSessions ? shellView.target : "";
    const restingPhase = shellView.phase === "disconnected" ? "disconnected" : "idle";

    shellView.phase = "listing";
    shellView.detail = discoverSessions ? `Listing sessions for ${target}...` : "Listing shell targets...";
    shellView.error = "";
    shellView.taskID = "";
    shellView.discoveryTaskID = "";
    shellView.optionsOpen = true;
    rememberShellSelection();
    scheduleRender();

    try {
      const created = await createTask("sh_ls", { peer_id: peerID, target });
      const taskID = upsertTask(attachPeerFact(created, peerID));
      shellView.discoveryTaskID = taskID;
      scheduleRender();

      const latest = await waitForShellTaskOutput(taskID);
      const taskObj = state.tasks.get(taskID) || latest || created;
      if (shellTaskFailed(taskObj)) throw new Error(taskFailureSummary(taskObj));

      if (discoverSessions) {
        const sessions = shellTaskValues(taskObj, "session", "session=");
        shellView.sessionOptions = sessions;
        shellView.sessionTarget = target;
        if (!el("shell-session") || !el("shell-session").value.trim()) {
          shellView.session = sessions.includes("main") ? "main" : (sessions[0] || shellDefaultSession(peerID));
        }
        shellView.detail = sessions.length ? `Session names discovered for ${target}.` : `No session names discovered for ${target}.`;
      } else {
        const targets = shellTaskValues(taskObj, "target", "target=");
        shellView.targetOptions = targets;
        shellView.sessionOptions = [];
        shellView.sessionTarget = "";
        if (!typedTarget) {
          shellView.target = targets.includes("local") ? "local" : (targets[0] || shellDefaultTarget(peerID));
        }
        shellView.detail = targets.length ? "Targets discovered." : "No targets discovered.";
      }

      shellView.phase = restingPhase;
      shellView.error = "";
      shellView.optionsOpen = true;
      rememberShellSelection();
      scheduleRender();
    } catch (err) {
      shellView.phase = "failed";
      shellView.detail = discoverSessions ? "Session discovery failed. Retry is available." : "Target discovery failed. Retry is available.";
      shellView.error = `Shell lookup failed: ${String(err)}`;
      shellView.optionsOpen = true;
      rememberShellSelection();
      scheduleRender();
      toast(shellView.error);
    }
  };

  const addShellTargetFromInput = () => {
    const peerID = state.view.type === "shell-peer" ? String(state.view.peerID || "").trim() : String(shellView.peerID || "").trim();
    const targetInput = el("shell-target");
    const target = targetInput && targetInput.value.trim() ? targetInput.value.trim() : "";
    if (!peerID || !target) {
      failShellAction("Add target failed: enter a target name first.");
      return;
    }
    syncShellSelectionForPeer(peerID);
    shellView.target = target;
    shellView.targetOptions = [...new Set([...(shellView.targetOptions || []), target])];
    shellView.detail = `${target} added. Click the target to open it.`;
    shellView.error = "";
    shellTargetContextMenu = null;
    rememberShellSelection(peerID);
    scheduleRender();
  };

  const deleteShellTarget = (value) => {
    const peerID = state.view.type === "shell-peer" ? String(state.view.peerID || "").trim() : String(shellView.peerID || "").trim();
    const target = String(value || "").trim();
    if (!peerID || !target || target === "local") return;
    const hasLiveSession = shellSessionsForPeer(peerID).some((item) => String(item.target || "local").trim() === target);
    if (hasLiveSession) {
      failShellAction("Delete target failed: close or resume live sessions for this target first.");
      return;
    }
    shellView.targetOptions = (shellView.targetOptions || []).filter((item) => String(item || "").trim() !== target);
    if (shellView.target === target) shellView.target = shellDefaultTarget(peerID);
    shellView.detail = `${target} deleted.`;
    shellView.error = "";
    shellTargetContextMenu = null;
    rememberShellSelection(peerID);
    scheduleRender();
  };

  const resumeShellSession = async (taskID) => {
    const peerID = state.view.type === "shell-peer" ? String(state.view.peerID || "").trim() : String(shellView.peerID || "").trim();
    const id = String(taskID || "").trim();
    if (!peerID || !id) {
      failShellAction("Resume failed: no shell session is available");
      return;
    }
    const session = shellSessionsForPeer(peerID).find((item) => String(item && item.task_id || "").trim() === id);
    if (!session) {
      failShellAction("Resume failed: shell session is no longer available");
      return;
    }
    if (!shellSessionAttachable(session)) {
      failShellAction("Resume failed: shell session is not attachable. Open another shell instead.");
      return;
    }
    const target = String(session.target || "local").trim() || "local";
    const shellSession = String(session.session || "main").trim() || "main";

    closeShellTransport();
    state.view = { type: "shell-peer", peerID };
    shellView.peerID = peerID;
    shellView.target = target;
    shellView.session = shellSession;
    shellView.taskID = id;
    shellView.activeSessionTaskID = id;
    shellView.phase = "connecting";
    shellView.detail = `Resuming ${target}/${shellSession}...`;
    shellView.error = "";
    shellView.discoveryTaskID = "";
    rememberShellSelection();
    scheduleRender();

    try {
      if (state.previewMode) openPreviewShellTask(id, peerID, target, shellSession, "resumed");
      else await attachLiveShellTask(id, peerID, target, shellSession, "resume");
    } catch (err) {
      failShellAction(`Resume failed: ${String(err)}`);
    }
  };

  const openShellTarget = async (peerID, target, session) => {
    const id = String(peerID || "").trim();
    const shellTarget = String(target || "").trim() || shellDefaultTarget(id);
    const shellSession = String(session || "").trim() || shellDefaultSession(id);
    if (!id) {
      failShellAction("Connect failed: missing peer_id");
      return;
    }
    syncShellSelectionForPeer(id);
    if (!shellCanConnect(id)) {
      failShellAction("Connect failed: shell is not ready for a new session.");
      return;
    }
    shellView.targetOptions = [...new Set([...(shellView.targetOptions || []), shellTarget])];
    closeShellTransport();
    state.view = { type: "shell-peer", peerID: id };
    shellView.peerID = id;
    shellView.target = shellTarget;
    shellView.session = shellSession;
    shellView.phase = "connecting";
    shellView.detail = `Connecting to ${shellTarget}/${shellSession}...`;
    shellView.error = "";
    shellView.activeSessionTaskID = "";
    shellView.discoveryTaskID = "";
    shellTargetContextMenu = null;
    rememberShellSelection(id);
    scheduleRender();

    try {
      if (state.previewMode) await startPreviewShell(id, shellTarget, shellSession);
      else await startLiveShell(id, shellTarget, shellSession);
    } catch (err) {
      failShellAction(`Connect failed: ${String(err)}`);
    }
  };

  const submitShell = async () => {
    const peerID = state.view.type === "shell-peer" ? String(state.view.peerID || "").trim() : String(shellView.peerID || "").trim();
    const targetInput = el("shell-target");
    const sessionInput = el("shell-session");
    if (!peerID) {
      failShellAction("Connect failed: missing peer_id");
      return;
    }
    syncShellSelectionForPeer(peerID);
    const target = targetInput && targetInput.value.trim() ? targetInput.value.trim() : (shellView.target || shellDefaultTarget(peerID));
    const session = sessionInput && sessionInput.value.trim() ? sessionInput.value.trim() : (shellView.session || shellDefaultSession(peerID));
    const matchingSession = shellSessionsForPeer(peerID)
      .find((item) => String(item.target || "local").trim() === target && String(item.session || "main").trim() === session) || null;
    if (matchingSession) {
      if (!shellSessionAttachable(matchingSession)) {
        failShellAction("Resume failed: shell session is not attachable. Open another shell instead.");
        return;
      }
      await resumeShellSession(matchingSession.task_id);
      return;
    }
    await openShellTarget(peerID, target, session);
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
    state.activeTab = tab === "access" ? "network" : tab === "admin" && roleKnown() && !adminVisible() ? "network" : tab === "shell" && !shellVisible() ? "network" : tab;
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
