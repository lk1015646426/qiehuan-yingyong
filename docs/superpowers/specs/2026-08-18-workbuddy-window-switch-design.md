# WorkBuddy 切换并打开窗口激活设计

## 目标

修复 WorkBuddy“切换并打开”只启动进程、不可靠激活目标窗口的问题，使已运行实例能切换到已有窗口，未运行实例能启动并置前，并对失效句柄、最小化窗口、多个同名窗口、启动失败和权限异常给出可诊断结果。

## 根因

`workbuddy_switch::perform_switch` 当前只调用运行时的 `launch` 和认证校验，没有调用窗口激活逻辑。项目已有的按 PID 聚焦实现依赖单一 `MainWindowHandle`，没有枚举顶层窗口、检查句柄有效性，也没有确认 `SetForegroundWindow` 成功后的前台 PID；Electron/WorkBuddy 的启动返回 PID 可能是中间进程而不是拥有主窗口的进程。

## 方案

在 `WorkBuddySwitchRuntime` 增加 `activate_window` 能力。Windows 实现按 WorkBuddy 进程树重新收集 PID，使用 Win32 `EnumWindows`、`GetWindowThreadProcessId`、`IsWindow`、`IsWindowVisible`、`IsWindowEnabled`、`IsIconic`、`ShowWindowAsync(SW_RESTORE)`、`BringWindowToTop` 和 `SetForegroundWindow` 枚举并尝试所有候选顶层窗口；每次尝试后读取 `GetForegroundWindow` 的所属 PID 验证结果。候选句柄失效或中间进程无窗口时继续重试，直到启动/激活超时。非 Windows 平台复用现有平台聚焦工具，并返回明确错误。

切换流程保持现有关闭、备份、注入、回滚语义：认证写入并启动成功后，先等待账号校验，再激活窗口；激活失败会走原有回滚路径并返回新的窗口激活错误。目标未运行时同样执行启动并激活，已运行时关闭重启后激活新窗口。测试运行时增加激活调用记录，覆盖已有实例、未运行实例和激活失败路径。

## 错误处理

- 进程启动失败仍返回 `LAUNCH_FAILED`。
- 进程存在但在超时内找不到有效顶层窗口，或 `SetForegroundWindow` 被系统拒绝，返回 `WINDOW_ACTIVATION_FAILED`，详情包含平台错误信息。
- 激活失败继续恢复认证备份；若恢复失败，保留备份路径并返回 `ROLLBACK_FAILED`。
- 进程 PID 和窗口句柄均视为短生命周期数据，每轮尝试都重新验证，不缓存跨切换事务的 HWND。

## 验证

先运行 WorkBuddy 切换模块的失败回归测试确认测试能捕获缺失激活，再实现最小修复并运行该模块测试、完整 Rust 测试、前端测试和 Windows 构建。实际验证使用当前机器的进程/窗口查询；无法安全切换真实账号时，至少验证启动路径和窗口枚举辅助函数。
