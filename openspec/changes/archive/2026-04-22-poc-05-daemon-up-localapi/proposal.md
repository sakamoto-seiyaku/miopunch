## Why

当前 `miopunch` POC CLI 命令树（`up/ls/invite/approve/join/ping/sh/reset/...`）仍处于占位状态，且缺少“常驻进程 + CLI”的最小闭环，导致：

- CLI/UI 无法共享同一套可解释性输出与任务进度（阶段机/诊断/报告），口径容易漂移；
- 长操作（例如 `approve` 监听、`join` 进度、未来 `sh` 交互）无法以统一的 task 形式被观察/重试/导出报告；
- 缺少稳定的本机 IPC 接口，后续 HTTP 面板（POC-07）与产品化演进缺少地基。

因此需要在 POC-05 先把 `miopunch up` 常驻与 LocalAPI（CLI↔daemon）跑通，并冻结最小输出契约，为 POC-06/07 提供稳定接口。

## What Changes

- 引入 `miopunch up` 常驻进程（前台运行；后台化交给系统服务托管），并冻结“同机同一 operator 只允许 1 个实例”的互斥与探测语义。
- 引入 LocalAPI v0（仅本机 IPC，不暴露为可被外网访问的 TCP 端口）：
  - HTTP/JSON：资源读取与 task 创建/查询
  - SSE：全局事件流与 task 事件流（统一事件模型；连接后必发 `snapshot`）
  - WebSocket：仅用于 `sh_attach` 的字节流通道（本 change 冻结子协议与路由；交互实现留给 POC-06）
- 冻结 POC v0 输出契约（CLI + LocalAPI 共用）：
  - 稳定字段：`stage/reason_code/term_id/exit_code`（不重命名，只新增；改名用 alias/deprecated）
  - `--format json` 顶层最小稳定 envelope（只承诺顶层字段存在，不承诺完整 schema）
  - LocalAPI 错误模型：HTTP status 反映成败，响应体至少包含 `stage/reason_code/exit_code/message/facts/suggestions/request_id`
- 纳入 `install-system-daemon/uninstall-system-daemon` 的最小契约与实现任务：安装到稳定路径、最小权限模型、以及“不清 state；state 用 reset”的边界。

## Capabilities

### New Capabilities
- `miopunch-poc-daemon-up`: 定义 `miopunch up` 的常驻职责边界、实例互斥、system/user 两种运行形态的默认目录/权限，以及 system service 安装/卸载的最小行为契约。
- `miopunch-poc-localapi-v0`: 定义 LocalAPI v0 的 transport（Linux unix socket / Windows named pipe）、默认地址、访问控制（OS ACL + 固定 Host）、路由集合、SSE/WS 子协议约束与 task 资源模型。
- `miopunch-poc-output-contract-v0`: 定义 POC v0 的稳定输出字段、`miopunch.json.v0` envelope 下限、以及 `exit_code` 到 HTTP status 的粗分类映射要求。

### Modified Capabilities
- (none)

## Impact

- Affected code (expected):
  - `cmd/miopunch`：落地 `up` 与 LocalAPI client 模式（除少数命令外，CLI 只创建 task 并渲染 events/输出）。
  - `internal/*`：新增/扩展 daemon、LocalAPI server、task 运行框架、SSE/WS 支撑与报告导出。
- Dependencies (expected):
  - 引入跨平台 service 安装库（例如 `github.com/kardianos/service`）用于 systemd/Windows Service 托管。
- Non-impact:
  - 不改变打洞/数据面协议；`sh_attach` 的真实交互与单写者锁语义留给 POC-06（本 change 只冻结 LocalAPI/WS 契约并提供 stub）。

