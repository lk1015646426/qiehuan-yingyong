# 切换应用 — 开发文档（AI 快速理解版）

> 本文档面向 AI 助手与新接手的开发者，是项目的**唯一权威开发文档**，整合自已删除的各阶段报告/设计/交接文档。日期：2026-08-24。

## 1. 项目是什么

**一句话**：Windows 桌面工具，分别管理 TRAE Work CN 与 WorkBuddy 的本地登录账号；支持一键切换客户端账号、同步云端签到凭证，以及查看 WorkBuddy 的真实积分和签到状态。

- **本质**：基于上游 `cockpit-tools` v1.3.16（commit `e1ef55ce`）改造并完成独立命名的 Windows 桌面应用。
- **许可证**：继承上游 **CC-BY-NC-SA-4.0**（非商业、相同方式共享），不可更改。
- **标识**：identifier `com.qiehuanyingyong.desktop`；productName「切换应用」；Cargo 包名 `qiehuan-yingyong`；Rust 库 `qiehuan_yingyong_lib`；数据目录 `~/.qiehuan_yingyong`。
- **日常签到在云端**：本地只同步凭证与触发 workflow，**绝不在本地执行签到/claim**。WorkBuddy 的桌面端“刷新积分”同样是只读查询，不会签到。
- **仓库职责**：本项目源代码位于私有仓库 [lk1015646426/qiehuan-yingyong](https://github.com/lk1015646426/qiehuan-yingyong)；日常签到由独立仓库 [daily-checkin](https://github.com/lk1015646426/daily-checkin) 的 GitHub Actions 负责。本项目不新增、不复制签到工作流。
- **敏感数据边界**：账号快照、Token、PAT、私钥、日志、数据库和构建产物不得进入 Git；上传快照只保留源码、配置模板、文档、测试和必要静态资源。

## 2. 快速上手

```powershell
# 开发（注意是 tauri:dev，不是 tauri dev —— 后者绕过脚本包装会缺 prepare 步骤）
npm run tauri:dev

# 类型检查 / 前端构建
npx tsc --noEmit
npm run build

# 后端检查与测试
cargo check --manifest-path src-tauri/Cargo.toml
cargo test  --manifest-path src-tauri/Cargo.toml

# 产出 Windows NSIS 安装包（唯一发布目标）
npm run tauri -- build --ci
# → target\release\bundle\nsis\切换应用_0.1.2_x64-setup.exe
```

- 开发目录：`...\Desktop\脚本\切换应用\source`；**禁止**改动原版 Cockpit Tools 安装目录（`AppData\Local\Cockpit Tools`），两者数据完全隔离。
- 本工具数据目录：`~/.qiehuan_yingyong`（可用环境变量 `QIEHUAN_YINGYONG_DATA_DIR` 覆盖；测试另有 `_TEST_DATA_DIR`）。dev 与 release **共用**同一数据目录，并自动迁移 TRAE、Antigravity、Cockpit 等旧目录及旧环境变量。
- 用户客户端目录（探测目标）：`%USERPROFILE%\.trae-solo-cn\` 与 `%APPDATA%\TraeWork CN\`（见 §5 路径别名）。

## 3. 技术栈

| 层 | 技术 |
|---|---|
| 前端 | React 19 + TypeScript 5.8 + Vite 7，Zustand 5（无路由库，页签用 useState），i18next（18 语言），lucide-react 图标 |
| 样式 | 原生 CSS + 设计 token（`src/styles/base.css` 定义 `--primary/--success/--warning/--danger/--bg-*/--border*` 等变量，自动适配暗色）。**未启用** Tailwind/daisyUI（devDeps 存在但无配置，勿引入） |
| 后端 | Tauri 2（tray-icon / image-png / macos-private-api features），Rust 2021 |
| 关键依赖 | reqwest 0.12（blocking + async）、rusqlite(bundled)、tokio(full)、tracing、chrono、base64、winreg(Windows) |
| 插件 | opener / dialog / fs / notification / autostart / single-instance / deep-link（scheme `qiehuanyingyong`，兼容旧 `cockpit-tools`） |

## 4. 代码结构地图（核心：区分"本 fork 新增"与"上游遗留"）

```
src/
  App.tsx                      # 侧边栏双页签：账号切换(switcher) / 云端签到(checkin)
  pages/
    WorkCnSwitcherPage.tsx     # 主页：安装探测、账号卡（完整度/积分/活跃标记）、导入/切换/删除
    CheckinPanelPage.tsx       # 签到面板：token 倒计时、半自动刷新流、同步全部、运行历史、gh 引导弹窗触发
    WorkBuddyPage.tsx          # WorkBuddy：导入/切换、自动签到开关、真实积分与签到状态只读展示
  stores/
    useWorkCnStore.ts          # 账号库/积分/GitHub 配置/CLI 状态/gh 安装登录/会话监测（原子动作编排层）
    useCheckinStore.ts         # workflow runs、触发、半自动刷新流状态机（复用 useWorkCnStore，不加后端状态机）
    useWorkBuddyStore.ts       # WorkBuddy 账号库、切换、GitHub 同步、按账号状态刷新
  services/
    workCnService.ts           # 全部 work_cn* invoke 封装 + 事件常量 + openExternalUrl
    checkinService.ts          # trigger/list 两个签到命令
    workBuddyService.ts        # WorkBuddy Tauri 命令封装，含 getWorkBuddyAccountStatus
  components/work-cn/
    GhSetupDialog.tsx          # gh 上传三条件引导弹窗（安装/PAT 登录/配置），自动检测自动弹
    WorkCnAddAccountDialog.tsx / WorkCnSettingsDialog.tsx / WorkCnStatusBanner.tsx
  types/ workCn.ts checkin.ts  # 镜像后端 serde 模型（camelCase）
  styles/pages/work-cn.css     # wc-* / gh-* 类；checkin.css 为 ck-* 类

src-tauri/src/
  lib.rs                       # Builder 注册全部命令/插件/托盘/窗口事件；启动 ensure_started(watcher)
  commands/
    work_cn.rs                 # 安装探测/导入/列表/校验/切换/积分/删除/清凭证/会话状态
    work_cn_github.rs          # GitHub 配置读写、cli 状态、gh 自动安装、PAT 登录、单账号同步（全 async+spawn_blocking）
    work_cn_checkin.rs         # 触发云端 workflow、查运行历史（async+spawn_blocking）
    workbuddy.rs               # WorkBuddy 安装、账号、切换、GitHub 与状态查询命令
  modules/
    trae_account.rs            # ★ 上游核心复用：storage.json 解析、设备 ID 提取、快照导入、注入
    work_cn_github.rs          # GitHubRunner trait（Real/Fake）、secret 同步、gh 路径解析、超时看门狗
    work_cn_session_watcher.rs # 后台 60s 会话监测：token 轮换回写 + 同步 GitHub
    work_cn_checkin.rs         # gh workflow run / gh run list
    workbuddy_account.rs       # WorkBuddy 加密快照、导入、写入和 access token 读取
    workbuddy_status.rs        # WorkBuddy 真实积分、今日奖励、连续签到的只读 HTTP 查询与解析
    gh_setup.rs                # gh MSI 自动下载安装（msiexec /passive）+ PAT stdin 登录
    account.rs secure_account_storage.rs atomic_write.rs   # 账号库、AES-256-GCM、原子写（上游）
    process.rs config.rs db.rs tray.rs main_window_state.rs # 上游遗留基础设施
  models/work_cn.rs            # 全部 Work CN 数据结构 + WorkCnErrorCode
```

**上游遗留大块**（`crates/`、`sidecars/cockpit-cliproxy/`、codex/多 IDE 模块、多语言 README、CHANGELOG 等）：与 Work CN 无关但保留在仓库里，**改动 Work CN 时不要碰它们**，也不要误把它们当成本项目职责。

## 5. 核心概念与协议（最容易踩坑的部分）

### 5.1 两种设备 ID —— 绝不可混淆（代码为准确认）

| 字段 | 来源 | 形态 | 用途 |
|---|---|---|---|
| `auth_device_id` | storage key 前缀 `iCubeAuthInfo://icube-dc:<ID>` | **数字**长 ID | 登录/注入链路 |
| `checkin_device_id` | `telemetry.devDeviceId` | **UUID** | 签到接口 + GitHub Secret `TRAE{N}_DEVICE_ID` |

早期文档把两者写反过，以本表为准（`trae_account.rs:3783` 注释为准绳）。

### 5.2 平台标识 —— 双平台兼容

内部枚举 `TraePlatformKind::TraeSoloCn`（`trae_solo_cn`）与 `TraeCn`（`trae_cn`）**都要识别**：账号导入与路径发现必须同时接受两者（历史 bug：只认 `trae_solo_cn` 导致重开程序账号消失）。显示层统一叫「TRAE Work CN」，内部标识**禁止**新增 `trae_work_cn` 枚举。

### 5.3 客户端路径与注册表

- 数据目录别名（都可能是真实目录）：`.trae-solo-cn` / `TraeWork CN` / `.trae-cn` 等；EXE 候选按序探测。
- 注册表读取**必须用 winreg crate**（原生 REG_SZ Unicode）；上游的 `cmd /u /c reg query` 在中文 Windows 上返回"找不到键"且会损坏 GBK 路径，禁止回退使用。

### 5.4 槽位与 Secret 命名

- 槽位 1~4，每账号最多绑一个槽位、每槽位最多一个账号；`TRAE{N}_TOKEN` / `TRAE{N}_DEVICE_ID` 自动生成，secret 名只允许 `[A-Z0-9_]+`。
- `github.json`（存 enabled/repository/workflowFile/slots）**永不存 PAT**；写入必须原子写（先 `.json.bak` 备份 → `.json.tmp` → rename），历史上有半截 JSON 导致配置丢失的事故。

### 5.5 账号快照（导入时抓全 8 字段）

access_token、refresh_token、`auth_device_id`（icube 数字）、`checkin_device_id`（UUID）、`machine_id`、`deviceKeyPair`（部分版本嵌套在 `trae_auth_raw.deviceKeyPair`）、email/user_id。**禁止只存 access token**（会导致切过去无法免登）。落盘走 `secure_account_storage` AES-256-GCM；对前端只回脱敏 `WorkCnAccountView`。

## 6. 六条功能链路

### 6.1 安装探测（`get_work_cn_installation`）
只读路径/注册表/进程，**不碰任何登录密钥**，可安全地每次页面加载调用。

### 6.2 导入快照（阶段 3）
读客户端 `storage.json` → 解析上文 8 字段 → 校验完整度 → 加密落盘 → 返回脱敏视图。label 存为 tag，email 永不覆盖。

### 6.3 事务切换（阶段 4，`switch_work_cn_account`）
状态机：**全局切号锁 → 快照完整度校验 → 账号库同步 → 备份客户端当前 storage → 关闭客户端进程 → 注入目标账号凭证 → 绑定 → 启动客户端 → 校验登录身份 → 失败全量回滚**。错误以序列化 `WorkCnCommandError`（code+message+detail）JSON 返回，前端按 `code` 分支。锁必须 `try_lock` 非阻塞获取，防 watcher 与用户操作竞争。

### 6.4 积分查询（阶段 5，只查不领）
调用量接口解析 `user_entitlement_pack_list`：多包求和；`unlimited` 记无限；`status=0` 的积分包跳过；剩余值**按浮点解析**（有 0.5 积分包）；剩余不为负。查询失败保留缓存值展示。

### 6.5 GitHub 同步 + 云端签到（阶段 6/8）
- 同步：`GitHubRunner` 抽象（Real 调 `gh` CLI，Fake 供测试断言）；只做 `gh auth status` + `gh secret set`，secret 值**只走 stdin**；token 需带未过期 JWT `exp`，过期则跳过并注明原因；首个 secret 失败短路，绝不谎报整体成功。
- 签到：本地只 `gh workflow run`（手动验证/补签）+ `gh run list`（历史）；日常由云端 Actions 北京时间 04:00 定时执行。
- **Real runner 实现要点**（都是修过的真实事故）：spawn 加 `CREATE_NO_WINDOW`（否则闪黑框）；超时看门狗 探测 30s / 写 secret 180s 强杀进程树（否则网络黑洞永久挂死）；gh 路径先探 `Program Files\GitHub CLI\gh.exe` 等默认安装位再扫 PATH（装完 gh 无需重启应用）；stderr 经 `redact_for_log` 把 ≥20 位 base64 串替换 `[REDACTED]`。
- **命令层要点**：所有 gh 相关 Tauri 命令必须 `async` + `tauri::async_runtime::spawn_blocking`（Tauri 2 同步命令跑主线程，`gh auth status` 联网数秒会冻结整个窗口——修过的最大卡顿事故）。

### 6.6 后台会话监测 watcher（阶段 7）
启动后延迟 10s，每 60s `watch_once`：读客户端 `storage.json`（mtime 门禁去重）→ 身份匹配账号库 → 回写轮换后的新 token（**必须 `preserve_account_metadata`：回写时保留 deviceKeyPair 等既有元数据，否则会把设备密钥清掉——重大历史坑）→ 若该账号绑定槽位且启用同步，则同步 GitHub。约束：只 `gh auth status` + `gh secret set`，**禁触 workflow**；拿不到切号锁立即放弃本轮；会话失败退避 15min、GitHub 失败退避 10min；状态变化经事件 `work-cn:session-watch` 推送前端（payload 为脱敏 `WorkCnSessionWatchStatus`）。

### 6.7 gh 条件引导（GhSetupDialog，最新）
进入签到页自动检测三条件（gh 已安装 / gh 已登录 / 同步配置完整），不全则弹引导（每次应用会话只自动弹一次）；弹窗内可：自动下载安装 gh（Releases API 取最新版，失败回退固定版本 2.63.2；MSI 下载带进度条事件 `gh-setup:progress`；`msiexec /passive /norestart`，UAC 由系统弹）、PAT 登录（stdin，绝不落盘/不进命令行/不进日志）、就地填配置。点「同步全部 GitHub」条件不全也弹引导而非报错。

### 6.8 WorkBuddy 真实积分与签到状态（只读）

后端入口为 `commands/workbuddy.rs:get_workbuddy_account_status`，调用 `modules/workbuddy_status.rs:query_account_status`；令牌从本地 AES-256-GCM 加密的 WorkBuddy 账号快照读取，绝不返回前端或写入日志。

- 积分：`POST /v2/billing/meter/get-user-resource`。仅累加 `CapacityType == 1`、`Status == 0` 的资源包；优先使用 `CapacityRemainPrecise`，缺失时回退 `CapacityRemain`，结果保留两位小数。体验包和失效包不计入真实积分。
- 签到状态：`POST /v2/billing/meter/checkin-activity-status`，兼容 `today_credit` / `daily_credit` 与 `streak_days` 字段。
- 两个请求并发执行，但一项失败不会清空另一项成功数据；错误仅显示在对应账号卡片中。
- **禁止**在状态查询模块调用 `daily-checkin`。该接口会执行签到，只能由云端 GitHub Actions 链路使用。
- 前端在 `loadAccounts` 后按账号顺序刷新；页面提供“刷新积分”和“刷新全部积分”。卡片状态使用 `wb-status-line` 紧凑文字行，禁止改回嵌套指标卡片或固定最小高度。

## 7. 硬约束（红线，违反即返工）

1. **本地绝不签到/claim**；签到永远在云端 Actions。watcher 连 workflow 都不许触发；WorkBuddy 状态查询只能使用只读接口。
2. **切换必须事务**：注入前关闭客户端；任一步失败回滚到备份；绝不留半切换状态。
3. **快照必须完整**：禁止只存 access token；两种设备 ID 不可混用。
4. **凭证安全**：PAT/secret 只走 stdin；日志一律 `redact_for_log`；`github.json` 永不存 PAT；前端只见脱敏视图。
5. **配置写入原子化**（backup → tmp → rename），杜绝半截 JSON。
6. **UI 用设计 token**（base.css 变量），禁止硬编码颜色/尺寸内联样式（项目约束，视觉一致性）。
7. **gh 子进程**：CREATE_NO_WINDOW + 超时看门狗 + async/spawn_blocking，三者缺一不可。
8. **外部链接**：WebView 内 `<a target="_blank">` 不会唤起浏览器，必须 `openExternalUrl()`（opener 插件）。
9. **平台兼容**：`trae_solo_cn` 与 `trae_cn` 双识别；不新增 `trae_work_cn` 枚举。
10. **git 纪律**：小提交、TDD（新逻辑先写测试）；`cargo test` 全绿 + `tsc` 通过才能提交；不 push 不发布除非用户明确同意。
11. **目录隔离**：开发目录与原版 Cockpit Tools 安装目录严禁混写；测试用 `_TEST_DATA_DIR` 隔离，绝不触真实用户数据。

## 8. 前端约定

- **状态分层**：`useWorkCnStore` 是原子动作层（后端 invoke 的薄封装 + 错误处理），`useCheckinStore` 只做跨动作编排（半自动刷新流：switching → waiting(事件+轮询双通道,3min 超时) → syncing → done/failed，完成后提供"切回原账号"按钮）。新功能优先复用该模式，不加后端状态机。
- **样式类名**：work-cn 界面 `wc-*`，WorkBuddy 界面 `wb-*`，gh 引导弹窗 `gh-*`，签到面板 `ck-*`；颜色一律 `var(--*)`。
- **WorkBuddy 卡片**：内容区保持紧凑自然高度；积分、奖励、连续签到使用 `wb-status-line` 横向文本，不创建嵌套指标卡片。
- **错误展示**：后端 `WorkCnCommandError` JSON 字符串由 `parseWorkCnCommandError` 还原为带 `code` 的 Error。
- **关闭行为**：主窗口 × → 最小化到托盘（不退出）；退出走托盘菜单。
- **安装包**：NSIS `currentUser` 模式 + `installer-hooks.nsh`（安装时自动杀旧进程，解决覆盖安装不生效）。

## 9. 测试

- 后端：`cargo test`；WorkBuddy 定向验证为 `cargo test workbuddy --manifest-path src-tauri/Cargo.toml`，覆盖账号快照、GitHub 同步、状态接口和积分过滤解析。
- 前端：`npm run typecheck` 必过；WorkBuddy 页面契约为 `node --test src/utils/workBuddyLifecycle.test.ts`。
- 验收纪律：任何改动本人跑 typecheck + cargo test + tauri build 并人工确认后才算完成（DoD）。

### 当前修复快照验证记录（2026-08-24）

- `npm test`：46 项通过，0 项失败。
- `npm run typecheck`：退出码 0。
- `npm run build`：退出码 0。
- Rust 单元测试：255 项通过，0 项失败，3 项忽略。
- `git diff --check`：退出码 0。

以上是修复快照的已执行验证；Tauri 启动、Windows 安装包和真实客户端切换仍需在目标 Windows 环境进行人工验收。

## 10. 已知坑速查（按症状找原因）

| 症状 | 根因 | 处理 |
|---|---|---|
| 整窗冻结数秒~永久 | gh 命令同步跑主线程 / 无超时 | async+spawn_blocking + 看门狗 |
| 闪黑色命令行窗口 | spawn 控制台程序未加 CREATE_NO_WINDOW | 见 §6.5 |
| 装完 gh 仍"未检测到" | 进程 PATH 不刷新 | resolve_gh_path 探默认安装位 |
| 重启后账号消失 | 只识别单一平台标识 | 双平台识别（§5.2） |
| GitHub 配置丢失 | 非原子写半截 JSON | 原子写（§5.4） |
| 积分显示不准 | 整数解析漏 0.5 包 / 未过滤 status=0 | 浮点 + 过滤规则（§6.4） |
| WorkBuddy 显示“暂无数据” | token 失效、网络请求失败或上游字段变更 | 点击刷新积分；查看账号卡片局部错误；不要用签到接口代替查询（§6.8） |
| WorkBuddy 卡片内容重叠或空白过大 | 使用固定行高或嵌套指标卡片 | 保持自然高度与 `wb-status-line` 紧凑布局（§8） |
| 覆盖安装新版不生效 | 旧进程占用 | installer-hooks 杀旧进程 |
| 点外链没反应 | WebView 限制 | openExternalUrl |
| watcher 清掉设备密钥 | 回写未保留元数据 | preserve_account_metadata |
| 中文 Windows 注册表读不到 | cmd reg 编码问题 | winreg |

## 11. 环境备注

- 仅支持 Windows 构建目标（NSIS）；updater 已关闭（`createUpdaterArtifacts: false`，无 updater 插件配置）。
- 基线机器曾有 Rust 工具链 `0xC0000409` 崩溃与 MSVC link 问题：绕过方式是 safe-delete shim / 环境调整，详见 git 历史中 STAGE0/2 相关提交信息。
- `npm run tauri:dev`（不是 `tauri dev`）；构建前 `prepare-tauri.cjs` 会自动跑。

## 12. 发布

见 [release-process.md](release-process.md)（已按本 fork 的 Windows NSIS 流程重写）。杀软误报工单模板见 [false-positive-template.md](false-positive-template.md)。

## 13. 历史摘要（阶段 → commit，供追溯）

| 阶段 | commit | 交付 |
|---|---|---|
| 基线 | `e1ef55ce` | fork 自 cockpit-tools v1.3.16 |
| 1 | `4a2d304f` | 应用壳特化：改名、identifier、数据目录隔离 |
| 2 | `1a59a03b` | 安装路径发现 + winreg |
| 3 | `84d5fd29` | 完整账号快照导入（8 字段 + AES 落盘） |
| 4 | `48223bb6` | 事务切换 + 回滚 + 错误码 |
| 5 | `f177ff48` | 积分查询与展示 |
| 6 | `741ce86f` | GitHub Secrets 同步（TDD） |
| 7 | `7ab5459b` | 后台会话监测 watcher |
| 8 | `43dde028` | 清理/发布/安装包 |
| 后续 | `a6da900a`…`febf8647` | 积分浮点、×最小化到托盘、trae_cn 兼容、覆盖装杀旧进程、积分联网根因、日志按钮/重复账号/槽位删除 |
| 2026-08-15 | — | gh 引导弹窗（自动安装/PAT 登录）、性能修复（async 化/看门狗/无黑框）、外链修复、文档整合（本文档） |
| 2026-08-19 | — | WorkBuddy 真实积分/今日奖励/连续签到只读查询、多账号刷新、紧凑卡片布局与 NSIS 安装包更新 |

## 14. 文档索引

| 文档 | 用途 |
|---|---|
| `DEVELOPMENT.md`（本文档） | 唯一权威开发文档 |
| `release-process.md` | 发布流程（Windows NSIS） |
| `release-template.md` | Release Notes 模板 |
| `false-positive-template.md` | 杀软误报工单模板 |
| `DONATE.md` / `.en` / `.pt-br` | 三语赞助页（对外） |
| `images/` | README/DONATE 引用的截图与收款码 |
