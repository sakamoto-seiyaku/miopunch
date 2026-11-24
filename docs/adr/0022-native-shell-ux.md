# 采用 NSH 作为产品界面框架

2026-11-24，用户确认采用 Longbridge Native Shell（NSH）作为 miopunch 的产品界面框架。Windows、Linux 桌面和 Android 使用 NSH 的 Rust 入口和原生平台后端，在各端复用适用的界面组件与操作流程。该选择取代 [ADR 0018](0018-shared-vue-ui-with-tauri-shells.md) 中的 Vue／Tauri 技术栈决定。[NSH 平台说明](https://nsh.dev/docs/platforms/)

浏览器／PWA 使用 NSH Web 后端，复用适用的界面组件，接入自己的 Rust/WASM 节点。浏览器节点保留独立身份与成员资格，按浏览器能力适配存储、文件、剪贴板和页面生命周期。Linux 无界面宿主直接运行 Rust 节点。[NSH Web 后端](https://nsh.dev/docs/web/) · [浏览器节点入口](../../platform/browser/README.md)

节点身份、授权、连接与业务状态由 Rust 应用掌管。桌面宿主负责关窗后台运行、显式退出与登录自启，Android 宿主负责后台服务与界面重建后的接回。内网 Web 服务按首版 PRD 的要求在应用内承载远端内容，并隔离节点管理权限。[首版客户端 PRD](http://192.168.4.22:2080/work/projects/aa58cd82-2afc-404b-a097-58ef553c7ca4/issues/6159ec62-a03e-4eb7-9b95-4ddc19ce7a02)

此前 NSH v0.6.1 的 Pixel 6a 原型验证了原生界面、ANSI 终端显示和中文输入回显。产品接入仍需验证真实终端、Android 宿主生命周期、桌面交付及浏览器节点联动。实施和验收随各平台交付推进。
