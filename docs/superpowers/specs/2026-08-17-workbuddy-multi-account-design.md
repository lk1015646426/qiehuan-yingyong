# WorkBuddy 多账号切换与自动签到设计

## 1. 背景与目标

现有桌面工具已经提供 TRAE Work CN 账号导入、加密保存、切换、积分查询、GitHub Secrets 同步和云端签到管理。独立的 `云签到` 仓库已经能够使用 WorkBuddy access token 调用官方签到、签到状态和资源余额 API，但账号仍固定为 `WB1_TOKEN`、`WB2_TOKEN`，桌面端也没有 WorkBuddy 账号管理能力。

本功能在同一个 Windows 桌面工具中新增独立 WorkBuddy 页面，实现数量不限的本机账号管理、完整认证快照切换、凭证变化自动同步和 GitHub Actions 多账号签到。现有 WorkBuddy 签到接口、每天北京时间 04:00 的调度、通知和余额口径保持不变。

## 2. 已确认的产品决策

- WorkBuddy 与 TRAE Work CN 在侧栏中使用两个独立页面。
- 第一版只支持 Windows 和单实例切换，不支持多开。
- 账号数量不设业务上限。
- 用户必须在官方客户端完成手机号验证码、扫码或其他正常登录，工具不代替登录。
- 每个账号保存完整 `workbuddy-desktop.info` 快照，本机加密存储。
- 切换采用完整认证文件原子替换，不合并 `accounts` 或 `allAccounts`。
- 切换前保存当前账号最新快照；启动后以 `uid` 为主、`uin` 为辅校验。
- 客户端正常退出超时后必须询问用户，得到确认才能强制结束进程树。
- GitHub Actions 使用一个聚合 Secret：`WORKBUDDY_ACCOUNTS_JSON`。
- 聚合 Secret 只包含签到所需的 access token 和脱敏账号元数据，不包含完整认证快照、refresh token 或本机配置。
- 每个账号有“参与自动签到”开关，默认开启。
- 导入成功或检测到 access token 变化时自动同步；同步失败不影响本地导入和切换。
- 过渡期保留 `WB1_TOKEN`、`WB2_TOKEN` 回退读取。
- 账号卡支持触发 GitHub Actions，仅签到目标 WorkBuddy 账号。

## 3. 已核实的本机事实

设计时核实的目标电脑环境为：

```text
WorkBuddy 版本：5.3.13
卸载项显示名：WorkBuddy 5.3.13
实际 EXE：D:\联想软件下载内容\WorkBuddy\WorkBuddy.exe
进程名：WorkBuddy.exe
认证文件：%LOCALAPPDATA%\CodeBuddyExtension\Data\Public\auth\workbuddy-desktop.info
```

安装路径和版本仅是验收样本，程序不得写死上述绝对路径或版本。

当前认证文件顶层结构已经核实为：

```text
account
auth
accounts
allAccounts
```

`account` 包含 `uid`、`uin`、`nickname`、`phoneNumber` 等账号资料；`auth` 包含 `accessToken`、`refreshToken`、`expiresAt`、`sessionState` 等认证资料。`accounts` 和 `allAccounts` 只有账号资料，不能替代每个账号的完整 `auth` 快照。

## 4. 范围

### 4.1 第一版必须完成

- 自动检测 WorkBuddy 安装、进程和认证文件，并允许手动指定路径。
- 从当前官方客户端导入账号，按 `uid` 去重更新。
- 加密保存完整认证快照，索引只保存非敏感字段。
- 修改账号备注和自动签到开关。
- 一键切换并打开 WorkBuddy。
- 切换前捕获当前账号最新认证文件。
- 正常关闭、超时确认强制结束、原子替换、启动验证和失败回滚。
- 显示当前账号、Token 到期时间、积分余额和 GitHub 同步状态。
- 自动或手动同步 `WORKBUDDY_ACCOUNTS_JSON`。
- 兼容旧 `WB1_TOKEN`、`WB2_TOKEN`。
- 通过 GitHub Actions 触发全量或单账号 WorkBuddy 签到并显示运行状态。
- 全链路敏感信息脱敏。

### 4.2 明确不做

- WorkBuddy 多开或多个工作区实例同时绑定不同账号。
- 自动输入手机号、验证码、扫码或绕过安全验证。
- 合并或构造 WorkBuddy 内部 `accounts`、`allAccounts`。
- 本机计划任务或本机定时签到。
- 重写已经正常运行的 WorkBuddy 签到 API、余额算法和每日调度。
- 上传完整认证文件、refresh token、手机号或其他本机账号数据到 GitHub。
- 绕过平台风控、服务条款或服务端限制。

## 5. 总体架构

桌面端调用链：

```text
WorkBuddyPage
  -> workBuddyService.ts
  -> commands/workbuddy.rs
  -> workbuddy_account.rs / workbuddy_switch.rs / workbuddy_github.rs
  -> WorkBuddy.exe / workbuddy-desktop.info / GitHub CLI
```

云端调用链：

```text
WORKBUDDY_ACCOUNTS_JSON
  -> Config 动态账号解析
  -> WorkBuddySigner
  -> copilot.tencent.com 签到与余额 API
  -> 现有通知汇总
```

### 5.1 模块职责

`workbuddy_account.rs`：

- 定位、稳定读取和校验当前认证文件。
- 提取当前 UID、UIN、昵称、脱敏手机号和 Token 时间信息。
- 按 UID 导入或更新账号。
- 使用现有安全存储能力加密序列化完整认证快照。
- 生成不含凭证的账号视图模型。

`workbuddy_switch.rs`：

- 检测 WorkBuddy 安装与进程树。
- 管理互斥切换事务和事务 ID。
- 保存当前快照、关闭客户端、备份、原子替换、启动、验证和回滚。
- 区分“需要确认强制结束”和最终失败。

`workbuddy_github.rs`：

- 从所有开启自动签到的本地账号构造版本化聚合 JSON。
- 通过现有 `GitHubRunner` 和标准输入更新 Secret。
- 保存最后成功同步的内容摘要、时间和错误状态，摘要不得由 Token 明文构成。
- 复用现有 GitHub 仓库、工作流、CLI 安装与认证能力。

React WorkBuddy 功能：

- 独立类型、服务和 Zustand store。
- 独立页面、添加账号对话框、强制关闭确认框和设置入口。
- 复用已有按钮、状态条、GitHub 准备引导和签到运行状态组件的视觉规范。

Python 云签到：

- 在现有静态 YAML 账号之外增加聚合 Secret 动态账号源。
- 保持 `WorkBuddySigner.checkin()` 及三个 WorkBuddy API 的行为不变。
- 支持稳定账号键筛选，同时保留用户可读账号名称用于日志和通知。

## 6. 数据设计

### 6.1 本地目录

在专用版根目录内增加平台隔离数据：

```text
%USERPROFILE%\.trae_work_cn_switcher\
  workbuddy_accounts.json
  workbuddy_accounts\<account-id>.json
  workbuddy_github.json
  logs\workbuddy\
```

`workbuddy_accounts.json` 是非敏感索引；每个 `workbuddy_accounts/<account-id>.json` 是 AES-256-GCM 加密后的完整快照。不得与 TRAE 的账号详情文件、索引或 GitHub 槽位配置混用。

### 6.2 本地账号索引

索引至少包含：

```text
id                  稳定内部 ID，由 UID 的单向摘要派生
uid                 WorkBuddy UID；仅后端使用，前端按产品需要部分展示
uin                 辅助账号校验值
display_name        用户备注或昵称
masked_phone        脱敏手机号
checkin_enabled     是否进入聚合 Secret，默认 true
token_expires_at    access token 到期时间
created_at
updated_at
last_used_at
last_github_sync_at
last_github_sync_state
```

索引禁止包含 access token、refresh token、session state、完整手机号或完整认证 JSON。

### 6.3 聚合 Secret

`WORKBUDDY_ACCOUNTS_JSON` 使用带版本号的对象，不使用无版本裸数组：

```json
{
  "version": 1,
  "accounts": [
    {
      "key": "wb-7f3a8c91d2e4",
      "name": "个人号",
      "access_token": "[REDACTED_ACCESS_TOKEN]"
    }
  ]
}
```

规则：

- `key` 是由本地稳定账号 ID 生成的唯一云端键，不随备注修改。
- `name` 用于通知显示，必须去除控制字符并限制长度。
- `access_token` 是唯一上传的 WorkBuddy 认证字段。
- 账号按 `key` 排序后序列化，保证内容稳定和测试可重复。
- 同步状态比较使用安全摘要，不记录或输出 JSON 明文。
- 空账号列表也要覆盖远端 Secret，确保最后一个账号停用或删除后不会继续签到。

### 6.4 旧凭证兼容

云签到按以下优先级加载 WorkBuddy 账号：

1. `WORKBUDDY_ACCOUNTS_JSON` 存在且结构有效时，仅使用聚合账号。
2. 聚合 Secret 不存在时，使用现有 `config.yaml` 中的 `WB1_TOKEN`、`WB2_TOKEN`。
3. 聚合 Secret 存在但结构错误时，记录安全错误并回退旧账号，保证过渡期不中断。
4. 聚合 Secret 合法但账号列表为空时，视为用户明确停用全部 WorkBuddy 签到，不回退旧账号。

## 7. 核心流程

### 7.1 导入账号

1. 检测安装与认证文件。
2. 用户在官方 WorkBuddy 客户端正常登录。
3. 工具等待文件稳定后读取两次；读取结果一致才继续。
4. 解析 JSON，要求存在非空 `account.uid` 和 `auth.accessToken`。
5. 根据 UID 查找已有账号；存在则更新加密快照，不存在则新增。
6. 更新时保留用户备注、签到开关、创建时间和同步历史。
7. 加密写入账号详情，原子更新非敏感索引。
8. 刷新余额并异步同步 GitHub；同步失败不撤销导入。

### 7.2 切换账号

切换使用一个互斥、可恢复的事务：

1. 创建事务 ID；已有事务时返回 `BUSY`。
2. 读取当前认证文件，若 UID 匹配本地账号，则先更新其完整快照。
3. 请求 WorkBuddy 正常退出并等待主进程树结束。
4. 超时则返回 `FORCE_CLOSE_REQUIRED`，保留事务上下文但不修改认证文件。
5. 用户确认后以同一事务 ID 继续并强制结束进程树；取消则释放事务。
6. 将当前认证文件复制到事务备份文件。
7. 解密目标快照、校验 JSON，并在同目录使用临时文件加原子重命名替换认证文件。
8. 回读写入文件，验证其 UID 与目标 UID 一致。
9. 启动 WorkBuddy，轮询认证文件和进程状态验证 UID；UID 缺失时可用 UIN 辅助诊断，但不得把不同 UID 仅因 UIN 相同判为成功。
10. 成功后删除事务备份、更新 `last_used_at`，并在 Token 变化时异步同步 GitHub。
11. 任一步骤失败时恢复原认证文件；原客户端此前运行则尝试重新启动。

强制结束只能发生在认证文件尚未替换时。前端取消确认后不得留下锁、备份或半完成事务。

### 7.3 后台凭证监测

- 复用 TRAE 会话监测器的设计模式，但监测路径和账号库完全独立。
- 只有认证文件修改时间或内容安全摘要变化时才解析。
- 当前 UID 匹配本地账号且完整快照变化时，更新加密快照。
- access token 变化且账号开启签到时，触发防抖后的聚合 Secret 同步。
- 监测失败只更新脱敏状态，不中断 WorkBuddy 或桌面应用。

### 7.4 GitHub 同步

1. 读取所有 `checkin_enabled=true` 的账号。
2. 解密各自快照并提取 access token。
3. 任一启用账号缺少 token 时，将该账号标为不可同步并在界面列出；不得静默上传不完整集合。
4. 构造、排序并校验版本化聚合 JSON。
5. 使用 `gh secret set WORKBUDDY_ACCOUNTS_JSON --repo owner/repo`，内容只写入 stdin，不传 `--body` 或任何包含 Secret 的命令行参数。
6. 远端成功后更新本地同步摘要和时间。
7. 失败时保留旧远端 Secret，记录脱敏错误并允许重试。

为了避免一次坏账号阻止其他已配置账号长期签到，用户可以关闭该账号的签到开关后重新同步。

### 7.5 单账号立即签到

1. 要求账号已开启自动签到且 GitHub 配置就绪。
2. 先同步最新聚合 Secret。
3. 使用现有工作流触发能力传入筛选值 `workbuddy:<stable-key>`。
4. Python 入口按 `Account.key` 匹配，通知仍显示 `Account.name`。
5. 获取本次 `workflow_dispatch` 的运行 ID，轮询到完成或界面超时。
6. 页面显示成功、失败、取消或仍在运行，并提供 Actions 链接。

## 8. 页面设计

WorkBuddy 页面顶部显示客户端检测、当前账号、GitHub 就绪状态和全量同步入口。账号列表为响应式卡片布局，账号数量不限。

每张账号卡显示：

- 用户备注或昵称、脱敏手机号和 UID。
- 当前账号、可切换、快照失效等状态。
- Token 到期时间。
- 积分余额、今日签到奖励和连续签到天数；没有新数据时显示未知，不伪造为 0。
- 自动签到开关。
- GitHub 已同步、待同步或失败状态。
- 切换并打开、刷新余额、同步 GitHub、立即签到、更多和删除操作。

删除当前账号只删除管理器记录和远端签到集合，不注销或关闭当前 WorkBuddy。删除后远端同步失败时显示“本地已删除，GitHub 待清理”并提供重试。

## 9. 客户端检测与生命周期

EXE 检测顺序：

1. 用户保存的手动路径。
2. Windows 卸载注册表的 `DisplayIcon` 或安装命令元数据。
3. 开始菜单 WorkBuddy 快捷方式。
4. 当前运行中 `WorkBuddy.exe` 的可执行路径。
5. 已知常见目录回退。

认证文件默认使用 `%LOCALAPPDATA%\CodeBuddyExtension\Data\Public\auth\workbuddy-desktop.info`，设置页允许手动指定。

关闭流程必须先请求正常退出并等待整个 WorkBuddy 进程树结束。超时不自动杀进程；只有用户确认后才能强制结束。启动时只启动主 EXE 一次，不逐个启动或结束其子进程。

## 10. 安全与日志

- 完整快照使用现有 AES-256-GCM 安全存储模块。
- 前端命令返回值不得包含 `auth`、token、session state 或完整手机号。
- GitHub Secret 内容只走子进程 stdin，不进入参数、环境变量、日志或持久化临时文件。
- 错误响应、Rust 调试输出、Python 异常、测试夹具和通知必须经过敏感字段脱敏。
- 日志允许记录稳定内部账号 ID、阶段、错误码、HTTP 状态和文件路径；不允许记录认证文件原文。
- 原子替换临时文件和事务备份必须位于认证文件所在目录，并在成功或完成回滚后清理。
- 删除本地账号前需要二次确认；清除全部凭证必须单独确认。

## 11. 错误模型

后端使用结构化错误码，至少覆盖：

```text
ACCOUNT_NOT_FOUND
AUTH_FILE_NOT_FOUND
AUTH_FILE_INVALID
SNAPSHOT_INCOMPLETE
CLIENT_NOT_INSTALLED
CLIENT_CLOSE_FAILED
FORCE_CLOSE_REQUIRED
SWITCH_CANCELLED
BACKUP_FAILED
INJECT_FAILED
LAUNCH_FAILED
VERIFY_TIMEOUT
VERIFY_ACCOUNT_MISMATCH
ROLLBACK_FAILED
GITHUB_NOT_READY
GITHUB_SYNC_FAILED
WORKFLOW_TRIGGER_FAILED
BUSY
```

本地切换错误和 GitHub 错误是两个独立结果域。GitHub 失败不得把已经成功的本地切换报告为失败；界面应显示“切换成功，GitHub 待同步”。

## 12. 测试与验收

### 12.1 Rust 自动测试

- 认证 JSON 完整、缺字段、格式错误和写入中读取。
- UID 去重更新并保留用户设置。
- 加密详情不出现 token 明文。
- 账号视图不序列化认证字段。
- 进程退出成功、超时等待确认、取消强制结束和确认强制结束。
- 正常切换、备份失败、注入失败、启动失败、UID 不匹配和回滚失败。
- 并发切换返回 `BUSY`。
- 聚合 JSON 排序、空列表、缺 token、Token 变化检测和日志脱敏。
- GitHub CLI 参数不含 Secret，Secret 内容只进入 stdin。

### 12.2 TypeScript 自动测试

- 独立 WorkBuddy 路由和页面加载。
- 账号卡当前状态、快照状态、Token 到期和 GitHub 状态。
- 自动签到开关与同步反馈。
- 强制结束确认框的继续与取消分支。
- 导入、切换、删除、立即签到期间禁用重复操作。
- 账号数量较多和窄窗口下无文字重叠或操作溢出。

### 12.3 Python 自动测试

- 合法聚合 Secret 生成任意数量的动态账号。
- 账号稳定键与显示名分离，筛选使用稳定键。
- 聚合 Secret 不存在时回退 `WB1_TOKEN`、`WB2_TOKEN`。
- 聚合 Secret 格式错误时安全回退且日志不含原文。
- 合法空列表不回退旧账号。
- 重复键、空 token、错误版本和超长字段被拒绝。
- 现有 WorkBuddy 签到成功、今日已签、401、非 JSON、余额和连续天数行为不回归。
- 一个账号失败后仍执行后续账号，并按现有规则汇总退出码。

### 12.4 Windows 人工验收

1. 在 WorkBuddy 5.3.13 上依次登录并导入至少两个账号。
2. 两个账号往返切换两轮，启动后 UID 均正确。
3. WorkBuddy 有未保存编辑内容时切换，确认正常关闭路径不会丢失内容。
4. 模拟退出超时，取消强制结束后认证文件完全不变。
5. 模拟注入后启动失败，确认恢复原文件并能重新打开原账号。
6. 检查本地账号文件、应用日志和前端状态，确认没有 token、refresh token 或完整手机号。
7. 同步两个以上账号到 `WORKBUDDY_ACCOUNTS_JSON`，触发单账号签到并确认只运行目标账号。
8. 等待或手动验证每天北京时间 04:00 的全量任务，确认 WorkBuddy、TRAE 和其他站点不回归。
9. 停用或删除一个 WorkBuddy 账号并重新同步，确认该账号不再执行云端签到。

## 13. 完成标准

满足以下条件才算第一版完成：

- 两个及以上真实 WorkBuddy 账号能够稳定导入、加密保存并往返切换。
- 任何已覆盖的切换失败场景都不会把用户留在损坏或空白认证状态。
- 任意数量的启用账号能够通过一个聚合 Secret 进入现有 GitHub Actions。
- 旧 `WB1_TOKEN`、`WB2_TOKEN` 在聚合 Secret 未部署时继续工作。
- 单账号立即签到和每天 04:00 全量签到均可观察结果。
- 自动测试通过，真实 Windows 与 GitHub 验收通过。
- 日志、界面、通知、命令参数和持久化非敏感索引均未泄露凭证。
