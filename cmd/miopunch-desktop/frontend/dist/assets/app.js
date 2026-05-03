(() => {
  const el = (id) => document.getElementById(id);

  const toastEl = el("toast");
  const toast = (message) => {
    if (!toastEl) return;
    toastEl.textContent = String(message || "");
    toastEl.classList.add("is-visible");
    window.clearTimeout(toastEl._hideTimer);
    toastEl._hideTimer = window.setTimeout(() => {
      toastEl.classList.remove("is-visible");
    }, 2200);
  };

  const state = {
    status: null,
    peers: [],
    tasks: new Map(),
  };

  let renderQueued = false;
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

  const listItem = (text) => {
    const div = document.createElement("div");
    div.className = "md3-list__item";
    div.textContent = String(text || "");
    return div;
  };

  const setLinkEnabled = (a, enabled) => {
    if (!a) return;
    if (enabled) a.classList.remove("is-disabled");
    else a.classList.add("is-disabled");
  };

  const copyToClipboard = async (text) => {
    try {
      await navigator.clipboard.writeText(text);
      toast("Copied");
      return;
    } catch {
      // Fall back to selection-based copy.
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

  const canonicalTaskID = (raw) => String(raw || "").trim();

  const mergeTask = (current, incoming) => {
    if (!current) return incoming || null;
    if (!incoming) return current;

    const merged = { ...current, ...incoming };

    const currentFacts = Array.isArray(current.facts) ? current.facts : [];
    const incomingFacts = Array.isArray(incoming.facts) ? incoming.facts : [];
    merged.facts = incomingFacts.length >= currentFacts.length ? incomingFacts : currentFacts;

    const currentSuggestions = Array.isArray(current.suggestions) ? current.suggestions : [];
    const incomingSuggestions = Array.isArray(incoming.suggestions) ? incoming.suggestions : [];
    merged.suggestions =
      incomingSuggestions.length >= currentSuggestions.length ? incomingSuggestions : currentSuggestions;

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
    return taskID;
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
      return;
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

    let t = state.tasks.get(taskID);
    if (!t) {
      t = { task_id: taskID, kind: "", status: "running", stage: "", facts: [], suggestions: [] };
      state.tasks.set(taskID, t);
    }

    applyEventToTask(t, ev);
    scheduleRender();
  };

  const getBridge = () => {
    const b = window.go && window.go.main && window.go.main.App;
    if (!b) throw new Error("desktop bridge is not ready");
    return b;
  };

  const bridgeErrorSummary = (err) => {
    if (!err) return "unknown error";
    const reason = err.reason_code ? String(err.reason_code) : "";
    const msg = err.message ? String(err.message) : "";
    if (reason && msg) return `${reason}: ${msg}`;
    return reason || msg || "unknown error";
  };

  let lastConn = null;
  const renderConnection = (conn) => {
    lastConn = conn || null;

    const subtitleOrigin = el("subtitle-origin");
    if (subtitleOrigin) {
      if (conn && conn.connected && conn.selected && conn.addr) {
        subtitleOrigin.textContent = `localapi=${String(conn.selected)} ${String(conn.addr)}`;
      } else if (conn && conn.failure) {
        subtitleOrigin.textContent = `localapi=disconnected (${bridgeErrorSummary(conn.failure)})`;
      } else {
        subtitleOrigin.textContent = "localapi=disconnected";
      }
    }

    const connSelected = el("conn-selected");
    if (connSelected) {
      if (conn && conn.connected && conn.selected && conn.addr) {
        connSelected.textContent = `LocalAPI: ${String(conn.selected)} (${String(conn.addr)})`;
      } else {
        connSelected.textContent = "LocalAPI: not connected";
      }
    }

    const known = el("localapi-known");
    if (known) {
      const sys = conn && conn.system_addr ? String(conn.system_addr) : "(unknown)";
      const user = conn && conn.user_addr ? String(conn.user_addr) : "(unknown)";
      const ov = conn && conn.override_addr ? String(conn.override_addr) : "";
      known.textContent = ov ? `override=${ov} · system=${sys} · user=${user}` : `system=${sys} · user=${user}`;
    }

    const overrideInput = el("localapi-override");
    if (overrideInput && conn && typeof conn.override_addr === "string") {
      overrideInput.value = conn.override_addr;
    }

    const errWrap = el("conn-error");
    const sugEl = el("conn-suggestions");
    const factsEl = el("conn-facts");
    if (!errWrap || !sugEl || !factsEl) return;

    if (!conn || !conn.failure) {
      errWrap.style.display = "none";
      sugEl.textContent = "";
      factsEl.textContent = "";
      return;
    }

    errWrap.style.display = "";
    sugEl.textContent = "";
    factsEl.textContent = "";

    const sug = Array.isArray(conn.failure.suggestions) ? conn.failure.suggestions : [];
    if (sug.length === 0) sugEl.appendChild(listItem("(none)"));
    for (const s of sug) sugEl.appendChild(listItem(s.message || ""));

    factsEl.appendChild(listItem(`stage=${String(conn.failure.stage || "—")}`));
    factsEl.appendChild(listItem(`reason_code=${String(conn.failure.reason_code || "—")}`));
    factsEl.appendChild(listItem(`exit_code=${typeof conn.failure.exit_code !== "undefined" ? String(conn.failure.exit_code) : "—"}`));
    const facts = Array.isArray(conn.failure.facts) ? conn.failure.facts : [];
    for (const f of facts) factsEl.appendChild(listItem(f.message || ""));
  };

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

  const createTask = async (kind, args) => {
    const resp = await getBridge().CreateTask(kind, args || null);
    if (!resp || !resp.ok) throw new Error(bridgeErrorSummary(resp && resp.error));
    return resp.task;
  };

  const getTask = async (taskID) => {
    const resp = await getBridge().GetTask(taskID);
    if (!resp || !resp.ok) throw new Error(bridgeErrorSummary(resp && resp.error));
    return resp.task;
  };

  const refreshSnapshot = async () => {
    try {
      const sResp = await getBridge().GetStatus();
      if (!sResp || !sResp.ok) throw new Error(bridgeErrorSummary(sResp && sResp.error));
      state.status = sResp.status || null;

      const pResp = await getBridge().GetPeers();
      if (!pResp || !pResp.ok) throw new Error(bridgeErrorSummary(pResp && pResp.error));
      state.peers = pResp.peers && Array.isArray(pResp.peers.peers) ? pResp.peers.peers : [];

      const tResp = await getBridge().GetTasks();
      if (!tResp || !tResp.ok) throw new Error(bridgeErrorSummary(tResp && tResp.error));
      const tasks = tResp.tasks && Array.isArray(tResp.tasks.tasks) ? tResp.tasks.tasks : [];
      state.tasks = new Map(tasks.map((t) => [canonicalTaskID(t.task_id), t]));

      const s = state.status || {};
      const subtitleVersion = el("subtitle-version");
      if (subtitleVersion) subtitleVersion.textContent = `version ${s.version || "dev"}`;

      scheduleRender();
    } catch (err) {
      toast(`Refresh failed: ${String(err)}`);
    }
  };

  const exportReport = async (taskID, onSavedPath) => {
    try {
      const resp = await getBridge().ExportTaskReport(taskID);
      if (!resp || !resp.ok) throw new Error(bridgeErrorSummary(resp && resp.error));
      if (resp.cancelled) return { cancelled: true };
      if (resp.path) {
        if (typeof onSavedPath === "function") onSavedPath(String(resp.path));
        toast(`Saved: ${String(resp.path)}`);
      } else {
        toast("Saved");
      }
      return resp;
    } catch (err) {
      toast(`Export failed: ${String(err)}`);
      return null;
    }
  };

  const renderStatus = () => {
    const s = state.status || {};
    const versionEl = el("status-version");
    if (versionEl) versionEl.textContent = s.version || "—";
    const uptimeEl = el("status-uptime");
    if (uptimeEl) uptimeEl.textContent = s.uptime_ms ? fmtUptime(s.uptime_ms) : "—";

    const peers = state.peers || [];
    const peersCount = el("peers-count");
    if (peersCount) peersCount.textContent = String(peers.length);
    const peersList = el("peers-list");
    if (peersList) {
      peersList.textContent = "";
      for (const p of peers) peersList.appendChild(listItem(p.peer_id));
      if (peers.length === 0) peersList.appendChild(listItem("(none)"));
    }

    const peerDatalist = el("peer-id-list");
    if (peerDatalist) {
      peerDatalist.textContent = "";
      for (const p of peers) {
        const opt = document.createElement("option");
        opt.value = p.peer_id;
        peerDatalist.appendChild(opt);
      }
    }

    const tasksCount = el("tasks-count");
    if (tasksCount) tasksCount.textContent = String(state.tasks.size);
    const tasksList = el("tasks-list");
    if (!tasksList) return;
    tasksList.textContent = "";

    const tasks = [...state.tasks.values()].sort((a, b) =>
      String(b.created_at || "").localeCompare(String(a.created_at || ""))
    );
    if (tasks.length === 0) {
      tasksList.appendChild(listItem("(no tasks yet)"));
      return;
    }

    for (const t of tasks) {
      const card = document.createElement("div");
      card.className = "md3-task";

      const hdr = document.createElement("div");
      hdr.className = "md3-task__hdr";
      const left = document.createElement("div");
      const kindDiv = document.createElement("div");
      kindDiv.className = "md3-task__kind";
      kindDiv.textContent = String(t.kind || "(unknown)");
      const idDiv = document.createElement("div");
      idDiv.className = "md3-task__id";
      idDiv.textContent = String(t.task_id || "");
      left.appendChild(kindDiv);
      left.appendChild(idDiv);
      const chip = document.createElement("span");
      chip.className = "md3-chip";
      chip.textContent = String(t.status || "—");
      hdr.appendChild(left);
      hdr.appendChild(chip);
      card.appendChild(hdr);

      const meta = document.createElement("div");
      meta.className = "md3-task__meta";
      const stageLine = document.createElement("div");
      stageLine.textContent = `stage=${String(t.stage || "—")}`;
      meta.appendChild(stageLine);
      if (t.reason_code) {
        const reasonLine = document.createElement("div");
        reasonLine.textContent = `reason_code=${String(t.reason_code)}`;
        meta.appendChild(reasonLine);
      }
      if (typeof t.exit_code !== "undefined" && t.exit_code !== null && t.exit_code !== 0) {
        const exitLine = document.createElement("div");
        exitLine.textContent = `exit_code=${String(t.exit_code)}`;
        meta.appendChild(exitLine);
      }
      card.appendChild(meta);

      const facts = Array.isArray(t.facts) ? t.facts : [];
      const suggestions = Array.isArray(t.suggestions) ? t.suggestions : [];
      const tail = (arr, n) => arr.slice(Math.max(0, arr.length - n));

      if (facts.length > 0) {
        const factsDiv = document.createElement("div");
        factsDiv.className = "md3-list md3-mt-sm";
        for (const f of tail(facts, 3)) factsDiv.appendChild(listItem(f.message || ""));
        card.appendChild(factsDiv);
      }

      if (suggestions.length > 0) {
        const sugDiv = document.createElement("div");
        sugDiv.className = "md3-list md3-mt-sm";
        for (const s of tail(suggestions, 2)) sugDiv.appendChild(listItem(s.message || ""));
        card.appendChild(sugDiv);
      }

      if (t.report_ready) {
        const btn = document.createElement("button");
        btn.className = "md3-btn md3-btn--tonal md3-mt-sm";
        btn.textContent = "Export report";
        btn.addEventListener("click", () => exportReport(String(t.task_id || "")));
        card.appendChild(btn);
      }

      tasksList.appendChild(card);
    }
  };

  const renderAll = () => {
    renderStatus();
    renderInvite();
    renderJoin();
  };

  const setActiveTab = (name) => {
    for (const btn of document.querySelectorAll(".md3-tab")) {
      const active = btn.dataset.tab === name;
      btn.classList.toggle("is-active", active);
      btn.setAttribute("aria-selected", active ? "true" : "false");
    }
    for (const panel of document.querySelectorAll(".md3-tabpanel")) {
      panel.classList.toggle("is-active", panel.id === `tab-${name}`);
    }
    localStorage.setItem("miopunch_http_panel_tab", name);
  };

  const wireTabs = () => {
    for (const btn of document.querySelectorAll(".md3-tab")) {
      btn.addEventListener("click", () => setActiveTab(btn.dataset.tab));
    }
    const last = localStorage.getItem("miopunch_http_panel_tab");
    if (last) setActiveTab(last);
  };

  // Invite tab
  const inviteState = { taskID: "", busy: false, message: "" };
  const renderInvite = () => {
    const task = inviteState.taskID ? state.tasks.get(inviteState.taskID) : null;
    el("invite-task-id").textContent = task && task.task_id ? task.task_id : "—";
    el("invite-stage").textContent = task && task.stage ? task.stage : "—";

    const code = findInviteCode(task);
    const codeEl = el("invite-code");
    const copyBtn = el("btn-copy-invite");
    const createBtn = el("btn-invite");
    if (codeEl) codeEl.value = code || "";
    if (copyBtn) copyBtn.disabled = !code;
    if (createBtn) createBtn.disabled = inviteState.busy || !(lastConn && lastConn.connected);

    const qrEl = el("invite-qr");
    if (qrEl) {
      qrEl.textContent = "";
      if (code && typeof window.qrcode === "function") {
        try {
          const qr = window.qrcode(0, "M");
          qr.addData(code);
          qr.make();
          qrEl.innerHTML = qr.createSvgTag({ scalable: true, cellSize: 4, margin: 4, alt: "invite code" });
        } catch {
          qrEl.textContent = "(QR failed)";
        }
      }
    }

    const hint = el("invite-hint");
    if (!hint) return;
    hint.textContent = "";
    if (inviteState.message) {
      hint.textContent = inviteState.message;
      return;
    }
    if (!(lastConn && lastConn.connected)) {
      hint.textContent = "LocalAPI is not connected.";
      return;
    }
    if (task && Array.isArray(task.suggestions) && task.suggestions.length > 0) {
      hint.textContent = task.suggestions.map((s) => s.message).filter(Boolean).join(" · ");
      return;
    }
    if (!code && task && task.status === "done" && task.reason_code && task.reason_code !== "OK") {
      const facts = Array.isArray(task.facts) ? task.facts.map((f) => f.message).filter(Boolean) : [];
      hint.textContent = [`Failed: ${task.reason_code}`, ...facts].join(" · ");
    }
  };

  const findInviteCode = (taskObj) => {
    if (!taskObj || !Array.isArray(taskObj.facts)) return "";
    for (const f of taskObj.facts) {
      const msg = String(f.message || "");
      const prefix = "invite_code=";
      if (msg.startsWith(prefix)) return msg.slice(prefix.length).trim();
      const idx = msg.indexOf(prefix);
      if (idx >= 0) return msg.slice(idx + prefix.length).trim();
    }
    return "";
  };

  const wireInvite = () => {
    const btn = el("btn-invite");
    const copyBtn = el("btn-copy-invite");
    if (btn) {
      btn.addEventListener("click", async () => {
        inviteState.busy = true;
        inviteState.message = "Creating invite...";
        renderInvite();
        try {
          inviteState.taskID = "";
          renderInvite();

          const created = await createTask("invite", null);
          const taskID = upsertTask(created);
          if (!taskID) throw new Error("Create invite did not return task_id");

          inviteState.taskID = taskID;
          scheduleRender();

          const latest = await getTask(taskID);
          upsertTask(latest);
          inviteState.message = "";
          scheduleRender();
        } catch (err) {
          inviteState.message = `Create failed: ${String(err)}`;
          toast(inviteState.message);
        } finally {
          inviteState.busy = false;
          renderInvite();
        }
      });
    }
    if (copyBtn) {
      copyBtn.addEventListener("click", async () => {
        const code = el("invite-code").value || "";
        if (!code) return;
        await copyToClipboard(code);
      });
    }
  };

  // Join tab
  const joinState = { taskID: "", lastExportPath: "" };
  const renderJoin = () => {
    const task = joinState.taskID ? state.tasks.get(joinState.taskID) : null;
    el("join-task-id").textContent = task && task.task_id ? task.task_id : "—";
    el("join-stage").textContent = task && task.stage ? task.stage : "—";
    el("join-reason").textContent = task && task.reason_code ? task.reason_code : "—";
    el("join-exit").textContent = typeof task?.exit_code !== "undefined" ? String(task.exit_code) : "—";

    const factsEl = el("join-facts");
    if (factsEl) {
      factsEl.textContent = "";
      const facts = task && Array.isArray(task.facts) ? task.facts : [];
      if (facts.length === 0) factsEl.appendChild(listItem("(none)"));
      for (const f of facts) factsEl.appendChild(listItem(f.message || ""));
    }
    const sugEl = el("join-suggestions");
    if (sugEl) {
      sugEl.textContent = "";
      const sug = task && Array.isArray(task.suggestions) ? task.suggestions : [];
      if (sug.length === 0) sugEl.appendChild(listItem("(none)"));
      for (const s of sug) sugEl.appendChild(listItem(s.message || ""));
    }

    const exportBtn = el("join-report-export");
    if (exportBtn) exportBtn.disabled = !(task && task.task_id && task.report_ready);

    const pathEl = el("join-report-path");
    if (pathEl) pathEl.textContent = joinState.lastExportPath || "";
  };

  const wireJoin = () => {
    const form = el("join-form");
    const exportBtn = el("join-report-export");
    if (exportBtn) {
      exportBtn.addEventListener("click", async (e) => {
        e.preventDefault();
        if (!joinState.taskID) return;
        await exportReport(joinState.taskID, (p) => {
          joinState.lastExportPath = p;
          renderJoin();
        });
      });
    }
    if (!form) return;
    form.addEventListener("submit", async (e) => {
      e.preventDefault();
      const code = el("join-code").value.trim();
      if (!code) {
        toast("Missing invite code");
        return;
      }

      try {
        const created = await createTask("join", { code });
        joinState.taskID = upsertTask(created);
        joinState.lastExportPath = "";
        scheduleRender();
      } catch (err) {
        toast(String(err));
      }
    });
  };

  // Shell tab
  const shellState = { ws: null, term: null, resizeObs: null, taskID: "" };
  const encoder = new TextEncoder();
  const decoder = new TextDecoder("utf-8");

  const measureCell = (fontFamily, fontSizePx) => {
    const span = document.createElement("span");
    span.textContent = "W";
    span.style.fontFamily = fontFamily;
    span.style.fontSize = `${fontSizePx}px`;
    span.style.position = "absolute";
    span.style.visibility = "hidden";
    span.style.top = "-9999px";
    document.body.appendChild(span);
    const r = span.getBoundingClientRect();
    document.body.removeChild(span);
    return { w: Math.max(6, r.width), h: Math.max(10, r.height) };
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
      // ignore
    }

    try {
      ws.send(JSON.stringify({ op: "winsize", winsize: { cols, rows } }));
    } catch {
      // ignore
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
      } catch {}
      shellState.ws = null;
    }
    if (shellState.term) {
      try {
        shellState.term.dispose();
      } catch {}
      shellState.term = null;
    }
    shellState.taskID = "";
    el("btn-shell-disconnect").disabled = true;
    el("shell-status").textContent = "—";
  };

  const wireShell = () => {
    const form = el("shell-form");
    const btnDisconnect = el("btn-shell-disconnect");
    if (btnDisconnect) btnDisconnect.addEventListener("click", (e) => {
      e.preventDefault();
      disconnectShell();
    });

    if (!form) return;
    form.addEventListener("submit", async (e) => {
      e.preventDefault();

      disconnectShell();

      const peerID = el("shell-peer-id").value.trim();
      const target = el("shell-target").value.trim();
      const session = el("shell-session").value.trim() || "main";
      if (!peerID) {
        toast("Missing peer_id");
        return;
      }

      if (typeof window.Terminal !== "function") {
        toast("xterm.js failed to load");
        return;
      }

      try {
        const created = await createTask("sh_attach", { peer_id: peerID, target, session });
        shellState.taskID = created.task_id;
        el("shell-status").textContent = `task=${created.task_id}`;

        const term = new window.Terminal({
          cursorBlink: true,
          fontFamily: "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
          fontSize: 13,
          theme: { background: "#0b0b0f", foreground: "#e8e6ea" },
        });
        shellState.term = term;

        const container = el("terminal");
        container.textContent = "";
        term.open(container);
        term.writeln("Connecting…");

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
            el("shell-status").textContent = "connected";
            el("btn-shell-disconnect").disabled = false;
            fitAndSendWinSize();
          };

          ws.onmessage = (msg) => {
            if (!shellState.term) return;
            if (typeof msg.data === "string") {
              shellState.term.write(msg.data);
              return;
            }
            const buf = new Uint8Array(msg.data);
            shellState.term.write(decoder.decode(buf));
          };

          ws.onclose = () => {
            if (shellState.term) shellState.term.writeln("\r\n[disconnected]");
            el("shell-status").textContent = "disconnected";
            el("btn-shell-disconnect").disabled = true;

            if (!opened && shellState.taskID && retries < maxRetries) {
              retries += 1;
              el("shell-status").textContent = `reconnecting (${retries}/${maxRetries})…`;
              window.setTimeout(connectWS, 350 * retries);
            }
          };

          ws.onerror = () => {
            el("shell-status").textContent = "error";
          };
        };

        connectWS();

        term.onData((data) => {
          const ws = shellState.ws;
          try {
            if (ws && ws.readyState === WebSocket.OPEN) ws.send(encoder.encode(data));
          } catch {}
        });

        const ro = new ResizeObserver(() => {
          window.clearTimeout(shellState._fitTimer);
          shellState._fitTimer = window.setTimeout(fitAndSendWinSize, 80);
        });
        ro.observe(container);
        shellState.resizeObs = ro;
      } catch (err) {
        toast(String(err));
      }
    });
  };

  const wireAdvancedLocalAPI = () => {
    const input = el("localapi-override");
    const applyBtn = el("btn-localapi-apply");
    const clearBtn = el("btn-localapi-clear");

    if (applyBtn) {
      applyBtn.addEventListener("click", async (e) => {
        e.preventDefault();
        const value = input ? input.value.trim() : "";
        try {
          const conn = await getBridge().SetLocalAPIOverride(value);
          renderConnection(conn);
          await refreshSnapshot();
        } catch (err) {
          toast(`Apply override failed: ${String(err)}`);
        }
      });
    }

    if (clearBtn) {
      clearBtn.addEventListener("click", async (e) => {
        e.preventDefault();
        if (input) input.value = "";
        try {
          const conn = await getBridge().ClearLocalAPIOverride();
          renderConnection(conn);
          await refreshSnapshot();
        } catch (err) {
          toast(`Clear override failed: ${String(err)}`);
        }
      });
    }
  };

  const wireRefresh = () => {
    const btn = el("btn-refresh");
    if (!btn) return;
    btn.addEventListener("click", (e) => {
      e.preventDefault();
      refreshSnapshot();
    });
  };

  const startRuntimeEvents = () => {
    if (!window.runtime || typeof window.runtime.EventsOn !== "function") {
      toast("runtime events unavailable");
      return;
    }

    window.runtime.EventsOn("desktop:startup_error", (payload) => {
      try {
        const component = payload && payload.component ? String(payload.component) : "startup";
        const err = payload && payload.error ? payload.error : null;
        toast(`${component}: ${bridgeErrorSummary(err)}`);
      } catch {
        // ignore
      }
    });

    window.runtime.EventsOn("localapi:event", (ev) => {
      try {
        handleGlobalEvent(ev);
      } catch {
        // ignore
      }
    });

    window.runtime.EventsOn("localapi:connection", (conn) => {
      try {
        renderConnection(conn);
      } catch {
        // ignore
      }
    });
  };

  // Bootstrap
  wireTabs();
  wireAdvancedLocalAPI();
  wireRefresh();
  wireInvite();
  wireJoin();
  wireShell();

  startRuntimeEvents();

  connectBridge().then(refreshSnapshot);
})();
