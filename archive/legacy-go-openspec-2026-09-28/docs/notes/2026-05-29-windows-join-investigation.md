# Windows Join Investigation Logbook

日期：2026-05-29

状态：持续追加的排查流水账。本文只记录现象、证据、代码结论和本轮修复动作；除非标注“已确认”，否则不要把它当成最终根因。

## 1. 现象

- Linux 端创建网络并生成 join code 看起来成功。
- Windows 端使用该 join code 加入时，日志表现异常，但 join 最终没有成功完成。
- 目前 Windows 端日志只看到 daemon 启动，缺少足够的 signaling / broker / topic 事实，无法单靠界面判断失败点。

## 2. 当前可见事实

- extracted bundle 的 daemon log 只记录了启动与 state path。
- desktop log 也只记录了启动，连接失败时没有展开 join / approve / invite 的上下文。
- runtime 的 join / approve / invite 路径本身能返回 `reason_code`、`message`、`facts`、`suggestions`，适合继续承接诊断信息。

## 3. 这轮修复

- 为 `invite` / `approve` / `join` 增加 broker/topic/invite 的非敏感 facts。
- 让 `join` 的三个失败阶段更可分辨：
  - 打开 signaling session 失败
  - 发布 join request 失败
  - 等待 enroll response 超时
- 让桌面连接卡直接显示 failure facts，避免只看到一句摘要。
- 新增这份流水账文档，后续每次再遇到 Windows join 失败就追加时间戳、日志片段和结论。

## 4. 下一轮要看什么

- Windows join 失败时，优先看：
  - `broker_endpoint`
  - `join_topic`
  - `reply_topic`
  - `invite_id`
  - `network_id`
  - `peer_id`
- 如果 failure facts 证明 broker 可达但仍超时，再转向：
  - admin 是否真的 approve
  - reply topic 是否匹配
  - Windows 端是否连到了错误的运行时实例

## 5. 本轮验证

- `go test ./internal/pocv1/runtime ./cmd/miopunch-desktop`：通过
- `npm test`（frontend）：通过
- 结论：当前这批修改已经把 join 失败的排查信息稳定地推到 runtime / LocalAPI / desktop 可见层，下一步继续看真实 Windows 端 join 失败样本。
