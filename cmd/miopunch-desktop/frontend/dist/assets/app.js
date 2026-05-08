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
    status: null,
    topology: null,
    peers: [],
    tasks: new Map(),
    activeTab: "network",
    view: { type: "overview" },
    previewMode: false,
    previewFixture: "owner",
  };

  const inviteState = { taskID: "", busy: false, message: "", missingTaskID: "" };
  const joinState = { taskID: "", lastExportPath: "" };
  const approveState = { taskID: "", message: "" };
  const shellState = { ws: null, term: null, resizeObs: null, taskID: "", fitTimer: 0 };

  let lastConn = null;
  let renderQueued = false;
  let previewTaskSeq = 1;

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
          reason_code: "LOCALAPI_UNREACHABLE",
          exit_code: 70,
          message: "LocalAPI is not reachable",
          suggestions: [{ message: "Start the miopunch service, then refresh" }],
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
          reason_code: "LOCALAPI_UNREACHABLE",
          exit_code: 70,
          report_ready: true,
          created_at: "2026-05-04T00:00:00Z",
          facts: [{ message: "LocalAPI is not reachable" }],
          suggestions: [{ message: "Start the daemon and retry" }],
        },
      ],
    },
  };

  const self = () => (state.topology && state.topology.self ? state.topology.self : {});
  const selfRole = () => String(self().role || "unknown").toLowerCase();
  const roleKnown = () => !!(state.topology && state.topology.self && state.topology.self.role);
  const isAdminRole = (role) => ["owner", "admin"].includes(String(role || "").toLowerCase());
  const adminVisible = () => isAdminRole(selfRole());

  const members = () => {
    const top = state.topology || {};
    const list = Array.isArray(top.members) ? top.members.slice() : [];
    const selfPeerID = String(self().peer_id || "");
    if (selfPeerID && !list.some((m) => m.peer_id === selfPeerID)) {
      list.unshift({
        peer_id: selfPeerID,
        role: self().role || "unknown",
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

  const statusForMember = (mem) => {
    if (!mem) return { label: "none", cls: "chip-muted" };
    if (mem.revoked) return { label: "revoked", cls: "chip-revoked" };
    if (String(mem.peer_id || "") === String(self().peer_id || "")) return { label: "this node", cls: "chip-role" };
    if (activeNeighbor(mem.peer_id)) return { label: "active", cls: "chip-active" };
    if (selectedNeighbor(mem.peer_id)) return { label: "selected", cls: "chip-running" };
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
    state.tasks.set(taskID, mergeTask(state.tasks.get(taskID), taskObj));
    return taskID;
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

  const handleGlobalEvent = (ev) => {
    const kind = String(ev.kind || "");
    if (kind === "snapshot") {
      const tasks = Array.isArray(ev.tasks) ? ev.tasks : [];
      const nextTasks = new Map();
      for (const t of tasks) {
        const taskID = canonicalTaskID(t.task_id);
        if (!taskID) continue;
        nextTasks.set(taskID, mergeTask(state.tasks.get(taskID), t));
      }
      state.tasks = nextTasks;
      scheduleRender();
      return;
    }
    const taskID = canonicalTaskID(ev.task_id);
    if (!taskID) return;
    let taskObj = state.tasks.get(taskID);
    if (!taskObj) {
      taskObj = { task_id: taskID, kind: "", status: "running", stage: "", facts: [], suggestions: [] };
      state.tasks.set(taskID, taskObj);
    }
    applyEventToTask(taskObj, ev);
    scheduleRender();
  };

  const renderConnection = (conn) => {
    lastConn = conn || null;
  };

  const loadPreviewFixture = (name) => {
    const selected = previewFixtures[name] ? name : "owner";
    const fx = clone(previewFixtures[selected]);
    state.previewFixture = selected;
    state.status = fx.status || null;
    state.topology = fx.topology || null;
    state.peers = fx.topology && Array.isArray(fx.topology.members)
      ? fx.topology.members.map((m) => ({ peer_id: m.peer_id }))
      : [];
    state.tasks = new Map((fx.tasks || []).map((t) => [canonicalTaskID(t.task_id), t]));
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
    if (leavingShell && !enteringShell) disconnectShell();
    state.view = view;
    scheduleRender();
  };

  const backToOverview = () => {
    navigate({ type: "overview" });
  };

  const setPage = (html) => {
    const host = el("page-host");
    if (!host) return;
    host.innerHTML = html;
    renderPostDOM();
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
    const selfPeerID = String(self().peer_id || "");
    const isRemote = !!(selected && selected.peer_id && selected.peer_id !== selfPeerID);
    const canOperate = !!(selected && selected.peer_id && isRemote && !selected.revoked && (state.previewMode || lastConn && lastConn.connected));
    let body = "";

    if (!selected) {
      body = `<section class="card">${listItemHTML("Peer was not found", "empty")}</section>`;
    } else if (section === "shell") {
      body = `
        <section class="card">
          <div class="card-header">
            <div>
              <p class="eyebrow">Remote session</p>
              <h3 class="card-title">Shell</h3>
            </div>
            <button class="btn btn-tonal" id="btn-shell-disconnect" ${shellState.ws || shellState.term ? "" : "disabled"}>Disconnect</button>
          </div>
          <form class="form-grid" id="shell-form">
            <div class="grid grid-3">
              <label>Peer ID<input class="textfield mono" id="shell-peer-id" value="${esc(peerID)}" autocomplete="off" /></label>
              <label>Target<input class="textfield" id="shell-target" value="local" autocomplete="off" /></label>
              <label>Session<input class="textfield" id="shell-session" value="main" autocomplete="off" /></label>
            </div>
            <div class="action-row">
              <button class="btn btn-primary" type="submit" ${canOperate ? "" : "disabled"}>Connect</button>
              <div class="helper" id="shell-status">-</div>
            </div>
          </form>
          <div class="terminal mt" id="terminal"></div>
        </section>`;
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
              ${detailRowHTML("Role", role)}
              ${detailRowHTML("IPv4", selected.v4_hint || "-")}
              ${detailRowHTML("IPv6", selected.v6_hint || "-")}
              ${detailRowHTML("Path", neighbor ? `${neighbor.data_proto || "-"} / ${neighbor.path_family || "-"}` : "-")}
              ${detailRowHTML("Selection", selectedEdge ? selectedEdge.reason || selectedEdge.bucket || "selected" : "-")}
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
      ${renderTaskSummary(taskObj)}`;
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
      </section>`);
  };

  const renderSettingsSection = (section) => {
    let body = "";
    if (section === "diagnostics") {
      const failure = lastConn && lastConn.failure ? lastConn.failure : null;
      const suggestions = failure && Array.isArray(failure.suggestions) ? failure.suggestions : [];
      const facts = failure && Array.isArray(failure.facts) ? failure.facts : [];
      body = `
        <section class="detail-grid">
          <div class="card">
            <div class="card-header"><div><p class="eyebrow">Suggestions</p><h3 class="card-title">Connection</h3></div></div>
            <div class="list">${suggestions.length ? suggestions.map((s) => listItemHTML(s.message || "")).join("") : listItemHTML(COPY.empty.errors, "empty")}</div>
          </div>
          <div class="card">
            <div class="card-header"><div><p class="eyebrow">Facts</p><h3 class="card-title">LocalAPI</h3></div></div>
            <div class="list">
              ${listItemHTML(`mode=${state.previewMode ? "static preview" : "connected"}`)}
              ${failure ? listItemHTML(`reason_code=${failure.reason_code || "-"}`) : ""}
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
    return ov ? `override=${ov} | system=${sys} | user=${user}` : `system=${sys} | user=${user}`;
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

  const renderPostDOM = () => {
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

  const connectBridge = async () => {
    try {
      const conn = await getBridge().Connect();
      renderConnection(conn);
      return conn;
    } catch (err) {
      toast(`Connect failed: ${String(err)}`);
      return null;
    }
  };

  const refreshSnapshot = async () => {
    if (state.previewMode) {
      scheduleRender();
      return;
    }
    try {
      const b = getBridge();
      const sResp = await b.GetStatus();
      if (!sResp || !sResp.ok) throw new Error(bridgeErrorSummary(sResp && sResp.error));
      state.status = sResp.status || null;

      const pResp = await b.GetPeers();
      if (!pResp || !pResp.ok) throw new Error(bridgeErrorSummary(pResp && pResp.error));
      state.peers = pResp.peers && Array.isArray(pResp.peers.peers) ? pResp.peers.peers : [];

      if (typeof b.GetTopology === "function") {
        const topResp = await b.GetTopology();
        if (!topResp || !topResp.ok) throw new Error(bridgeErrorSummary(topResp && topResp.error));
        state.topology = topResp.topology || topologyFromPeers(state.peers);
      } else {
        state.topology = topologyFromPeers(state.peers);
      }

      const tResp = await b.GetTasks();
      if (!tResp || !tResp.ok) throw new Error(bridgeErrorSummary(tResp && tResp.error));
      const tasks = tResp.tasks && Array.isArray(tResp.tasks.tasks) ? tResp.tasks.tasks : [];
      state.tasks = new Map(tasks.map((t) => [canonicalTaskID(t.task_id), t]));
      scheduleRender();
    } catch (err) {
      toast(`Refresh failed: ${String(err)}`);
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
    } else if (kind === "ping") {
      taskObj.stage = "payload exchanged";
      taskObj.facts.push({ message: "path=quic/udp4 rtt_ms=18" });
    } else if (kind === "sh_ls") {
      taskObj.stage = "sessions listed";
      taskObj.facts.push({ message: "sessions=main, maintenance" });
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

  const disconnectShell = () => {
    if (shellState.resizeObs) {
      shellState.resizeObs.disconnect();
      shellState.resizeObs = null;
    }
    if (shellState.ws) {
      try {
        shellState.ws.close(1000, "bye");
      } catch {
        // ignore close race
      }
      shellState.ws = null;
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
    const btn = el("btn-shell-disconnect");
    if (btn) btn.disabled = true;
    const status = el("shell-status");
    if (status) status.textContent = "-";
  };

  const openTerminal = () => {
    const container = el("terminal");
    if (!container) throw new Error("terminal container is missing");
    container.textContent = "";
    if (typeof window.Terminal !== "function") {
      container.textContent = "xterm.js failed to load";
      return null;
    }
    const term = new window.Terminal({
      cursorBlink: true,
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
      fontSize: 13,
      theme: { background: "#0b1014", foreground: "#e8edf2", cursor: "#82d2ff" },
    });
    shellState.term = term;
    term.open(container);
    return term;
  };

  const startPreviewShell = async (peerID, target, session) => {
    const created = await createTask("sh_attach", { peer_id: peerID, target, session });
    upsertTask(attachPeerFact(created, peerID));
    shellState.taskID = created.task_id;
    const term = openTerminal();
    const status = el("shell-status");
    if (status) status.textContent = "preview connected";
    const disconnectBtn = el("btn-shell-disconnect");
    if (disconnectBtn) disconnectBtn.disabled = false;
    if (term) {
      term.writeln("miopunch preview shell");
      term.writeln(`peer=${peerID}`);
      term.writeln(`session=${session}`);
      term.writeln("");
      term.write("$ ");
    }
  };

  const startLiveShell = async (peerID, target, session) => {
    const encoder = new TextEncoder();
    const decoder = new TextDecoder("utf-8");
    const created = await createTask("sh_attach", { peer_id: peerID, target, session });
    upsertTask(attachPeerFact(created, peerID));
    shellState.taskID = created.task_id;
    const status = el("shell-status");
    if (status) status.textContent = `task=${created.task_id}`;
    const term = openTerminal();
    if (!term) throw new Error("xterm.js failed to load");
    term.writeln("Connecting...");

    const bridgeInfo = await getBridge().TerminalBridgeInfo();
    if (!bridgeInfo || !bridgeInfo.ok) throw new Error(bridgeErrorSummary(bridgeInfo && bridgeInfo.error));
    const token = String(bridgeInfo.token || "");
    const baseURL = String(bridgeInfo.base_url || "");
    const subprotocol = String(bridgeInfo.subprotocol || "miopunch.sh.v0");
    if (!baseURL || !token) throw new Error("terminal bridge is not ready");

    const wsURL = `${baseURL}/api/v0/tasks/${encodeURIComponent(created.task_id)}/ws?token=${encodeURIComponent(token)}`;
    let opened = false;
    let retries = 0;
    const maxRetries = 2;

    const connectWS = () => {
      const ws = new WebSocket(wsURL, [subprotocol]);
      ws.binaryType = "arraybuffer";
      shellState.ws = ws;
      ws.onopen = () => {
        opened = true;
        const shellStatus = el("shell-status");
        if (shellStatus) shellStatus.textContent = "connected";
        const btn = el("btn-shell-disconnect");
        if (btn) btn.disabled = false;
        fitAndSendWinSize();
      };
      ws.onmessage = (msg) => {
        if (!shellState.term) return;
        if (typeof msg.data === "string") {
          shellState.term.write(msg.data);
          return;
        }
        shellState.term.write(decoder.decode(new Uint8Array(msg.data)));
      };
      ws.onclose = () => {
        if (shellState.term) shellState.term.writeln("\r\n[disconnected]");
        const shellStatus = el("shell-status");
        if (shellStatus) shellStatus.textContent = "disconnected";
        const btn = el("btn-shell-disconnect");
        if (btn) btn.disabled = true;
        if (!opened && shellState.taskID && retries < maxRetries) {
          retries += 1;
          if (shellStatus) shellStatus.textContent = `reconnecting (${retries}/${maxRetries})...`;
          window.setTimeout(connectWS, 350 * retries);
        }
      };
      ws.onerror = () => {
        const shellStatus = el("shell-status");
        if (shellStatus) shellStatus.textContent = "error";
      };
    };

    connectWS();
    term.onData((data) => {
      const ws = shellState.ws;
      try {
        if (ws && ws.readyState === WebSocket.OPEN) ws.send(encoder.encode(data));
      } catch {
        // ignore closed websocket race
      }
    });

    const container = el("terminal");
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
      refreshSnapshot();
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
      window.runtime.EventsOn("localapi:event", (ev) => {
        try {
          handleGlobalEvent(ev);
        } catch {
          // ignore malformed event payload
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
    if (target.id === "btn-shell-disconnect") {
      event.preventDefault();
      disconnectShell();
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
    const created = await createTask("approve", { code });
    approveState.taskID = upsertTask(created);
    approveState.message = "";
    scheduleRender();
  };

  const applyLocalAPIOverride = async () => {
    if (state.previewMode) return;
    const input = el("localapi-override");
    const value = input ? input.value.trim() : "";
    const conn = await getBridge().SetLocalAPIOverride(value);
    renderConnection(conn);
    await refreshSnapshot();
  };

  const clearLocalAPIOverride = async () => {
    if (state.previewMode) return;
    const conn = await getBridge().ClearLocalAPIOverride();
    renderConnection(conn);
    await refreshSnapshot();
  };

  const submitShell = async () => {
    disconnectShell();
    const peerInput = el("shell-peer-id");
    const targetInput = el("shell-target");
    const sessionInput = el("shell-session");
    const peerID = peerInput ? peerInput.value.trim() : "";
    const target = targetInput ? targetInput.value.trim() : "";
    const session = sessionInput && sessionInput.value.trim() ? sessionInput.value.trim() : "main";
    if (!peerID) {
      toast("Missing peer_id");
      return;
    }
    if (state.previewMode) await startPreviewShell(peerID, target, session);
    else await startLiveShell(peerID, target, session);
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
      connectBridge().then((conn) => {
        if (conn && conn.connected) refreshSnapshot();
      });
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
