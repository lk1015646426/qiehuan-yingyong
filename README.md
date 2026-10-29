# 切换应用

一款 **Windows 专用**的 TRAE / WorkBuddy 多账号管理工具，支持账号切换、积分查询和本地账号池。

> 本项目基于开源项目 [Cockpit Tools](https://github.com/jlcodes99/cockpit-tools) `v1.3.16` 改造而来，复用其成熟的 TRAE 账号管理与切号实现。详见 [NOTICE.md](./NOTICE.md)。

项目仓库：[lk1015646426/qiehuan-yingyong](https://github.com/lk1015646426/qiehuan-yingyong)。本次版本已完成 Work CN 与 WorkBuddy 多账号切换链路的稳定性修复。

## 下载与安装

当前版本：**0.1.17**。前往 [GitHub Releases](https://github.com/lk1015646426/qiehuan-yingyong/releases/tag/v0.1.17) 下载 `qiehuan-yingyong_0.1.17_x64-setup.exe`，同页提供 `SHA256SUMS.txt`。已安装旧版时可原地升级；升级前保存正在执行的任务。账号保存在当前 Windows 用户的本地数据目录，安装包不包含个人账号或密钥。

## 功能定位

每个账号首次正常登录一次并导入后，日常切换只需在管理器中点击「切换并打开」：

```text
点击「切换并打开」
→ 管理器保存当前账号最新会话
→ 正常关闭当前 TRAE Work CN
→ 注入目标账号完整登录快照
→ 启动官方 TRAE Work CN
→ 校验打开后的 UID 是目标账号
→ 完成
```

正常情况下无需再次输入密码、扫码、手动退出账号或编辑 `storage.json`。

主指标为 **总积分 / 已用积分 / 剩余积分**；积分签到继续由现有 GitHub Actions 完成，本软件只查询积分、维护登录快照并同步签到凭证。签到工作流由独立的 [daily-checkin](https://github.com/lk1015646426/daily-checkin) 仓库维护，本项目不新增或替代该工作流。

## TRAE 合并版兼容（2026-10-21）

官方 TRAE 3.4.1 起融合 TraeWork 与 TraeCode。工具会识别已安装合并版的产品元数据，自动使用其 EXE 和 `Trae CN` 数据目录；旧 Work/SOLO CN 安装保留兼容。导入、启动、切换回滚和会话监测使用同一个选定平台，不把旧 Work 的认证文件直接当作新客户端路径。原自定义 EXE 已失效时重新自动探测，搬动或重命名文件夹时优先按官方产品元数据识别。

合并版默认实例启动会尊重官方 `window.startupWindowType` 偏好，不再添加或沿用 `--new-window` / `--reuse-window` 编辑器窗口参数。本机已将默认启动窗口设为 `agent`；后续若自行更改官方偏好，默认启动界面也会随之改变。旧版 TRAE 和显式多开实例保持原来的开窗行为。

参考：[官方合并与迁移说明](https://docs.trae.cn/work_traework-to-traecode-data-migration?lang=zh)、[官方 3.4.x 更新日志](https://docs.trae.cn/ide_changelog)、[Cockpit Tools 路径处理](https://github.com/jlcodes99/cockpit-tools/blob/fe5f046c14d46b55da16d438eb853a2d2bfb89d4/src-tauri/src/modules/process_path_resolution.rs)。只移植适用的检测和兼容思路，没有整体升级上游，也没有引入另一套本地签到调度。

## WorkBuddy 多账号

WorkBuddy 活动轮询改为静默执行，每天一次完整云端汇总；Buddy 拥有状态通过官方 Buddy 信息确认，空闲旅行的 `buddy_id=0` 不再显示为未领取。

侧栏中的 **WorkBuddy** 页面与 TRAE Work CN 账号库完全隔离，支持导入任意数量的官方客户端登录快照、加密保存、切换并启动客户端，以及选择参与云端自动签到的账号。

- 首先在官方 WorkBuddy 客户端完成正常登录，再导入当前账号；工具不会自动输入验证码、扫码或绕过登录限制。
- 认证快照只保存在本机并使用 AES-256-GCM 加密。GitHub 仅接收签到需要的 access token，绝不接收完整认证文件、refresh token、session state 或完整手机号。
- 切换会完整原子替换认证文件。客户端无法正常退出时会提示保存任务并手动退出，不自动强制结束进程。
- 默认自动检测 WorkBuddy；可在“路径设置”中填写 EXE 或认证文件路径。认证文件路径会同时用于导入、切换与后台凭证监测。
- 启用“参与自动签到”的账号会同步到一个 `WORKBUDDY_ACCOUNTS_JSON` GitHub Secret。旧的 `WB1_TOKEN`、`WB2_TOKEN` 在聚合 Secret 尚未部署时仍可作为云端回退。
- 账号卡片会显示真实积分、今日奖励和连续签到天数；打开页面或点击“刷新积分 / 刷新全部积分”时，仅调用 WorkBuddy 的只读状态接口，不会执行签到。
- 页面顶部汇总所有已保存账号的可用积分，复用账号卡片数据。余额查询失败或缺失时显示已查询数量和部分合计，不把未知余额当作零。
- “多账号连续使用”从官方会话模型目录按需添加或移除模型，首次默认留空。保存所选模型后会启用本地账号池，程序重启后自动恢复；停止时会等待在途请求结束；清空并保存可移除全部账号池模型，原有官方及其他自定义模型保留。
- 模型选择保存在本机 `workbuddy-pool-selection.json`；目录查询失败时可使用已有缓存。客户端模型列表未更新时，保存当前任务后重新打开 WorkBuddy。账号池使用期间需要保持本工具运行；本地停止不会强制终止正在输出的回答。

- 普通积分查询不会刷新认证。需要恢复登录时，使用单个账号的“恢复认证”；当前正由官方客户端管理的账号优先吸收客户端新凭证，不另起竞争刷新。
- 凭证恢复会校验官方账号身份和权限上下文，保存完整有效期及刷新元数据。刷新后验证暂时失败的结果会加密保存在本机 `workbuddy-auth-pending/`，后续恢复会先继续验证，不直接重用旧 refresh token。
- 切换仅请求正常退出；20秒内不能退出时提示手动保存任务并退出，不自动强制结束。回滚以退出后的最后稳定登录状态为准，并保护其他写入的新状态。
- 当前认证写回按支持官方加密接口的 WorkBuddy 版本执行（本次修复在5.9.2上验证）；密钥不可用时停止写回，不退回明文格式。缺失本机账号库原密钥时停止读取，不创建替代密钥。

当前版本集中维护 TRAE 与 WorkBuddy；原智谱页面已从界面及命令入口移除。

## 最近修复

- 统一 Work CN 的平台别名和客户端路径识别，避免重启后账号消失或切换时找不到数据。
- WorkBuddy 认证写入失败时自动回滚，避免留下半保存状态。
- 区分 WorkBuddy 账号切换成功与窗口激活失败，错误提示不再误报。
- 统一 WorkBuddy 卡片 metrics 数据结构，修复积分和状态展示异常。
- 补充 Work CN 会话监听器的异常清理，降低重复监听和状态错乱风险。
- 同步前端锁文件版本，并补充切换、认证回滚、卡片数据和生命周期回归测试。

## 0.1.17 验证状态

- 修复 WorkBuddy 官方返回空账号类型时切换被误拦截的问题，保留 UID、令牌主体和企业身份校验。
- 修复 Windows 退出路径误调用 macOS 命令，并补充切换阶段与失败日志。
- WorkBuddy Rust 回归 78 项、进程控制 14 项、切换事务 2 项通过；另有 1 项原失败样本只读验证通过。4 个需显式启用的真实环境用例默认忽略。
- 前端相关回归 12 项、类型检查、生产构建和 diff 检查通过。
- Windows x64 NSIS 安装包已实际原地安装，版本与文件哈希核对通过；用户确认切换恢复正常。
- 本版本没有重新验证全部历史功能，其他失效账号仍需分别处理。

## 支持平台

- Windows 11（第一版仅保证 Windows）

## 致谢

- 上游项目：[Cockpit Tools](https://github.com/jlcodes99/cockpit-tools) by jlcodes
- 上游许可证：CC-BY-NC-SA-4.0

## 许可证

本项目继承上游许可证 **CC-BY-NC-SA-4.0**（非商业性使用 · 相同方式共享）。
