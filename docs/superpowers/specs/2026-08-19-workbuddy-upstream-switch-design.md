# WorkBuddy 上游切换设计

## 目标

按 cockpit-tools 的默认实例流程重建 WorkBuddy 的“切换并打开”：按官方 `~/.workbuddy/app` 数据目录识别和关闭目标实例，关闭成功后原子写入账号认证，启动新窗口并激活真实顶层窗口。

## 行为

- 运行中的实例只按 user-data 目录匹配，不按单个探测到的 `node.exe` 路径匹配。
- 关闭超时或权限失败直接返回错误，不写入目标认证，也不再弹出强制关闭事务。
- 未运行时直接注入认证并启动。
- 写入前清理 `.logged-out` 标记，写入后重新读取并校验 UID/accessToken。
- 启动后恢复最小化窗口，并通过 EnumWindows/SetForegroundWindow 激活 WorkBuddy 顶层窗口。

## 兼容性

保留现有账号快照和自定义认证路径设置；返回结构继续提供 `transactionId`、`forcedClose` 字段以兼容前端，但新流程不产生待确认事务且 `forcedClose=false`。
