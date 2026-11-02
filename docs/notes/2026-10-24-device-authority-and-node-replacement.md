# 设备权限、创世凭证与同一节点换机

MIOPUNCH-116 的非 UI 操作边界。产品依据为 MIOPUNCH-107、ADR 0013—0017；这里记录实现方式，不增加角色、热切换或物理单活承诺。正式页面和安装版平台接线由后续交付处理。

## 具体操作确认

`NativeClient` 与 `BrowserNetwork` 使用同一个 `DeviceOperation` 描述及签名结果。描述固定网络、发起节点及其授权来源、动作、目标及其当前角色和授权来源、随机操作标识；影响文案由动作决定。改变动作、目标或角色对应另一操作。任命管理员不会交付根凭证；撤销管理员保留普通成员。移除普通成员由日常管理员处理；普通成员可明确退出自身。任命、降权、移除管理员及管理员退出需要逐项创世确认。

| 行为 | Native JSON host | Browser API |
| --- | --- | --- |
| 生成待确认操作 | `genesis_propose` | `propose_device_operation` |
| 查看原操作及结果 | `device_operations` | `device_operations` |
| 本机或恢复卡逐项确认 | `genesis_confirm`，可选私有 `card_path` | `confirm_device_operation` / `confirm_device_operation_offline` |
| 向持有端请求本次确认 | `genesis_request` | `request_device_confirmation` |
| 查询本次签名结果 | `genesis_result` | `collect_device_result` |
| 执行无需创世权限的动作 | `device_execute` | `execute_device_operation` |

持有端收到请求只保存待确认描述，用户仍需在该端确认。结果绑定原描述和唯一管理记录，重试返回原记录；历史结果随节点备份保存。结果查询使用受限入口，允许已经完成自身退出的发起端取回该次结果，不授予业务访问。远端确认仅回传具体结果及签名历史，发起节点不会收到根私钥。离线确认只对本操作使用恢复卡，不把它自动保存为本机托管副本。Browser 的旧通用创世提权窗口拒绝授权；日常服务 ACL 沿用原权限规则。

确认入口与普通执行入口共享结果查询资格：尚未执行的普通动作走普通执行接口；已有签名结果的同一普通动作可通过确认入口查询原结果，不重新签名。

退出成功描述本机已结束成员资格，其他节点仍需收到签名记录；关闭应用或页面只停止运行，不自动发布退出。

## 实际凭证托管

托管独立于管理员角色。原持有端通过 `propose_credential_custody` 指定当前成员，`transfer_credential_custody` 首先只交付签名提议。接收端通过 `accept_credential_custody` 确认后，原端才能交付该网络根材料；接收端实际持久保存并读回后签署保存回执，原端收到它才报告 `saved`。原端保留副本，接收端保持原日常角色。重试保留同一提议和保存回执。

Native host 的对应命令为 `custody_propose`、`custody_transfer`、`custody_accept`、`custody_status`；Browser 状态入口为 `credential_custody`。Native 使用私有文件写入和读回，Browser 使用 origin 内的严格 IndexedDB 事务和读回；协议通过实际认证连接安全交付。它们不声称硬件密钥保险箱或能撤回外部副本。

停止托管先调用 `check_other_credential_custodians`（Native host：`custody_check`）。检查遍历本网其他当前成员，用新的挑战验证该成员当前确实持有根副本，并保留离线、无法确认、当前未保存等观察。检查有效期为 60 秒；删除时再次核对成员授权。恢复卡不计为另一成员的已确认副本，离线不计为没有副本。

`stop_credential_custody` / `custody_stop` 需要本次检查标识。没有近期已确认的其他成员副本时，普通删除拒绝；用户明确接受风险后可传 `force_without_confirmed_copy=true`。停止同时持久取消所有已有接收提议的待交付同意，包括尚未保存首份根材料的情况。已完成提议保留历史保存回执，不再执行保存；未完成的旧提议不能重新接受。需要重新保管时，由原端新建另一份提议并由接收端重新确认。Native 先保存停止决定再删除材料，Browser 在同一存储事务中保存停止决定与删除；重开与完整节点备份保留该决定。

删除仅移除本机软件管理的根材料，节点身份、成员资格及有效管理员角色保留。移除成员、删除本机副本都不能使别处保留的完整根副本失效。

## 恢复原网络的管理权限

只有恢复卡时，通过 `prepare_management_recovery` 创建新独立节点。该节点使用恢复卡中的原始网络锚点和根材料，向一个当前普通成员证明原根所有权；不需要旧管理员在线批准。根证明绑定本次实际 TLS 会话，不能用备份中的旧会话证明代替新证明。

`restore_management_authority` 完成签名管理授权与入网材料的持久保存后报告 `Complete`。同一恢复尝试重试保留原授权记录；之后若该授权被降权或撤销，旧尝试不能再次授予管理员。恢复不会自动撤销旧管理员，也不恢复旧节点身份或业务文件。Native host 对应 `recover` 启动方式、`management_recover` 与 `management_recovery_status`；Browser 同名状态入口返回本次恢复状态。

## 完整节点备份与换机

显式备份包含完整身份私钥、原网络锚点、签名管理和服务历史、入网材料、必要的审批和预编码账本、具体操作与托管记录、恢复证明及原结果、适用配置；原本持有根时一并携带。它是秘密材料，只在明确的私有导出／导入接口返回，不属于普通状态或日志。用户选择的业务文件不包含在身份胶囊中。

| 阶段 | Native | Browser |
| --- | --- | --- |
| 停止原端并导出 | `prepare_node_replacement(path)` | `prepare_node_replacement()` 返回私有文本 |
| 保存失败后仅重导出 | `retry_node_replacement_backup(source, path)` | 静态 `retry_node_replacement_backup()` |
| 新端导入 | `restore_node_replacement(text, target, bind)` | 静态 `restore_node_replacement(configuration, text)` |
| 联系当前成员核对授权 | `complete_node_replacement(contact)` | `complete_node_replacement(contact_id)` |
| 原端核对签名回执并退役 | 静态 `finalize_node_replacement(source, receipt)` | 静态 `finalize_node_replacement(receipt)` |
| 明确排除新端激活后中止 | 静态 `abort_node_replacement(source, package, true)` | 静态 `abort_node_replacement(package, true)` |

原端先停止并等待业务任务和端点结束，再生成一致快照；Browser 保留 Web Lock 到暂停标记实际提交后才释放。导出失败保留原端暂停状态，可重试保存，不静默重开。中止需要对应包标识及明确的“新端未激活”确认。

新端保持相同逻辑身份，初始为 `RestoringAuthorization`，不能提前提供普通业务或修改服务规则。它必须从实际当前联系人取得签名状态并核对仍有效资格；旧管理员快照采用已知降权后的普通成员角色，已移除成员或已撤销／到期来宾快照继续被拒绝。此核对相对该联系人已知的签名状态，不声称所有节点都已同步。

成功后新端签署对应包的回执，原端验证后删除自身管理的身份及私有材料，保留退役标记。错误回执不能删除；同一有效回执可重复提交。该过程不发布同一身份的 Leave／Remove。它不提供无中断热切换、跨设备全局锁或阻止用户复制原始磁盘的保证。

Native 与 Browser 的便携格式互通。平台特有配置单独保留；当前共同联络策略、禁用资源、路径选择和维护预算转入目标端适用配置，不重新启用已禁用设施。完整预编码核销账本和原授权结果保留，但恢复的旧码全部停止；新的明确发码正常可用。未完成的 Native 入网密钥轮换必须先完成，已完成的提案、确认和材料引用随备份保留。传统目录备份也保留服务及控制历史。每个新备份副本均重新建立授权隔离，直接打开已知备份、未核对的恢复再备份、便携导入后再做目录备份都不能洗掉该状态；旧格式已知备份首次打开时补存隔离元数据。完成核对后的回执只证明其对应目标，不能代替另一份新副本的当前联系人核对。 Native 在备份副本的任何公开激活入口先停止继承的邀请码，并保存与当前替换包绑定的本机停止标记；签名联系人历史不能替代发行端私有核销计数。旧备份缺少此停止证据时，即便已有资格回执，也先保守停止当时保存的全部邀请码一次，保留身份和有效签名资格。停止标记保存后，正常完成资格核对再发的新码及其计数可在同一已资格目录重开时延续，不重复停用。停止标记属于本机迁移材料，原端退役时一并清理；它不增加物理磁盘复制／回滚检测保证。

实际平台、构件、原始失败、恢复类别和清理证据见本票验收记录。Android 与正式安装宿主的系统接线仍由各自交付验收，不能由这里的 Native／Chromium 结果替代。


2026-10-29：Native 的共享 SQLite 关闭边界补齐连接归还与关闭交错时的 worker 等待；池已关闭但连接计数未归零时继续排空，避免目录发布早于实际文件句柄释放。此修复不增加发布重试、启动睡眠或放宽权限。实际 Windows/Linux 账本完整续办、同身份替换和凭证管理结果及原失败见[实机补充记录](../evidence/device-authority-2026-10-29/current-platform-qualification.json)。

2026-10-29 效果补充：当前 Native 接收端在停止与重开后，已完成旧 Deliver 只返回原回执，已停止的待交付和未接受提议均以真实托管协议关闭拒绝；每次独立读回根文件不存在、托管记录未变，新提议重新双端确认可实际保存。已知目录备份直开、恢复后再备份、便携导入后目录备份三种组合均实际拒绝隔离前 ACL 写入；当前联系人核对后采用降权，继续拒绝管理写入且普通成员 PING 正常，独立端的策略未改变。原测试漏注册协议的前置失败与清理保留；没有为此修改产品。Browser 保留既有普通托管生命周期及共享守卫源码资格，不声称运行了 crafted Browser Deliver。见[效果与整票验收](../evidence/device-authority-2026-10-29/effect-qualification.json)。
