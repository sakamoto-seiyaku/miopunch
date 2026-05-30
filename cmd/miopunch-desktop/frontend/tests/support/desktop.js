const base = require("@playwright/test");

const { expect } = base;

const RUNTIME_STAGES = ["Network", "Enroll", "Discover", "Punch", "SecureSession", "Shell"];
const PEERS = {
  self: "peer-self-alpha-0001",
  remote: "peer-remote-bravo-0002",
};
const inviteCode = "miopunch1inviteconsole00000000000000000000000000000000000000";

const clone = (value) => JSON.parse(JSON.stringify(value));

function mergeDeep(baseValue, patchValue) {
  if (Array.isArray(patchValue)) return clone(patchValue);
  if (!patchValue || typeof patchValue !== "object") return patchValue;

  const baseObject = baseValue && typeof baseValue === "object" && !Array.isArray(baseValue) ? baseValue : {};
  const out = { ...baseObject };
  for (const [key, value] of Object.entries(patchValue)) {
    if (Array.isArray(value)) {
      out[key] = clone(value);
      continue;
    }
    if (value && typeof value === "object") {
      out[key] = mergeDeep(baseObject[key], value);
      continue;
    }
    out[key] = value;
  }
  return out;
}

function defaultSnapshot(stage = "Network") {
  return {
    stage,
    reason_code: "OK",
    summary: {
      text: `runtime stage: ${stage}`,
    },
    evidence: {
      facts: [{ message: `stage=${stage}` }],
      suggestions: [{ message: "inspect the runtime console" }],
    },
    discover_view: {
      network_id: "net_console_lab",
      self_peer_id: PEERS.self,
      peers: [{
        peer_id: PEERS.remote,
        online_state: "online",
        device_name: "Remote Bravo",
        platform: "linux",
        app_ver: "0.1.0",
      }],
    },
    peer_sessions: [],
    shell_sessions: [],
  };
}

function snapshotFor(stage = "Network", patch = {}) {
  const safeStage = RUNTIME_STAGES.includes(stage) ? stage : "Network";
  return mergeDeep(defaultSnapshot(safeStage), patch);
}

function defaultConnection() {
  return {
    connected: true,
    selected: "user",
    addr: "unix:/tmp/miopunch-localapi.sock",
    user_addr: "unix:/tmp/miopunch-localapi.sock",
    system_addr: "",
    override_addr: "",
    bootstrap_state: "ready",
    desktop_managed: false,
    diagnostics: [{ message: "selected_endpoint=user" }],
  };
}

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

async function installFakeBridge(page, options = {}) {
  const init = {
    snapshot: clone(options.snapshot || snapshotFor("Network")),
    connection: clone(mergeDeep(defaultConnection(), options.connection || {})),
    runtimeStartFailures: Number(options.runtimeStartFailures || 0),
    diagnosticsPath: String(options.diagnosticsPath || "/tmp/miopunch-diagnostics.zip"),
    inviteCode,
    inviteDataMode: String(options.inviteDataMode || "object"),
    terminalBridgeInfo: clone(options.terminalBridgeInfo || {
      ok: true,
      base_url: "ws://127.0.0.1:4173",
      token: "ui-test-token",
      subprotocol: "miopunch.sh.v0",
    }),
    websocketMessages: clone(options.websocketMessages || {
      "shell-session-01": ["welcome to miopunch shell\n"],
    }),
  };

  await page.addInitScript((data) => {
    const clone = (value) => JSON.parse(JSON.stringify(value));

    const mergeDeep = (baseValue, patchValue) => {
      if (Array.isArray(patchValue)) return clone(patchValue);
      if (!patchValue || typeof patchValue !== "object") return patchValue;

      const baseObject = baseValue && typeof baseValue === "object" && !Array.isArray(baseValue) ? baseValue : {};
      const out = { ...baseObject };
      for (const [key, value] of Object.entries(patchValue)) {
        if (Array.isArray(value)) {
          out[key] = clone(value);
          continue;
        }
        if (value && typeof value === "object") {
          out[key] = mergeDeep(baseObject[key], value);
          continue;
        }
        out[key] = value;
      }
      return out;
    };

    const calls = [];
    const eventHandlers = new Map();
    const sockets = new Set();
    let connection = clone(data.connection);
    let snapshot = clone(data.snapshot);
    let runtimeStartCount = 0;
    let shellSessionSeq = 0;

    const record = (entry) => {
      calls.push(clone(entry));
    };
    const emit = (name, payload) => {
      const handlers = eventHandlers.get(name) || [];
      for (const handler of handlers) {
        handler(clone(payload));
      }
    };
    const buildInviteData = () => {
      const payload = { invite_code: data.inviteCode };
      if (data.inviteDataMode === "string") return JSON.stringify(payload);
      if (data.inviteDataMode === "bytes") {
        return Array.from(new TextEncoder().encode(JSON.stringify(payload)));
      }
      return payload;
    };
    const actionResult = (extra = {}) => ({
      stage: snapshot.stage,
      reason_code: snapshot.reason_code,
      exit_code: 0,
      summary: clone(snapshot.summary),
      evidence: clone(snapshot.evidence),
      snapshot: clone(snapshot),
      ...extra,
    });
    const upsertPeerSession = (peerID, patch) => {
      const current = Array.isArray(snapshot.peer_sessions) ? snapshot.peer_sessions.slice() : [];
      const index = current.findIndex((item) => String(item && item.peer_id || "") === peerID);
      const next = {
        peer_id: peerID,
        healthy: true,
        path_family: "udp4",
        protocol: "kcp",
        ping_gate_satisfied: false,
        ...patch,
      };
      if (index >= 0) current[index] = { ...current[index], ...next };
      else current.push(next);
      snapshot.peer_sessions = current;
    };
    const upsertShellSession = (sessionID, peerID, patch) => {
      const current = Array.isArray(snapshot.shell_sessions) ? snapshot.shell_sessions.slice() : [];
      const index = current.findIndex((item) => String(item && item.id || "") === sessionID);
      const now = Date.now();
      const next = {
        id: sessionID,
        peer_id: peerID,
        status: "attached",
        created_at_unix_ms: now,
        attached_unix_ms: now,
        ...patch,
      };
      if (index >= 0) current[index] = { ...current[index], ...next };
      else current.push(next);
      snapshot.shell_sessions = current;
    };
    const firstPeerID = () => {
      const peers = snapshot.discover_view && Array.isArray(snapshot.discover_view.peers)
        ? snapshot.discover_view.peers
        : [];
      return String(peers[0] && peers[0].peer_id || "peer-remote-bravo-0002");
    };

    window.__miopunchCalls = calls;
    window.__miopunchShellLog = [];
    window.__miopunchClipboard = "";
    window.__miopunchEmit = (name, payload) => emit(name, payload);
    window.__miopunchSetRuntimeSnapshot = (nextSnapshot) => {
      snapshot = clone(nextSnapshot);
      return clone(snapshot);
    };
    window.__miopunchMergeRuntimeSnapshot = (nextSnapshot) => {
      snapshot = mergeDeep(snapshot, nextSnapshot || {});
      return clone(snapshot);
    };
    window.__miopunchSetConnection = (nextConnection) => {
      connection = mergeDeep(connection, nextConnection || {});
      emit("localapi:connection", connection);
      return clone(connection);
    };
    window.__miopunchPushShellData = (sessionID, text) => {
      for (const socket of sockets) {
        if (socket.__sessionID !== sessionID) continue;
        socket.__emitMessage(String(text || ""));
      }
    };

    window.runtime = {
      EventsOn: (name, handler) => {
        const handlers = eventHandlers.get(name) || [];
        handlers.push(handler);
        eventHandlers.set(name, handlers);
      },
    };

    Object.defineProperty(window.navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (text) => {
          window.__miopunchClipboard = String(text || "");
        },
      },
    });

    class FakeWebSocket {
      static CONNECTING = 0;
      static OPEN = 1;
      static CLOSING = 2;
      static CLOSED = 3;

      constructor(url, protocol) {
        this.url = String(url || "");
        this.protocol = protocol;
        this.readyState = FakeWebSocket.CONNECTING;
        this.binaryType = "blob";
        this.__sessionID = "";

        const match = this.url.match(/\/api\/v1\/shell\/([^/?]+)\/ws\?token=([^&]+)/);
        if (match) this.__sessionID = decodeURIComponent(match[1]);
        sockets.add(this);

        window.setTimeout(() => {
          if (this.readyState !== FakeWebSocket.CONNECTING) return;
          this.readyState = FakeWebSocket.OPEN;
          window.__miopunchShellLog.push(clone({
            type: "open",
            url: this.url,
            protocol: this.protocol,
            sessionID: this.__sessionID,
          }));
          if (typeof this.onopen === "function") this.onopen();
          const lines = data.websocketMessages[this.__sessionID] || [];
          for (const line of lines) this.__emitMessage(line);
        }, 0);
      }

      send(payload) {
        window.__miopunchShellLog.push(clone({
          type: "send",
          url: this.url,
          sessionID: this.__sessionID,
          data: String(payload || ""),
        }));
      }

      close(code = 1000, reason = "") {
        if (this.readyState === FakeWebSocket.CLOSED) return;
        this.readyState = FakeWebSocket.CLOSED;
        sockets.delete(this);
        window.__miopunchShellLog.push(clone({
          type: "close",
          url: this.url,
          sessionID: this.__sessionID,
          code,
          reason: String(reason || ""),
        }));
        if (typeof this.onclose === "function") this.onclose({ code, reason });
      }

      __emitMessage(text) {
        if (this.readyState !== FakeWebSocket.OPEN) return;
        window.__miopunchShellLog.push(clone({
          type: "message",
          url: this.url,
          sessionID: this.__sessionID,
          data: String(text || ""),
        }));
        if (typeof this.onmessage === "function") this.onmessage({ data: String(text || "") });
      }
    }

    window.WebSocket = FakeWebSocket;

    window.go = {
      main: {
        App: {
          DesktopRuntimeStart: async () => {
            record({ method: "DesktopRuntimeStart" });
            runtimeStartCount += 1;
            if (runtimeStartCount <= data.runtimeStartFailures) {
              return {
                ok: false,
                connection: clone(connection),
                error: {
                  stage: "desktop",
                  reason_code: "UNAVAILABLE",
                  exit_code: 69,
                  message: "fake runtime start failure",
                },
              };
            }
            return {
              ok: true,
              connection: clone(connection),
              state: clone(snapshot),
            };
          },
          DesktopRuntimeResync: async () => {
            record({ method: "DesktopRuntimeResync" });
            return {
              ok: true,
              connection: clone(connection),
              state: clone(snapshot),
            };
          },
          RuntimeAction: async (action, args = {}) => {
            record({ method: "RuntimeAction", action, args: clone(args) });

            switch (String(action || "")) {
              case "init-network":
                snapshot = mergeDeep(snapshot, {
                  stage: "Enroll",
                  summary: { text: "network bootstrapped" },
                  evidence: {
                    facts: [{ message: "network_id=net_console_lab" }],
                    suggestions: [{ message: "open Admin to create or use an invite" }],
                  },
                  discover_view: { network_id: "net_console_lab" },
                });
                return { ok: true, result: actionResult() };
              case "invite":
                return {
                  ok: true,
                  result: actionResult({
                    data: buildInviteData(),
                    evidence: {
                      facts: [{ message: `invite_code=${data.inviteCode}` }],
                      suggestions: [{ message: "share the invite code with the joiner" }],
                    },
                  }),
                };
              case "approve":
                snapshot = mergeDeep(snapshot, {
                  stage: "Discover",
                  summary: { text: "approval completed" },
                  evidence: {
                    facts: [{ message: `approved_code=${String(args.code || "")}` }],
                    suggestions: [{ message: "return to Network and wait for the peer projection" }],
                  },
                });
                return { ok: true, result: actionResult() };
              case "join":
                snapshot = mergeDeep(snapshot, {
                  stage: "Discover",
                  summary: { text: "join completed" },
                  evidence: {
                    facts: [{ message: "join_status=complete" }],
                    suggestions: [{ message: "return to Network and refresh peer presence" }],
                  },
                });
                return { ok: true, result: actionResult() };
              case "ping": {
                const peerID = String(args.peer_id || firstPeerID());
                snapshot = mergeDeep(snapshot, {
                  stage: "SecureSession",
                  summary: { text: "identity-bound ping succeeded" },
                  evidence: {
                    facts: [
                      { message: `ping_peer=${peerID}` },
                      { message: "ping_gate=satisfied" },
                    ],
                    suggestions: [{ message: "open Shell and attach the console" }],
                  },
                });
                upsertPeerSession(peerID, {
                  ping_gate_satisfied: true,
                  shell_ready_unix_ms: Date.now(),
                });
                return { ok: true, result: actionResult() };
              }
              case "sh_ls":
                if (args && args.target) {
                  return {
                    ok: true,
                    result: actionResult({
                      data: { sessions: ["main", "ops"] },
                    }),
                  };
                }
                return {
                  ok: true,
                  result: actionResult({
                    data: { targets: ["local", "logs"] },
                  }),
                };
              case "sh": {
                const peerID = String(args.peer_id || firstPeerID());
                shellSessionSeq += 1;
                const shellSessionID = `shell-session-${String(shellSessionSeq).padStart(2, "0")}`;
                snapshot = mergeDeep(snapshot, {
                  stage: "Shell",
                  summary: { text: "shell session attached" },
                  evidence: {
                    facts: [{ message: `shell_session_id=${shellSessionID}` }],
                    suggestions: [{ message: "use the remote console" }],
                  },
                });
                upsertShellSession(shellSessionID, peerID, {
                  target: String(args.target || "local"),
                  session: String(args.session || "main"),
                });
                return {
                  ok: true,
                  result: actionResult({
                    shell_session_id: shellSessionID,
                  }),
                };
              }
              default:
                return {
                  ok: false,
                  error: {
                    stage: "desktop",
                    reason_code: "BAD_REQUEST",
                    exit_code: 64,
                    message: `unsupported fake action: ${String(action || "")}`,
                  },
                };
            }
          },
          TerminalBridgeInfo: async () => {
            record({ method: "TerminalBridgeInfo" });
            return clone(data.terminalBridgeInfo);
          },
          SetLocalAPIOverride: async (addr) => {
            record({ method: "SetLocalAPIOverride", addr });
            connection = mergeDeep(connection, {
              connected: true,
              selected: "override",
              addr,
              override_addr: addr,
            });
            return clone(connection);
          },
          ClearLocalAPIOverride: async () => {
            record({ method: "ClearLocalAPIOverride" });
            connection = mergeDeep(connection, {
              connected: true,
              selected: "user",
              addr: connection.user_addr || connection.addr || "",
              override_addr: "",
            });
            return clone(connection);
          },
          ExportDiagnostics: async () => {
            record({ method: "ExportDiagnostics" });
            return {
              ok: true,
              path: data.diagnosticsPath,
            };
          },
          Quit: async () => {
            record({ method: "Quit" });
            return null;
          },
        },
      },
    };
  }, init);
}

async function openDesktop(page, options = {}) {
  await installFakeBridge(page, options);
  await page.goto(options.path || "/");
  await expect(page.locator("#page-host .page")).toBeVisible();
}

async function calls(page) {
  return page.evaluate(() => window.__miopunchCalls || []);
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

async function shellLog(page) {
  return page.evaluate(() => window.__miopunchShellLog || []);
}

async function clipboardText(page) {
  return page.evaluate(() => window.__miopunchClipboard || "");
}

module.exports = {
  PEERS,
  clipboardText,
  calls,
  emitRuntime,
  expect,
  inviteCode,
  openDesktop,
  setRuntimeSnapshot,
  shellLog,
  snapshotFor,
  test,
};
