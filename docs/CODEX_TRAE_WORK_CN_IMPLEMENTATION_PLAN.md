# TRAE Work CN 账号切换器实施计划

> **给接手开发的 Codex：**按本文逐任务执行。每个任务都使用测试驱动开发：先写失败测试，确认失败原因正确，再写最小实现。一次只完成一个任务，不跨阶段，不把真实凭证放进测试、日志或提交。

**目标：**基于 Cockpit Tools v1.3.16，在 Windows 上实现一个专用的 TRAE Work CN 四账号切换器：账号首次正常登录并导入后，可点击卡片免密码/免扫码切换，显示积分，并把最新签到凭证安全同步到 GitHub Secrets。

**架构：**内部继续复用上游 `TraePlatformKind::TraeSoloCn`、认证解密、加密账号存储、原子写、进程关闭、默认实例注入和启动逻辑。新增 Work CN 专用 DTO、安装检测、四槽视图、事务切换、积分展示、GitHub CLI 同步和后台会话监测；前端只调用专用的粗粒度 Tauri 命令，不直接接触 token 或分步操纵官方 storage。

**技术栈：**Tauri 2、Rust、React 19、TypeScript 5.8、Zustand、Node test、PowerShell、GitHub CLI。

## 全局约束

- 唯一开发目录：`C:\Users\10156\Desktop\脚本\切换应用\source`。
- 原版安装目录 `C:\Users\10156\AppData\Local\Cockpit Tools` 只读参考，禁止覆盖。
- 基线必须保持为 Cockpit Tools `v1.3.16` / `e1ef55ce9f158dd1ee9fd682cf8d9aa1b79601e8`。
- 当前开发分支：`codex/trae-work-cn-switcher`。
- 用户可见名固定为 `TRAE Work CN`；内部固定为 `TraePlatformKind::TraeSoloCn` 和 `trae_solo_cn`。
- 禁止新增 `trae_work_cn` 平台枚举，禁止复制并重写上游 TRAE 认证算法。
- 数字 `icube-dc` DeviceID 用于设备密钥；GitHub `x-device-id` 使用 `telemetry.devDeviceId` UUID。
- 本地禁止调用 `/trae/api/v2/ug/checkin_credits/claim`。
- 账号详情继续通过 `secure_account_storage` 加密；账号索引和前端 DTO 不含 token、refresh token 或私钥。
- 切换必须是一个后端事务命令；前端不得依次调用关闭、注入、启动。
- 测试不得读取真实 `%APPDATA%\TRAE SOLO CN`、`%APPDATA%\TRAE Work CN` 或真实 GitHub Secrets。
- 每阶段至少运行定向测试、`npm run typecheck`、`npm run build`、`cargo check -p cockpit-tools`。
- 未经用户明确同意，不 push、不发布、不触发真实签到 workflow。

---

## 一、文件结构和职责

以下结构是后续开发的目标边界。已有文件按现有模式修改，不做无关重构。

```text
src-tauri/src/models/work_cn.rs
  Work CN 安装信息、脱敏账号视图、快照校验、积分、GitHub 状态、切换错误 DTO。

src-tauri/src/modules/work_cn_installation.rs
  新旧名称、EXE、数据目录候选的纯解析和 Windows 安装检测。

src-tauri/src/modules/trae_account.rs
  继续负责官方 storage 的解密/解析、账号加密落盘、token 合并和 TraeSoloCn 注入。
  只在必须复用其私有加解密函数时增加小函数，不复制算法。

src-tauri/src/modules/work_cn_switch.rs
  切换锁、事务状态机、备份/提交/回滚和 UID 验证编排。

src-tauri/src/modules/work_cn_github.rs
  GitHub 配置校验、JWT exp 校验、gh runner、stdin secret 同步和脱敏。

src-tauri/src/modules/work_cn_session_watcher.rs
  只监测当前 Work CN storage 的 mtime，变化后回收最新会话并触发异步同步。

src-tauri/src/commands/work_cn.rs
  前端可调用的 Work CN 专用 Tauri commands；把内部完整账号映射成脱敏 DTO。

src-tauri/src/commands/work_cn_github.rs
  GitHub 配置、状态检查和手动同步 commands。

src/types/workCn.ts
  与 Rust camelCase DTO 一一对应的前端类型；不定义敏感字段。

src/services/workCnService.ts
  所有 `invoke()` 调用，固定平台细节不暴露给页面。

src/stores/useWorkCnStore.ts
  页面状态、加载/导入/切换/刷新/同步动作及 Busy 管理。

src/utils/workCnCredits.ts
  积分纯解析和显示模型。

src/utils/workCnErrors.ts
  后端错误码到简体中文可操作提示的纯映射。

src/components/work-cn/*
  状态栏、账号卡、添加账号、设置、日志入口。

src/pages/WorkCnSwitcherPage.tsx
  只负责组合组件，不直接解析 storage、token 或 gh 输出。
```

上游已有且必须优先复用的接口：

```rust
TraePlatformKind::TraeSoloCn
trae_account::import_from_local_for_platform(platform)
trae_account::inject_to_trae_for_platform(platform, account_id)
trae_account::inject_to_trae_at_path(storage_path, account_id)
trae_account::refresh_account_async(account_id)
trae_account::refresh_account_usage_only_async(account_id, storage_path)
trae_account::check_login_token(account_id)
trae_account::list_accounts_checked()
process::close_trae_platform_default(platform_id, timeout_secs)
process::is_trae_running_for_platform(platform)
atomic_write::write_bytes_atomic(path, content)
secure_account_storage::{serialize_account_file, deserialize_account_file}
trae_instance 默认实例设置和启动命令
```

上游当前已经具备的部分能力：

- 注册表名称匹配已包含 `TRAE Work CN` 和 `TRAE SOLO CN`；
- `iCubeAuthInfo://icube-dc:<数字ID>` 的设备密钥注入已有代码；
- refresh token、设备证明签名和 Token 刷新已有代码；
- CN 积分原始响应 `user_entitlement_pack_list` 已被保存；
- 运行中账号保护、原子写和加密账号详情已有代码。

因此后续实现应补缺口，不能重写这些能力。

---

## 二、所有阶段通用的验证脚本

每阶段在 PowerShell 中使用：

```powershell
Set-Location 'C:\Users\10156\Desktop\脚本\切换应用\source'
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
$env:TRAE_WORK_CN_SWITCHER_TEST_DATA_DIR = Join-Path $env:TEMP 'trae-work-cn-switcher-test'

npm run typecheck
npm run build
cargo check -p cockpit-tools
git diff --check
git status --short --branch
git diff --stat
```

若定向 Rust 测试触发 Go sidecar 构建，再增加：

```powershell
$env:GOPROXY='https://goproxy.cn,direct'
$env:GOSUMDB='sum.golang.google.cn'
```

不要把完整 `cargo test -p cockpit-tools` 的 `0xC0000409` 基线异常写成业务代码测试失败。仍要运行本阶段定向测试，并记录完整测试是否再次尝试、结果是什么。

---

## Task 1：完成并封存阶段 1 应用壳

**产物：**阶段 1 构建和开发启动证据、`docs/STAGE1_REPORT.md`，以及一个干净的阶段提交。

**文件：**

- 检查：当前全部未提交文件
- 测试：`scripts/tests/work-cn-stage1.test.mjs`
- 新建：`docs/STAGE1_REPORT.md`
- 仅在验证失败时修改对应阶段 1 文件

**接口：**本任务不新增业务接口；只确认应用壳、专用数据目录和构建配置可靠。

- [ ] **Step 1：检查工作区，禁止丢弃已有改动**

```powershell
git status --short --branch
git diff --stat
git diff --numstat -- package-lock.json
git diff -- src-tauri/src/modules/account.rs scripts/tauri-dev.cjs package-lock.json
```

预期：分支为 `codex/trae-work-cn-switcher`；lockfile 约为 `4 4`；账号目录常量为 `.trae_work_cn_switcher` 和 `.trae_work_cn_switcher_dev`。

- [ ] **Step 2：运行阶段 1 定向测试和构建**

```powershell
node --test scripts/tests/work-cn-stage1.test.mjs
npm run typecheck
npm run build
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
cargo check -p cockpit-tools
```

预期：Node 2/2 通过，其余退出码均为 0。Rust 的上游 warning 如实记录。

- [ ] **Step 3：实际启动开发版**

先确认没有旧进程：

```powershell
Get-CimInstance Win32_Process |
  Where-Object {
    $_.CommandLine -match '切换应用\\source' -or
    $_.ExecutablePath -like 'C:\Users\10156\Desktop\脚本\切换应用\source\target\debug\*'
  } |
  Select-Object ProcessId, Name, ExecutablePath, CommandLine
```

再启动：

```powershell
$env:GOPROXY='https://goproxy.cn,direct'
$env:GOSUMDB='sum.golang.google.cn'
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
npm run tauri dev
```

人工确认 Dev 标题、单一 Work CN 页面、4 个空槽、无其他平台导航，并确认 `%USERPROFILE%\.trae_work_cn_switcher_dev` 与原 Cockpit Tools 数据隔离。

- [ ] **Step 4：写阶段报告**

`docs/STAGE1_REPORT.md` 必须包含以下章节。表格中的命令、退出码和摘要必须在运行后按终端输出写入，不能提前声称成功：

```markdown
# 阶段 1 验收报告

- 验收日期：实际日期
- 分支：codex/trae-work-cn-switcher
- 基线：e1ef55ce

## 已实现

- 品牌、identifier、版本和窗口尺寸
- 仅 Work CN 应用壳和 4 个空槽
- updater endpoint 移除
- 专用正式/开发数据目录
- README 与 NOTICE 署名

## 自动验证

为以下四条命令逐条记录：完整命令、退出码、通过/失败、关键输出和运行时间。

1. node --test scripts/tests/work-cn-stage1.test.mjs
2. npm run typecheck
3. npm run build
4. cargo check -p cockpit-tools

## 人工验收

逐条记录窗口标题、页面与四槽、开发数据目录和控制台错误；每项只能写“通过”“失败”或“未执行”，并附观察证据。

## 已知基线问题

- 完整 cargo test 曾出现 rustc 0xC0000409
- rustfmt 当前是否安装及检查命令输出
- 上游 warning 的本次实际数量
```

- [ ] **Step 5：复核并向用户报告，暂不提交**

```powershell
git diff --check
git status --short
git diff --stat
```

把自动验证、人工验收、阶段报告路径、`git diff --stat` 和已知问题发给用户。阶段 1 首次接手必须在用户确认后才能提交。

- [ ] **Step 6：用户确认后提交**

```powershell
git add Cargo.lock README.md package-lock.json package.json scripts/tauri-dev.cjs src-tauri/Cargo.toml src-tauri/src/modules/account.rs src-tauri/tauri.conf.json src-tauri/tauri.dev.conf.json src/App.tsx NOTICE.md docs scripts/tests/work-cn-stage1.test.mjs src/pages/WorkCnSwitcherPage.tsx
git diff --cached --check
git commit -m "chore: specialize app shell for TRAE Work CN"
```

只有自动验证、人工启动和用户确认都满足后才能提交；不自动 push。

---

## Task 2：兼容 Work CN 新旧安装名称和路径

**产物：**页面能只读显示检测到的官方客户端版本、EXE、数据目录和是否使用旧路径。

**文件：**

- 新建：`src-tauri/src/models/work_cn.rs`
- 新建：`src-tauri/src/modules/work_cn_installation.rs`
- 新建：`src-tauri/src/commands/work_cn.rs`
- 修改：`src-tauri/src/modules/trae_account.rs`
- 修改：`src-tauri/src/modules/mod.rs`
- 修改：`src-tauri/src/models/mod.rs`
- 修改：`src-tauri/src/commands/mod.rs`
- 修改：`src-tauri/src/lib.rs`
- 新建：`src/types/workCn.ts`
- 新建：`src/services/workCnService.ts`
- 修改：`src/pages/WorkCnSwitcherPage.tsx`
- 测试：上述 Rust 模块内的 `#[cfg(test)]` 测试

**产生接口：**

```rust
pub struct WorkCnInstallation {
    pub installed: bool,
    pub executable_path: Option<String>,
    pub user_data_dir: Option<String>,
    pub storage_path: Option<String>,
    pub display_name: Option<String>,
    pub version: Option<String>,
    pub legacy_path: bool,
}

pub fn detect_work_cn_installation() -> Result<WorkCnInstallation, String>;

#[tauri::command]
pub fn get_work_cn_installation() -> Result<WorkCnInstallation, String>;

#[tauri::command]
pub fn set_work_cn_executable_path(executable_path: String)
    -> Result<WorkCnInstallation, String>;
```

- [ ] **Step 1：先为纯候选解析写失败测试**

在 `work_cn_installation.rs` 为以下纯函数先写测试。函数签名固定为：

```rust
pub(crate) fn work_cn_display_name_matches(value: &str) -> bool;
pub(crate) fn work_cn_executable_names() -> &'static [&'static str];

pub(crate) struct WorkCnDataDirCandidate {
    pub path: PathBuf,
    pub has_storage: bool,
    pub storage_modified_at: Option<SystemTime>,
    pub renamed_path: bool,
}

pub(crate) fn select_work_cn_data_dir(
    candidates: &[WorkCnDataDirCandidate],
) -> Option<PathBuf>;
```

测试断言必须逐项写清：

```text
work_cn_display_names_accept_new_and_legacy_names
  true: TraeWork CN (User), TRAE Work CN (User), TRAE SOLO CN

work_cn_display_names_reject_other_trae_products
  false: Trae CN (User), Trae (User), TRAE SOLO

work_cn_executable_names_include_new_and_legacy_names
  contains: TRAE Work CN.exe, TraeWork CN.exe, TRAE SOLO CN.exe

select_data_dir_prefers_newest_valid_storage
  输入两个 has_storage=true 候选，时间分别为 UNIX_EPOCH+1s 和 +2s
  断言返回 +2s 对应路径

select_data_dir_ignores_newer_directory_without_storage
  输入旧的 has_storage=true 与新的 has_storage=false
  断言返回旧的有效 storage 路径

select_data_dir_prefers_renamed_path_when_mtime_is_unavailable
  两个有效候选时间均为 None
  断言 renamed_path=true 的新名称目录胜出
```

运行：

```powershell
cargo test -p cockpit-tools work_cn_installation -- --nocapture
```

预期：因模块或函数尚不存在而失败。

- [ ] **Step 2：实现候选选择，不访问真实文件完成单元测试**

候选顺序包括：

```text
%APPDATA%\TRAE Work CN
%APPDATA%\TRAE SOLO CN
配置中 trae_solo_cn_app_path 推导出的安装位置
注册表 Uninstall 项的 DisplayIcon / InstallLocation / UninstallString
EXE 名：TRAE Work CN.exe、TraeWork CN.exe、TRAE SOLO CN.exe、Trae.exe、Electron.exe
```

选择数据目录的规则是：只比较包含 `User\globalStorage\storage.json` 的目录；多个有效目录时选择 storage 修改时间最新者；修改时间无法读取时按新名称优先；不删除任何目录。

- [ ] **Step 3：复用上游注册表和配置路径逻辑**

调整 `TraePlatformKind::TraeSoloCn` 的用户可见 `display_name()` 为 `TRAE Work CN`，但为路径选择新增候选函数，不能简单把 `app_support_dir_name()` 改成新名后丢失旧目录兼容。

`trae_account.rs` 中保留已有：

```rust
TraePlatformKind::TraeSoloCn
windows_uninstall_display_names(...)= ["TRAE SOLO CN", "TRAE Work CN"]
```

补充新 EXE 名候选，并把“默认 storage 路径”委托给安装检测选中的数据目录。

- [ ] **Step 4：实现脱敏命令和前端服务**

`src/types/workCn.ts`：

```ts
export interface WorkCnInstallation {
  installed: boolean;
  executablePath: string | null;
  userDataDir: string | null;
  storagePath: string | null;
  displayName: string | null;
  version: string | null;
  legacyPath: boolean;
}
```

`src/services/workCnService.ts`：

```ts
import { invoke } from '@tauri-apps/api/core';
import type { WorkCnInstallation } from '../types/workCn';

export const getWorkCnInstallation = () =>
  invoke<WorkCnInstallation>('get_work_cn_installation');

export const setWorkCnExecutablePath = (executablePath: string) =>
  invoke<WorkCnInstallation>('set_work_cn_executable_path', { executablePath });
```

在 `lib.rs` 的 handler 中注册两个命令。手动路径命令必须验证：路径存在、是普通文件、扩展名为 `.exe`、文件名属于 Work CN 新旧候选之一；验证成功后只更新上游 `UserConfig.trae_solo_cn_app_path` 并调用 `config::save_user_config`，然后重新检测并返回结果。页面加载时显示“检测中 / 已安装 / 未安装 / 检测失败”；失败提示提供“选择 EXE”的按钮，通过 Tauri dialog 选择文件后调用该命令，不把检测失败伪装为未安装。

- [ ] **Step 5：验证和人工验收**

```powershell
cargo test -p cockpit-tools work_cn_installation -- --nocapture
npm run typecheck
npm run build
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
cargo check -p cockpit-tools
npm run tauri dev
```

页面应显示目标机器实际的 EXE 和数据目录，并正确标记旧 `TRAE SOLO CN` 路径。检测只读，不导入账号、不启动客户端。

- [ ] **Step 6：提交**

```powershell
git add src-tauri/src/models/work_cn.rs src-tauri/src/modules/work_cn_installation.rs src-tauri/src/commands/work_cn.rs src-tauri/src/modules/trae_account.rs src-tauri/src/modules/mod.rs src-tauri/src/models/mod.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src/types/workCn.ts src/services/workCnService.ts src/pages/WorkCnSwitcherPage.tsx
git commit -m "feat: detect TRAE Work CN installations"
```

---

## Task 3：导入完整账号快照并固定四个槽位

**产物：**用户在官方客户端登录后点击“导入当前账号”，切换器将完整快照加密保存并显示脱敏账号卡；同 UID 更新原槽，第 5 个账号被拒绝。

**文件：**

- 修改：`src-tauri/src/models/trae.rs`
- 修改：`src-tauri/src/models/work_cn.rs`
- 修改：`src-tauri/src/modules/trae_account.rs`
- 修改：`src-tauri/src/commands/work_cn.rs`
- 修改：`src-tauri/src/lib.rs`
- 修改：`src/types/workCn.ts`
- 修改：`src/services/workCnService.ts`
- 新建：`src/stores/useWorkCnStore.ts`
- 新建：`src/components/work-cn/WorkCnAddAccountDialog.tsx`
- 新建：`src/components/work-cn/WorkCnAccountCard.tsx`
- 修改：`src/pages/WorkCnSwitcherPage.tsx`

**产生接口：**

```rust
pub struct LocalWorkCnDeviceSnapshot {
    pub auth_device_id: Option<String>,
    pub checkin_device_id: Option<String>,
    pub machine_id: Option<String>,
    pub device_key_pair: Option<serde_json::Value>,
}

pub struct WorkCnSnapshotValidation {
    pub valid_for_switch: bool,
    pub has_access_token: bool,
    pub has_refresh_token: bool,
    pub has_user_id: bool,
    pub has_auth_device_id: bool,
    pub has_device_private_key: bool,
    pub has_device_public_key: bool,
    pub has_checkin_device_id: bool,
    pub warnings: Vec<String>,
}

pub struct WorkCnAccountView {
    pub id: String,
    pub slot: u8,
    pub label: Option<String>,
    pub email: String,
    pub user_id: Option<String>,
    pub status: String,
    pub expires_at: Option<i64>,
    pub snapshot: WorkCnSnapshotValidation,
}

#[tauri::command]
pub async fn import_current_work_cn_account(label: Option<String>)
    -> Result<WorkCnAccountView, String>;

#[tauri::command]
pub fn list_work_cn_accounts() -> Result<Vec<WorkCnAccountView>, String>;
```

- [ ] **Step 1：扩展账号模型并验证向后兼容**

在 `TraeAccount` 和 `TraeImportPayload` 增加：

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub checkin_device_id: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub machine_id: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub auth_device_id: Option<String>,
```

`TraeImportPayload` 不是持久 DTO，无需 `serde` 属性，但字段名必须一致。为旧 JSON 反序列化测试确认三个字段缺失时均为 `None`。

- [ ] **Step 2：先写设备快照失败测试**

在 `trae_account.rs` 测试模块新增并逐项断言：

```text
work_cn_snapshot_extracts_numeric_auth_device_and_uuid_checkin_device
  storage 含 iCubeAuthInfo://icube-dc:1132918838145530
  storage 含 telemetry.devDeviceId=d6b8ac2e-f4d1-496d-a9a6-c9c7b4bd23e3
  断言 auth_device_id 是数字字符串
  断言 checkin_device_id 是 UUID，二者不相等

work_cn_snapshot_decrypts_both_device_keys
  使用现有测试加密函数生成只含 privateKeyPEM/publicKeyPEM 假值的密文
  断言两个字段都被合并到 trae_auth_raw.deviceKeyPair

work_cn_snapshot_rejects_device_uuid_as_auth_device_id
  storage 只有 telemetry.devDeviceId，没有 icube-dc 项
  断言 auth_device_id=None，checkin_device_id=该 UUID

work_cn_snapshot_validation_lists_every_missing_requirement
  构造仅有 access_token 的账号
  断言 valid_for_switch=false
  断言 refresh token、UID、数字设备 ID、公钥、私钥缺失项全部出现在 warnings
```

测试数据只能使用明显的假 token，例如 `test-access-token`，不得复制真实 storage。

- [ ] **Step 3：在现有 storage 解析链中合并设备字段**

实现：

```rust
fn extract_local_work_cn_device_snapshot(storage_root: &Value)
    -> LocalWorkCnDeviceSnapshot;

fn merge_work_cn_device_snapshot_into_payload(
    payload: &mut TraeImportPayload,
    snapshot: LocalWorkCnDeviceSnapshot,
);

pub fn validate_work_cn_account_snapshot(
    account: &TraeAccount,
) -> WorkCnSnapshotValidation;
```

提取规则：

- 遍历 `iCubeAuthInfo://icube-dc:` 前缀，后缀仅作为 `auth_device_id`；
- 使用现有 iCube 解密函数解析该项；
- 将密钥合并到已有 `trae_auth_raw.deviceKeyPair`，以便上游注入和 refresh 继续复用；
- `telemetry.devDeviceId` 原样保存为 `checkin_device_id`，并验证 UUID 格式；
- `telemetry.machineId` 保存为 `machine_id`；
- 绝不在日志输出密钥或 token。

- [ ] **Step 4：为四账号限制和槽位稳定性写失败测试**

测试使用 `TRAE_WORK_CN_SWITCHER_TEST_DATA_DIR` 临时目录：

```text
importing_same_uid_updates_existing_slot
  第一次导入 uid-1 后记录 slot
  第二次导入 uid-1 且使用新的假 access/refresh token
  断言账号总数仍为 1，slot 不变，加密详情解密后是新 token

importing_fifth_distinct_uid_is_rejected
  依次导入 uid-1 至 uid-4
  导入 uid-5
  断言返回明确的 ACCOUNT_LIMIT_REACHED，原四账号和槽位均不改变

encrypted_account_detail_contains_no_plain_token_or_private_key
  导入 test-access-token、test-refresh-token、test-private-key
  读取详情文件原始文本
  断言三个字符串均不存在；通过 secure_account_storage 解密后断言字段可恢复
```

槽位采用 1～4 的稳定映射，并确定使用专用非敏感索引文件：

```text
%USERPROFILE%\.trae_work_cn_switcher\work-cn-slots.json
```

结构固定为：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnSlotBinding {
    pub slot: u8,
    pub account_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkCnSlotIndex {
    pub version: u8,
    pub bindings: Vec<WorkCnSlotBinding>,
}
```

该文件只包含槽位、内部账号 ID 和备注，不含 email、UID、token、设备 ID 或密钥；使用 `atomic_write::write_string_atomic` 保存。同 UID 更新时保留原 slot；删除账号后该 slot 变为空；新账号使用最小空闲 slot。不要用数组当前排序隐式决定 GitHub 槽位，否则删除/重启后会错位。

- [ ] **Step 5：实现导入命令和脱敏视图**

导入命令固定调用 `TraePlatformKind::TraeSoloCn`。流程：检测 storage → 解析完整 payload → 按 UID 查重 → 检查四槽上限 → 加密 upsert → 返回 `WorkCnAccountView`。

`WorkCnAccountView` 不得包含下列字段：

```text
accessToken
refreshToken
traeAuthRaw
privateKeyPEM
publicKeyPEM
machineId
checkinDeviceId 完整值
```

若 UI 需要设备状态，只返回 `hasCheckinDeviceId: boolean` 或最多返回掩码。

- [ ] **Step 6：实现前端 store 和导入对话框**

Zustand 状态最少包含：

```ts
interface WorkCnState {
  accounts: WorkCnAccountView[];
  loading: boolean;
  importing: boolean;
  error: string | null;
  loadAccounts(): Promise<void>;
  importCurrentAccount(label?: string): Promise<void>;
}
```

页面固定渲染四槽。已占用槽显示账号脱敏信息和快照状态；空槽显示“先在官方客户端登录，再点击导入当前账号”。

- [ ] **Step 7：验证和提交**

```powershell
cargo test -p cockpit-tools work_cn_snapshot -- --nocapture
cargo test -p cockpit-tools importing_fifth_distinct_uid -- --nocapture
npm run typecheck
npm run build
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
cargo check -p cockpit-tools
git diff --check
```

人工验收只使用一个专门测试账号；导入前先退出原 Cockpit Tools。确认账号详情文件为密文，前端和日志无敏感字段。

```powershell
git add src-tauri/src/models/trae.rs src-tauri/src/models/work_cn.rs src-tauri/src/modules/trae_account.rs src-tauri/src/commands/work_cn.rs src-tauri/src/lib.rs src/types/workCn.ts src/services/workCnService.ts src/stores/useWorkCnStore.ts src/components/work-cn src/pages/WorkCnSwitcherPage.tsx
git commit -m "feat: import complete Work CN account snapshots"
```

---

## Task 4：实现事务式一键切换、验证和回滚

**产物：**一个后端命令完成完整切换；任何失败都恢复切换前 storage、账号绑定和运行状态；并发点击返回 Busy。

**文件：**

- 新建：`src-tauri/src/modules/work_cn_switch.rs`
- 修改：`src-tauri/src/models/work_cn.rs`
- 修改：`src-tauri/src/modules/trae_account.rs`
- 修改：`src-tauri/src/modules/mod.rs`
- 修改：`src-tauri/src/commands/work_cn.rs`
- 修改：`src-tauri/src/lib.rs`
- 修改：`src/types/workCn.ts`
- 修改：`src/services/workCnService.ts`
- 修改：`src/stores/useWorkCnStore.ts`
- 新建：`src/utils/workCnErrors.ts`
- 修改：`src/components/work-cn/WorkCnAccountCard.tsx`

**产生接口：**

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkCnErrorCode {
    AccountNotFound,
    SnapshotIncomplete,
    ClientNotInstalled,
    ClientCloseFailed,
    StorageBackupFailed,
    InjectFailed,
    LaunchFailed,
    VerifyTimeout,
    VerifyAccountMismatch,
    RollbackFailed,
    Busy,
}

pub struct WorkCnCommandError {
    pub code: WorkCnErrorCode,
    pub message: String,
    pub detail: Option<String>,
}

pub struct WorkCnSwitchResult {
    pub account: WorkCnAccountView,
    pub client_started: bool,
    pub github_sync_pending: bool,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn switch_work_cn_account(account_id: String)
    -> Result<WorkCnSwitchResult, WorkCnCommandError>;
```

- [ ] **Step 1：先定义可测试事务边界**

在 `work_cn_switch.rs` 定义小型 adapter，不引入新的进程实现：

```rust
type WorkCnIoFuture<'a, T> =
    Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub(crate) trait WorkCnSwitchIo {
    fn is_client_running(&self) -> bool;
    fn read_storage(&self, path: &Path) -> Result<Option<Vec<u8>>, String>;
    fn write_storage(&self, path: &Path, bytes: &[u8]) -> Result<(), String>;
    fn remove_storage(&self, path: &Path) -> Result<(), String>;
    fn close_client(&self, timeout_secs: u64) -> Result<(), String>;
    fn inject_account(&self, path: &Path, account_id: &str) -> Result<(), String>;
    fn launch_client<'a>(
        &'a self,
        account_id: &'a str,
    ) -> WorkCnIoFuture<'a, Result<(), String>>;
    fn read_current_user_id(&self, path: &Path) -> Result<Option<String>, String>;
}
```

该文件从 `std::future::Future` 和 `std::pin::Pin` 导入类型；生产 adapter 的启动实现使用 `Box::pin(async move { ... })` 包装现有异步默认实例启动命令，不新增 `async-trait` 依赖。生产 adapter 只包装上游 `process`、`trae_account`、`trae_instance` 和 `atomic_write`。测试 adapter 保存在同文件 `#[cfg(test)]` 模块中，通过枚举指定在哪一步失败，并以立即完成的 boxed future 返回启动结果。

- [ ] **Step 2：先写 10 个失败测试**

测试名和验收点：

```rust
snapshot_incomplete_does_not_close_client
close_failure_does_not_modify_storage
inject_failure_restores_original_bytes
launch_failure_restores_original_binding
verify_mismatch_rolls_back
verify_timeout_rolls_back
switch_lock_rejects_concurrent_request
switching_current_account_only_ensures_client_running
rollback_failure_reports_primary_and_rollback_errors
missing_original_storage_is_removed_on_rollback
```

再加一项日志脱敏测试，向错误中注入假 JWT 三段式字符串，断言序列化日志中不存在原文。

- [ ] **Step 3：实现唯一切换顺序**

事务必须严格执行：

1. `try_lock` 获取全局切换锁，失败返回 `Busy`；
2. 加载目标账号并运行 `validate_work_cn_account_snapshot`；
3. 检测安装和目标 storage；
4. 读取当前 storage，若 UID 与账号库匹配，则先把官方客户端最新 access/refresh token 合并并加密保存；
5. 记录切换前是否运行、storage 原始字节、当前账号绑定和默认实例设置；
6. 正常关闭客户端，20 秒超时；默认不强杀；
7. 调用现有 `inject_to_trae_at_path` 注入目标快照；
8. 更新 `provider_current_state` 和默认实例绑定；
9. 调用现有默认实例启动；
10. 最多 30 秒轮询 storage 中 UID，每 500 毫秒一次；
11. UID 一致才提交成功；
12. 失败时恢复原字节或删除本次新建 storage，恢复原绑定；
13. 若切换前客户端在运行，回滚后重新启动原账号；若原来未运行，则保持关闭；
14. 回滚也失败时同时返回主错误和回滚错误，不能覆盖主因。

原始 storage 备份保存在内存和专用临时事务文件中，成功后删除临时文件；不能把测试账号 storage 提交进仓库。

- [ ] **Step 4：前端只调用一个命令**

`workCnService.ts` 只暴露：

```ts
export const switchWorkCnAccount = (accountId: string) =>
  invoke<WorkCnSwitchResult>('switch_work_cn_account', { accountId });
```

账号卡点击后 store 设置全局 `switchingAccountId`。所有账号切换按钮在事务结束前禁用。`workCnErrors.ts` 按错误码显示“发生在哪一步、用户下一步、日志位置”，不要根据中文 `includes()` 判断。

- [ ] **Step 5：自动验证**

```powershell
cargo test -p cockpit-tools work_cn_switch -- --nocapture
npm run typecheck
npm run build
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
cargo check -p cockpit-tools
git diff --check
```

- [ ] **Step 6：双账号真实验收**

只在自动回滚测试全部通过后使用测试账号 A/B：

```text
A 导入 → B 导入 → A → B → A，连续至少 5 轮
每轮确认官方客户端实际 UID
全程不输入密码、不扫码
临时把 EXE 设置为不存在，确认启动失败后能恢复原账号
检查日志没有 token、refresh token 或私钥
```

真实验收前不要同时运行原 Cockpit Tools。若任一账号被服务端吊销，只标记该账号需要重新登录，不删除其他账号。

- [ ] **Step 7：提交**

```powershell
git add src-tauri/src/modules/work_cn_switch.rs src-tauri/src/models/work_cn.rs src-tauri/src/modules/trae_account.rs src-tauri/src/modules/mod.rs src-tauri/src/commands/work_cn.rs src-tauri/src/lib.rs src/types/workCn.ts src/services/workCnService.ts src/stores/useWorkCnStore.ts src/utils/workCnErrors.ts src/components/work-cn/WorkCnAccountCard.tsx
git commit -m "feat: switch Work CN accounts transactionally"
```

---

## Task 5：解析并显示 TRAE Work CN 积分

**产物：**每个账号卡显示总积分、已用积分、剩余积分或“无限积分”；没有有效字段时显示“暂无积分数据”；刷新积分不会执行签到。

**文件：**

- 修改：`src-tauri/src/models/work_cn.rs`
- 修改：`src-tauri/src/commands/work_cn.rs`
- 修改：`src/types/workCn.ts`
- 修改：`src/services/workCnService.ts`
- 修改：`src/stores/useWorkCnStore.ts`
- 新建：`src/utils/workCnCredits.ts`
- 新建：`src/utils/workCnCredits.test.ts`
- 修改：`src/components/work-cn/WorkCnAccountCard.tsx`

**说明：**上游 Rust 已经会查询并保存 `user_entitlement_pack_list` 到 `trae_usage_raw`，也已有通用 quota 解析。敏感原始响应只在 Rust 内解析并转换为 `WorkCnCreditsView`；前端 `workCnCredits.ts` 只把该脱敏 DTO 转成显示文案，不接收 `trae_usage_raw`。刷新命令复用 `refresh_account_usage_only_async`，不重复发明 HTTP 请求。

**产生接口：**

```ts
export interface WorkCnCredits {
  kind: 'finite' | 'unlimited' | 'unavailable';
  total: number | null;
  used: number | null;
  remaining: number | null;
}

export function formatWorkCnCredits(credits: WorkCnCredits): string;
```

Rust 脱敏视图增加：

```rust
pub struct WorkCnCreditsView {
    pub kind: String,
    pub total: Option<f64>,
    pub used: Option<f64>,
    pub remaining: Option<f64>,
}

#[tauri::command]
pub async fn refresh_work_cn_credits(account_id: String)
    -> Result<WorkCnAccountView, WorkCnCommandError>;
```

- [ ] **Step 1：写 Rust 积分解析失败测试**

在 `src-tauri/src/models/work_cn.rs` 或专用纯函数模块中固定实现：

```rust
pub(crate) fn parse_work_cn_credits(raw_usage: &serde_json::Value)
    -> WorkCnCreditsView;
```

测试 fixture 和断言：

```text
parses_one_finite_credit_pack
  credits_limit=1000, credits_amount=250
  finite / 1000 / 250 / 750

sums_multiple_finite_credit_packs
  1000/250 + 500/100
  finite / 1500 / 350 / 1150

any_active_unlimited_pack_makes_total_unlimited
  一个 credits_limit=-1 的活动包
  unlimited / total=None / used=所有活动包用量和 / remaining=None

missing_credit_fields_is_unavailable_instead_of_zero
  包存在但无 credits_limit
  unavailable / 三个数值均 None

hidden_or_inactive_packs_are_ignored
  is_hide=true 或 status 非活动值的包不参与计算

remaining_credit_never_becomes_negative
  total=100, used=120
  remaining=0
```

运行：

```powershell
cargo test -p cockpit-tools work_cn_credits -- --nocapture
```

预期：因函数尚不存在而失败。

- [ ] **Step 2：实现 Rust 最小积分解析**

只读取：

```text
user_entitlement_pack_list[*].entitlement_base_info.quota.credits_limit
user_entitlement_pack_list[*].usage.credits_amount
```

同时兼容响应外层的 `data`、`Result`、`result`、`payload` 和 `user_current_entitlement_list`。规则：

- 至少一个有效 `credits_limit = -1` → `unlimited`；
- 全部有效有限包求和；
- usage 缺失但 limit 有效时，used 视为 0；
- limit 全部缺失时 → `unavailable`，绝不显示总积分 0；
- 结果不得包含原始响应或 token。

- [ ] **Step 3：写前端格式化失败测试并实现**

`src/utils/workCnCredits.test.ts` 使用 Node 原生测试，不包含原始服务器 JSON，只测试脱敏 DTO：

```ts
assert.equal(
  formatWorkCnCredits({ kind: 'finite', total: 1000, used: 250, remaining: 750 }),
  '总积分 1000 · 已用 250 · 剩余 750',
);
assert.equal(
  formatWorkCnCredits({ kind: 'unlimited', total: null, used: 500, remaining: null }),
  '无限积分 · 已用 500',
);
assert.equal(
  formatWorkCnCredits({ kind: 'unavailable', total: null, used: null, remaining: null }),
  '暂无积分数据',
);
```

运行：

```powershell
node --test src/utils/workCnCredits.test.ts
```

预期：因实现不存在而失败。

- [ ] **Step 4：将积分加入后端脱敏账号视图**

`WorkCnAccountView` 只增加 `credits: WorkCnCreditsView`，不要把整个 `trae_usage_raw` 返回专用页面。`list_work_cn_accounts` 和 `refresh_work_cn_credits` 都调用同一个 Rust 解析函数，避免规则漂移。

刷新只调用：

```rust
trae_account::refresh_account_usage_only_async(account_id, selected_storage_path.as_deref())
```

禁止增加任何包含 `claim` 的 URL、函数或 command。

- [ ] **Step 5：账号卡显示和错误隔离**

显示文案：

```text
finite：总积分 N · 已用 N · 剩余 N
unlimited：无限积分 · 已用 N
unavailable：暂无积分数据
请求失败：积分暂时无法刷新，不影响账号切换
```

积分按钮单独有 loading 状态，不禁用其他账号的本地切换能力。

- [ ] **Step 6：静态扫描、测试和人工验收**

```powershell
cargo test -p cockpit-tools work_cn_credits -- --nocapture
node --test src/utils/workCnCredits.test.ts
rg -n "checkin_credits/claim|checkin.*claim" src src-tauri/src
npm run typecheck
npm run build
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
cargo check -p cockpit-tools
```

静态扫描预期：业务代码没有本地签到 claim。若开发文档或测试名称被扫描到，要明确区分，不能删除约束文档来伪造通过。

人工验收用浏览器网络日志或应用日志确认刷新积分只查询，不签到。

- [ ] **Step 7：提交**

```powershell
git add src-tauri/src/models/work_cn.rs src-tauri/src/commands/work_cn.rs src/types/workCn.ts src/services/workCnService.ts src/stores/useWorkCnStore.ts src/utils/workCnCredits.ts src/utils/workCnCredits.test.ts src/components/work-cn/WorkCnAccountCard.tsx
git commit -m "feat: display Work CN credit balances"
```

---

## Task 6：通过 GitHub CLI 安全同步四槽 Secrets

**产物：**用户配置 `owner/repo` 后，可以把每个槽的有效 JWT 和 `telemetry.devDeviceId` UUID 更新到两个固定 secret；secret 值只经 stdin 传递，GitHub 失败不影响本地切号。

**文件：**

- 修改：`src-tauri/src/models/work_cn.rs`
- 新建：`src-tauri/src/modules/work_cn_github.rs`
- 新建：`src-tauri/src/commands/work_cn_github.rs`
- 修改：`src-tauri/src/modules/mod.rs`
- 修改：`src-tauri/src/commands/mod.rs`
- 修改：`src-tauri/src/lib.rs`
- 修改：`src/types/workCn.ts`
- 修改：`src/services/workCnService.ts`
- 修改：`src/stores/useWorkCnStore.ts`
- 新建：`src/components/work-cn/WorkCnSettingsDialog.tsx`
- 修改：`src/components/work-cn/WorkCnAccountCard.tsx`
- 修改：`.gitignore`

**产生接口：**

```rust
pub struct WorkCnGitHubConfig {
    pub enabled: bool,
    pub repository: String,
}

pub struct WorkCnGitHubStatus {
    pub cli_installed: bool,
    pub authenticated: bool,
    pub repository: Option<String>,
    pub message: String,
}

pub struct WorkCnGitHubSyncResult {
    pub account_id: String,
    pub slot: u8,
    pub synced: bool,
    pub pending: bool,
    pub updated_secret_names: Vec<String>,
    pub message: String,
}

pub(crate) trait WorkCnCommandRunner {
    fn run(&self, program: &str, args: &[String], stdin: Option<&[u8]>)
        -> Result<CommandOutput, String>;
}
```

commands：

```rust
get_work_cn_github_status()
save_work_cn_github_config(config)
sync_work_cn_github_account(account_id)
```

- [ ] **Step 1：先写配置和 runner 失败测试**

在 `work_cn_github.rs` 的测试模块覆盖：

```rust
repository_accepts_owner_slash_repo
repository_rejects_urls_spaces_and_extra_segments
secret_names_are_derived_from_stable_slot
github_args_never_contain_secret_value
github_secret_value_is_sent_through_stdin
expired_jwt_is_not_synced
missing_checkin_uuid_is_not_synced
numeric_auth_device_id_is_never_used_as_checkin_device_id
first_secret_failure_is_not_reported_as_success
stderr_is_redacted_before_returning
```

默认 secret 名只能由槽位生成：

```rust
fn secret_names_for_slot(slot: u8) -> Result<(String, String), String> {
    // slot 1 -> TRAE1_TOKEN, TRAE1_DEVICE_ID
}
```

不允许用户输入任意 secret 名，减少配置和注入风险。

- [ ] **Step 2：实现 JWT exp 和 UUID 校验**

只解析 JWT payload 的 `exp`，不验证签名、不记录 payload。同步条件：

```text
slot 在 1～4
access_token 非空
JWT exp > 当前时间 + 60 秒
checkin_device_id 是合法 UUID
GitHub 配置 enabled
repository 符合 owner/repo
gh auth status 成功
```

校验失败返回“待同步/需要重新登录”，不删除账号、不阻止本地切换。

- [ ] **Step 3：实现 gh runner，secret 只走 stdin**

生产实现调用：

```text
gh auth status
gh secret set TRAE1_TOKEN --repo owner/repo
gh secret set TRAE1_DEVICE_ID --repo owner/repo
```

要求：

- `Command::new("gh")`，Windows 自动解析 `gh.exe`；
- 参数数组中只能有 secret 名和 repository，绝不能有值；
- stdin 写完立即关闭；
- stdout/stderr 经过统一脱敏后才进入错误对象；
- 日志只记录账号 ID、槽位和 secret 名；
- 不在应用配置中保存 PAT。

- [ ] **Step 4：保存非敏感 GitHub 配置**

配置文件放在专用应用数据目录，例如：

```text
%USERPROFILE%\.trae_work_cn_switcher\work-cn-github.json
```

只保存 `enabled` 和 `repository`。使用 `atomic_write::write_string_atomic`。`.gitignore` 增加：

```gitignore
# Local Work CN credentials and configuration
.env
.env.*
!.env.example
*.token
*.secret
work-cn-accounts/
work-cn-config.local.json
trae_accounts.json
gh_token.txt
*.storage.backup
*.storage.tmp
```

- [ ] **Step 5：接入设置 UI 和非阻塞同步**

设置页显示：

- gh 是否安装；
- gh 是否登录；
- repository；
- 4 个槽对应的两个 secret 名；
- 每个账号最后同步状态。

切换成功后可以异步触发同步，但 `switch_work_cn_account` 的本地成功结果不能因 GitHub 失败改成失败；只把 `githubSyncPending` 设为 true 并提示可手动重试。

- [ ] **Step 6：自动验证**

```powershell
cargo test -p cockpit-tools work_cn_github -- --nocapture
npm run typecheck
npm run build
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
cargo check -p cockpit-tools
git diff --check
```

fake runner 必须证明参数中没有假 token，stdin 中才有值，错误返回中也没有值。

- [ ] **Step 7：经用户确认后做真实 GitHub 验收**

真实 secret 写入属于外部状态变更，执行前再次向用户确认 repository。优先使用测试仓库或临时 secret 名。用户确认后：

```powershell
gh auth status
gh secret list --repo lk1015646426/daily-checkin
```

只有用户明确要求时才触发 workflow：

```powershell
gh workflow run daily-checkin.yml --repo lk1015646426/daily-checkin
gh run list --repo lk1015646426/daily-checkin --limit 5
```

不得尝试读取 secret 值；GitHub 本来也不会返回明文。

- [ ] **Step 8：提交**

```powershell
git add src-tauri/src/models/work_cn.rs src-tauri/src/modules/work_cn_github.rs src-tauri/src/commands/work_cn_github.rs src-tauri/src/modules/mod.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src/types/workCn.ts src/services/workCnService.ts src/stores/useWorkCnStore.ts src/components/work-cn/WorkCnSettingsDialog.tsx src/components/work-cn/WorkCnAccountCard.tsx .gitignore
git commit -m "feat: sync Work CN credentials to GitHub"
```

---

## Task 7：后台回收官方客户端轮换的最新会话

**产物：**官方客户端运行期间 token 变化后，切换器在不干扰客户端的前提下更新当前账号加密快照，并将对应槽标记为已同步或待同步。

**文件：**

- 新建：`src-tauri/src/modules/work_cn_session_watcher.rs`
- 修改：`src-tauri/src/modules/trae_account.rs`
- 修改：`src-tauri/src/modules/work_cn_switch.rs`
- 修改：`src-tauri/src/modules/mod.rs`
- 修改：`src-tauri/src/lib.rs`
- 修改：`src-tauri/src/models/work_cn.rs`
- 修改：`src/types/workCn.ts`
- 修改：`src/stores/useWorkCnStore.ts`
- 新建：`src/components/work-cn/WorkCnStatusBanner.tsx`
- 修改：`src/pages/WorkCnSwitcherPage.tsx`

**产生接口：**

```rust
pub struct WorkCnWatcherStatus {
    pub running: bool,
    pub current_account_id: Option<String>,
    pub last_checked_at: Option<i64>,
    pub last_synced_at: Option<i64>,
    pub last_error: Option<String>,
}

pub(crate) fn sync_current_work_cn_session_from_storage(
    storage_path: &Path,
) -> Result<Option<String>, String>; // 返回有变化的 account_id
```

- [ ] **Step 1：先写会话合并纯测试**

覆盖：

```rust
unchanged_mtime_skips_storage_decryption
changed_storage_with_matching_uid_updates_tokens
changed_storage_with_unknown_uid_does_not_overwrite_any_account
stale_storage_token_does_not_replace_newer_saved_token
switch_lock_causes_watcher_to_skip_cycle
repeated_failures_use_bounded_backoff
watcher_never_calls_checkin_claim
```

测试用假时钟和临时文件，不 sleep 60 秒，不启动真实后台线程。

- [ ] **Step 2：暴露安全的当前 storage 同步函数**

复用 `trae_account.rs` 中现有 `sync_account_tokens_from_storage_path` 思路，但新增公开的 Work CN 包装函数：

1. 解析 `TraeSoloCn` 当前 storage；
2. 用 UID 匹配账号库；
3. 只在 token 版本更新或到期时间更新时写入；
4. 使用加密账号详情和原子索引写；
5. 不把原始 payload 返回给 watcher；
6. 不对未知 UID 新建账号，导入必须由用户主动执行。

- [ ] **Step 3：实现 60 秒监测和退避**

规则：

```text
基础间隔 60 秒
storage mtime 未变化：只更新 last_checked_at
切换锁占用：跳过，不记录错误
一次失败：下一次仍 60 秒
连续失败：120、240、最大 600 秒
一次成功后恢复 60 秒
Token 变化：异步触发该槽 GitHub 同步
GitHub 失败：本地更新仍成功，状态为 pending
```

Tauri 启动时只创建一个 watcher。开发热重载不得重复创建多个循环。

- [ ] **Step 4：前端状态栏**

状态栏展示：客户端安装/运行状态、当前账号、最近会话同步时间、GitHub 待同步数量。错误只显示可操作摘要，详细脱敏信息放日志。

- [ ] **Step 5：自动验证和四账号短期验收**

```powershell
cargo test -p cockpit-tools work_cn_session_watcher -- --nocapture
npm run typecheck
npm run build
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
cargo check -p cockpit-tools
```

准备 A/B/C/D 测试账号后验证：

```text
A → B → C → D → A
快速连续点击返回 Busy
客户端未运行时能启动目标账号
客户端运行时正常关闭再启动
断网时本地快照仍可切换
GitHub 未登录时只标记待同步
管理器重启后槽位不变
```

- [ ] **Step 6：提交**

```powershell
git add src-tauri/src/modules/work_cn_session_watcher.rs src-tauri/src/modules/trae_account.rs src-tauri/src/modules/work_cn_switch.rs src-tauri/src/modules/mod.rs src-tauri/src/lib.rs src-tauri/src/models/work_cn.rs src/types/workCn.ts src/stores/useWorkCnStore.ts src/components/work-cn/WorkCnStatusBanner.tsx src/pages/WorkCnSwitcherPage.tsx
git commit -m "feat: complete four-account Work CN workflow"
```

---

## Task 8：长期验收、清理和 Windows 发布

**产物：**在干净 Windows 用户环境可安装的当前用户安装包；无开发凭证；保留上游许可；核心流程经过长期验收。

**文件：**

- 修改：`src-tauri/tauri.conf.json`
- 修改：`src-tauri/icons/*`（使用新项目图标时）
- 修改：`README.md`
- 修改：`NOTICE.md`
- 新建：`docs/FINAL_ACCEPTANCE_REPORT.md`
- 按可达性逐个删除确认无用的前端页面/后端 command，具体清单必须在删除前由 `rg` 和编译证据生成

- [ ] **Step 1：先执行 3 天长期验证，不急于删代码**

每天记录：

```text
日期和应用版本
A/B/C/D 各切换一次的结果
是否要求重新扫码/密码
官方客户端是否轮换 token
GitHub Secrets 同步状态
GitHub Actions 签到结果
积分查询结果
任何错误码和脱敏日志位置
```

服务端主动吊销不算切换器自动失效，但必须只影响对应账号，并提示重新登录。

- [ ] **Step 2：生成可达性清单后小批量清理**

先运行：

```powershell
rg -n "from './pages|from \"./pages|commands::|pub mod" src/App.tsx src src-tauri/src/lib.rs src-tauri/src/commands/mod.rs src-tauri/src/modules/mod.rs
```

每批只删除一组明确不再可达的页面或 command，随后立即运行 TypeScript 和 Rust 检查。禁止按 `Trae` 全局批量删除。必须保留：

```text
secure_account_storage
atomic_write
logger
process
provider_current_state
trae_account
trae_instance 默认实例链
Work CN 安装、切换、GitHub、watcher 模块
```

- [ ] **Step 3：新图标和安装配置验收**

要求：

```text
productName = TRAE Work CN 账号切换器
identifier = com.lk.trae-work-cn-switcher
installMode = currentUser
createUpdaterArtifacts = false
没有上游 updater endpoint
安装包不含 .trae_work_cn_switcher 数据目录
卸载默认不删除账号库
提供需要二次确认的“清除本地凭证”功能
```

图标必须是新项目资产或有明确授权，不能误用 TRAE 官方商标造成官方出品的误导。

- [ ] **Step 4：先实现并测试“清除本地凭证”**

在 `src-tauri/src/commands/work_cn.rs` 新增：

```rust
#[tauri::command]
pub fn clear_work_cn_local_credentials(confirm_text: String)
    -> Result<(), WorkCnCommandError>;
```

只有 `confirm_text == "清除本地凭证"` 时才执行。命令必须先取得切换锁，并且仅删除专用应用数据目录中的：

```text
trae_accounts.json
trae_accounts/*.json
work-cn-slots.json
work-cn-github.json
Work CN 专用临时事务文件
```

必须保留用户的通用非敏感应用设置，且绝对不能删除：

```text
%APPDATA%\TRAE Work CN
%APPDATA%\TRAE SOLO CN
官方 storage.json
官方客户端 EXE
C:\Users\10156\AppData\Local\Cockpit Tools
```

先写以下测试：

```rust
clear_credentials_rejects_wrong_confirmation
clear_credentials_removes_only_paths_below_switcher_data_dir
clear_credentials_never_touches_official_work_cn_data
clear_credentials_is_busy_during_account_switch
clear_credentials_handles_missing_files_idempotently
```

前端设置对话框必须二次确认并要求用户输入完整确认文字；成功后清空页面账号状态。测试使用 `TRAE_WORK_CN_SWITCHER_TEST_DATA_DIR`，不得在测试中调用真实用户目录。

- [ ] **Step 5：最终自动构建**

```powershell
npm ci
node --test scripts/tests/work-cn-stage1.test.mjs
node --test src/utils/workCnCredits.test.ts
npm run typecheck
npm run build
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
cargo check -p cockpit-tools
cargo test -p cockpit-tools work_cn -- --nocapture
npm run tauri build
git diff --check
```

如果完整 `cargo test -p cockpit-tools` 的工具链问题已解决，再运行并记录；仍出现 `0xC0000409` 时准确写入报告，不可伪造通过。

- [ ] **Step 6：干净环境安装验收**

在另一个 Windows 用户或干净虚拟机：

1. 安装官方 TRAE Work CN；
2. 安装切换器；
3. 确认安装包不自带任何账号；
4. 导入 2 个测试账号；
5. 重启 Windows；
6. 无密码来回切换；
7. 卸载切换器，确认官方客户端仍能启动；
8. 重装后验证保留数据策略；
9. 执行“清除本地凭证”，确认只删除切换器账号库，不删除官方 Work CN 用户目录。

- [ ] **Step 7：写最终验收报告和提交**

`docs/FINAL_ACCEPTANCE_REPORT.md` 必须包含构建产物绝对路径、SHA-256、测试命令和退出码、四账号矩阵、回滚演练、断网、GitHub 未登录、服务端吊销、3 天记录和所有未解决问题。

```powershell
git add README.md NOTICE.md src-tauri/tauri.conf.json src-tauri/icons docs/FINAL_ACCEPTANCE_REPORT.md
git commit -m "chore: prepare Work CN switcher release"
```

发布、push 或创建 GitHub Release 必须再次取得用户明确授权。

---

## 三、阶段验收矩阵

| 阶段 | 自动验收 | 人工验收 | 允许进入下一阶段的条件 |
|---|---|---|---|
| 1 应用壳 | Node、TS、Vite、Rust check | Dev 窗口和专用目录 | 全部成功且有 STAGE1_REPORT |
| 2 安装检测 | 候选选择 Rust 测试 | 显示实际 EXE/目录/版本 | 新旧路径识别正确且只读 |
| 3 快照导入 | 设备字段、加密、四槽测试 | 导入 1 个测试账号 | 完整快照有效、UI 无敏感字段 |
| 4 事务切换 | 10 个失败/回滚测试 | A/B 连续 5 轮和启动失败演练 | UID 验证和回滚均通过 |
| 5 积分 | 普通/多包/无限/缺失测试 | 账号卡显示且无 claim | 查询失败不影响切号 |
| 6 GitHub | fake runner、stdin、脱敏测试 | 经授权的测试仓库同步 | 8 个 secret 名正确，值不泄露 |
| 7 四账号 | watcher/并发/退避测试 | A/B/C/D 矩阵 | 槽位稳定、轮换 token 被回收 |
| 8 发布 | 全套构建和安装包 | 干净环境 + 3 天验证 | 报告完整且无凭证进入安装包 |

---

## 四、新对话中的正确开发方式

不要把本文整份交给模型后只说“一次做完”。正确方式是每次只给一个明确任务，并要求先报告检查结果。

### 第一个新对话

复制 `docs/CODEX_TRAE_WORK_CN_DEVELOPMENT_HANDOFF.md` 第 14 节的启动提示词。让新对话只完成 Task 1 阶段 1 验收。它报告成功后，由用户决定是否提交。

### 后续每个阶段的提示词模板

```text
继续开发 C:\Users\10156\Desktop\脚本\切换应用\source。
先阅读：
- docs/CODEX_TRAE_WORK_CN_DEVELOPMENT_HANDOFF.md
- docs/CODEX_TRAE_WORK_CN_IMPLEMENTATION_PLAN.md
- 上一阶段报告

当前只执行 Implementation Plan 的 Task N，不提前实现 Task N+1。
先检查 git status、当前分支和上一提交，再列出本任务将修改与不会修改的文件。
必须先写定向失败测试并展示预期失败，再实现最小代码。
完成后运行任务中列出的定向测试、npm run typecheck、npm run build、cargo check -p cockpit-tools 和人工验收。
报告实际命令、退出码、diff stat、风险和未解决问题。不要擅自 push、发布或改真实 GitHub Secrets。
始终使用简体中文，不输出任何真实 token、refresh token、私钥或 gh stdin。
```

### 每阶段结束时用户检查什么

用户只需检查五件事：

1. 模型有没有越过当前阶段；
2. 有没有真实测试命令和退出码；
3. 有没有明确区分自动测试与人工验收；
4. 有没有泄露或读取真实凭证；
5. 是否先报告再提交/push/外部写入。

---

## 五、禁止使用的“捷径”

- 只复制 `storage.json` 或只替换 JWT；
- 把四个账号保存为四个明文 JSON；
- 把官方客户端目录完整复制四份；
- 为 Work CN 再造一套认证、加密或进程管理；
- 前端串联三个 Tauri 命令完成切换；
- 默认 `taskkill /F` 强杀客户端；
- 本地执行签到 claim；
- 把 GitHub token/secret 放入命令参数；
- 用数字 `icube-dc` ID 作为 `x-device-id`；
- 自动运行 `npm audit fix` 升级固定基线；
- 在真实 Cockpit Tools 安装目录直接开发；
- 在阶段 2～7 期间大规模删除上游模块；
- 未做 UID 验证就显示“切换成功”；
- 启动失败后只提示错误、不回滚。

---

## 六、计划完成判断

本文所有任务完成不自动等于产品可以公开发布。公开发布至少还需要：

- 四个账号均为用户授权测试账号；
- 完整 3 天验证；
- 干净 Windows 用户安装验证；
- 安装包敏感文件扫描；
- 许可、NOTICE 和图标授权检查；
- 用户明确同意 push、Release 和分发。

开发中的核心不变量是：

> 每次切换前先保存当前官方客户端刚轮换的最新会话；完整注入目标身份；启动后验证 UID；失败恢复切换前全部状态。积分和 GitHub 同步都是附属能力，不能破坏这个事务边界。
