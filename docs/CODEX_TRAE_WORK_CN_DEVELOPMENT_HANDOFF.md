# TRAE Work CN 账号切换器：Codex 开发交接总文档

> 最后核对日期：2026-08-13  
> 项目状态：阶段 0 已完成；阶段 1 已写入工作区但尚未提交，仍需最终构建和开发版启动验收；阶段 2 及以后尚未实现。  
> 本文用途：换到新的 Codex 对话后，让新对话不依赖旧聊天记录即可继续开发。

---

## 1. 新对话先做什么

新对话不要重新克隆项目，不要从零设计，也不要直接进入账号切换核心开发。按以下顺序接手：

1. 阅读本文；
2. 阅读 `docs/TRAE_WORK_CN_SWITCHER_GLM52_DEVELOPMENT_GUIDE.md`；
3. 阅读 `docs/STAGE0_BASELINE_REPORT.md`；
4. 阅读 `docs/CODEX_TRAE_WORK_CN_IMPLEMENTATION_PLAN.md`；
5. 检查当前分支和未提交差异；
6. 完成“阶段 1 最终验收”；
7. 阶段 1 验收通过后再提交；
8. 从阶段 2 开始按测试驱动方式逐阶段开发。

不要一次跨越多个阶段。每个阶段必须形成可单独验证、可单独回退的提交。

---

## 2. 项目要解决的真实问题

用户共有 4 个 TRAE Work CN 账号，希望实现：

- 每个账号只在官方 TRAE Work CN 中正常登录一次；
- 切换器保存该账号的完整本地认证快照；
- 以后点击账号卡片即可切换账号并启动官方客户端；
- 正常情况下不再输入密码，也不再扫码；
- 主界面显示当前账号和积分；
- 本地应用不执行签到，签到继续由 GitHub Actions 执行；
- 本地应用把最新 JWT 和正确的设备 UUID 同步到 GitHub Secrets；
- 日常操作只需要打开这个切换器，点击 4 个账号中的一个。

这里的“登录一次后长期切换”并不等于承诺永久有效。只要服务端没有吊销会话、账号没有触发风控、官方认证格式没有发生不兼容变化，就可以持续复用并轮换登录态。若服务端主动吊销 refresh token，那个账号仍需重新正常登录一次。软件必须把这种状态显示为“需要重新登录”，不能宣称绝对永久免登录。

---

## 3. 路径、仓库和禁止区域

### 3.1 唯一开发目录

```text
C:\Users\10156\Desktop\脚本\切换应用\source
```

所有源代码、测试、文档和 Git 操作都在这个目录中进行。

### 3.2 已安装的原版程序，仅作对照

```text
C:\Users\10156\AppData\Local\Cockpit Tools
```

严禁在这个目录中直接开发、替换文件或存放测试数据。不要覆盖用户已经安装的原版程序。

### 3.3 官方客户端和真实数据

目标机器当前可能同时保留新旧命名：

```text
注册表显示名：TraeWork CN (User)
已知真实 EXE：D:\联想软件下载内容\TRAE SOLO CN\TRAE SOLO CN.exe
已知真实数据目录：%APPDATA%\TRAE SOLO CN
新命名候选目录：%APPDATA%\TRAE Work CN
```

测试不能读取或修改真实 `storage.json`。涉及 storage 的测试必须使用临时目录和模拟数据。

---

## 4. 固定源码基线

```text
上游仓库：jlcodes99/cockpit-tools
上游标签：v1.3.16
标签对象：e0c0292ff08476b7b02a4be4a46a9f5284a223d9
源码提交：e1ef55ce9f158dd1ee9fd682cf8d9aa1b79601e8
当前分支：codex/trae-work-cn-switcher
```

当前 `HEAD` 仍是上游提交 `e1ef55ce`。阶段 1 的内容都还是未提交工作区修改，因此新对话不能执行 reset、checkout 丢弃或清理未跟踪文件。

---

## 5. 最重要的技术判断

### 5.1 TRAE Work CN 是用户可见新名称，不是新的内部平台

必须继续使用：

```rust
TraePlatformKind::TraeSoloCn
```

以及：

```text
trae_solo_cn
```

用户界面显示：

```text
TRAE Work CN
```

禁止新增 `TraeWorkCn` 枚举，禁止复制一套 `work_cn_account.rs` 重写认证。需要做的是让现有 `TraeSoloCn` 同时兼容新旧显示名、EXE 名称和数据目录。

### 5.2 不是多实例并行登录

需求是一个官方客户端、一个当前账号、4 个可切换快照。不是同时打开 4 个 TRAE 窗口。不要把上游 `TraeInstancesPage` 当主页面。

### 5.3 不能只保存 JWT

可靠切换至少需要保存并恢复：

- access token；
- refresh token；
- token 到期时间及账号元数据；
- `iCubeAuthInfo://icube.cloudide`；
- `iCubeAuthInfo://icube-dc:<数字ID>` 对应的设备密钥材料；
- `iCubeAuthInfo://usertag`；
- `telemetry.devDeviceId`；
- `telemetry.machineId`；
- 能保证官方客户端后续 refresh 的相关字段。

### 5.4 两种 Device ID 不能混用

```text
iCubeAuthInfo://icube-dc:<数字ID>
```

其中的数字 ID 用于定位官方客户端里的设备密钥。

```text
telemetry.devDeviceId
```

这是 UUID，GitHub Actions 签到请求的 `x-device-id` 使用它。GitHub 的 `TRAE{N}_DEVICE_ID` 必须同步 UUID，不能同步前面的数字 ID。

### 5.5 本地永远不执行签到 claim

本地代码禁止请求：

```text
/trae/api/v2/ug/checkin_credits/claim
```

本地只负责查询积分、保存和切换登录态、同步 GitHub Secrets。签到 claim 继续由 GitHub Actions 负责，以免重复签到和职责冲突。

### 5.6 桌面实际代码位置

真正参与 Tauri 桌面编译的是：

```text
src-tauri/src/modules
src-tauri/src/commands
```

不要只改 `crates/cockpit-core`。如果确实需要改共享 crate，必须同时确认桌面调用链实际使用它。

---

## 6. 目标架构与数据流

```text
官方 TRAE Work CN 首次正常登录
        ↓
读取官方 storage.json（只在导入/同步时）
        ↓
解析完整认证快照 + 两种 Device ID
        ↓
使用上游 secure_account_storage 加密保存账号详情
        ↓
固定映射到 1～4 号槽位
        ↓
点击某账号
        ↓
同步“当前账号”刚被官方客户端轮换的最新 token
        ↓
正常关闭官方客户端
        ↓
备份原 storage 原始字节和原账号绑定
        ↓
调用上游 TraeSoloCn 注入逻辑写入目标完整快照
        ↓
调用上游默认实例逻辑启动官方客户端
        ↓
验证实际 UID 与目标 UID 一致
        ↓
成功：更新当前账号、积分和 GitHub Secrets
失败：原子恢复旧 storage、旧账号绑定和原运行状态
```

所有切换动作必须由单个后端事务命令完成。前端不得自行串联“关闭、写文件、启动”三个命令，因为中途失败会留下半切换状态。

---

## 7. 当前实际完成状态

### 7.1 阶段 0：已完成

- 固定上游 `v1.3.16`；
- 创建分支 `codex/trae-work-cn-switcher`；
- `npm install` 成功；
- 原始基线 `npm run typecheck` 成功；
- Go sidecar 已生成；
- `cargo check -p cockpit-tools` 成功；
- 原版 `npm run tauri dev` 曾成功启动。

完整记录见 `docs/STAGE0_BASELINE_REPORT.md`。

### 7.2 阶段 1：代码已写，尚未正式验收和提交

已经写入工作区：

- 产品名改为 `TRAE Work CN 账号切换器`；
- npm 包名改为 `trae-work-cn-switcher`；
- 应用版本改为 `0.1.0`；
- Tauri identifier 改为 `com.lk.trae-work-cn-switcher`；
- 开发 identifier 为 `com.lk.trae-work-cn-switcher.dev`；
- 主窗口为 960×680，最小 820×560；
- 上游 updater endpoint 配置已移除，更新构建产物关闭；
- `App.tsx` 只渲染 `WorkCnSwitcherPage`；
- 页面显示客户端检测占位、4 个空账号槽、设置和日志按钮；
- 新增 `NOTICE.md` 并保留上游作者署名；
- npm lockfile 根元数据与 package.json 对齐；
- 应用数据目录与原 Cockpit Tools 隔离。

阶段 1 额外安全修复：

```text
正式目录：%USERPROFILE%\.trae_work_cn_switcher
开发目录：%USERPROFILE%\.trae_work_cn_switcher_dev
正式覆盖变量：TRAE_WORK_CN_SWITCHER_DATA_DIR
测试覆盖变量：TRAE_WORK_CN_SWITCHER_TEST_DATA_DIR
profile 变量：TRAE_WORK_CN_SWITCHER_PROFILE
```

debug 构建通过 `cfg!(debug_assertions)` 强制使用开发数据目录，防止开发版误读正式账号库。

阶段 1 定向回归测试：

```text
scripts/tests/work-cn-stage1.test.mjs
```

已知最近一次结果为 2 个测试全部通过。新对话仍须重新运行，不能只引用旧结果。

### 7.3 尚未实现

- Work CN 新旧安装识别；
- 真实客户端检测 UI；
- 4 个完整账号快照导入；
- 第 5 个账号限制；
- 一键事务切换、UID 验证和回滚；
- 积分查询和展示；
- GitHub CLI 检测和 8 个 Secrets 同步；
- 后台会话同步；
- 四账号真实验收；
- 新图标、清理无关后端、安装包发布。

---

## 8. 当前未提交文件

当前工作区包含以下已修改文件：

```text
Cargo.lock
README.md
package-lock.json
package.json
scripts/tauri-dev.cjs
src-tauri/Cargo.toml
src-tauri/src/modules/account.rs
src-tauri/tauri.conf.json
src-tauri/tauri.dev.conf.json
src/App.tsx
```

当前未跟踪文件：

```text
NOTICE.md
docs/00_如何使用GLM5.2逐阶段开发.md
docs/STAGE0_BASELINE_REPORT.md
docs/TRAE_WORK_CN_SWITCHER_GLM52_DEVELOPMENT_GUIDE.md
docs/CODEX_TRAE_WORK_CN_DEVELOPMENT_HANDOFF.md
docs/CODEX_TRAE_WORK_CN_IMPLEMENTATION_PLAN.md
scripts/tests/work-cn-stage1.test.mjs
src/pages/WorkCnSwitcherPage.tsx
```

新对话接手时必须以实际 `git status --short --branch` 为准，因为本文写完后文件列表会发生一次可预期变化。

---

## 9. 阶段 1 接手后的第一轮命令

在 PowerShell 中执行：

```powershell
Set-Location 'C:\Users\10156\Desktop\脚本\切换应用\source'

git status --short --branch
git diff --stat
git diff --numstat -- package-lock.json

node --test scripts/tests/work-cn-stage1.test.mjs
npm run typecheck
npm run build

$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
cargo check -p cockpit-tools
```

验收要求：

- Node 测试 2/2 通过；
- TypeScript 退出码 0；
- Vite build 退出码 0；
- Rust check 退出码 0；
- `package-lock.json` 应只出现约 4 行新增、4 行删除；
- Rust 可能仍有上游 239 个 warning，不得把 warning 说成编译失败，也不得说 warning 已解决。

然后检查开发进程，防止 1420 端口冲突：

```powershell
Get-CimInstance Win32_Process |
  Where-Object {
    $_.CommandLine -match '切换应用\\source' -or
    $_.ExecutablePath -like 'C:\Users\10156\Desktop\脚本\切换应用\source\target\debug\*'
  } |
  Select-Object ProcessId, Name, ExecutablePath, CommandLine
```

若没有旧开发进程，启动：

```powershell
$env:GOPROXY='https://goproxy.cn,direct'
$env:GOSUMDB='sum.golang.google.cn'
$env:COCKPIT_SKIP_CLIPROXY_BUILD='1'
npm run tauri dev
```

人工验收：

- 窗口标题为 `TRAE Work CN 账号切换器 Dev`；
- 只显示 Work CN 页面；
- 有 4 个空账号槽；
- 没有其他平台导航；
- 开发目录为 `%USERPROFILE%\.trae_work_cn_switcher_dev`；
- 没有新建或读取 `.antigravity_cockpit_dev`；
- 开发控制台无前端错误。

完成后正常关闭开发版，不要在启动状态下继续进行真实切号开发。

### 当前工具链已知问题

本机尚未安装 `rustfmt`。若执行 `cargo fmt --check` 出现：

```text
cargo-fmt.exe is not installed
```

可执行：

```powershell
rustup component add rustfmt
```

完整 `cargo test -p cockpit-tools` 在上游巨大测试目标上曾由 `rustc` 以 Windows 状态 `0xC0000409 (STATUS_STACK_BUFFER_OVERRUN)` 异常退出。不能声称完整测试通过。后续应优先运行与本阶段相关的定向 Rust 测试，并在工具链条件允许时再次尝试完整测试。

---

## 10. 开发纪律

### 10.1 每个阶段固定流程

1. 阅读本阶段涉及的现有代码；
2. 列出拟修改文件和明确不修改的文件；
3. 先写会失败的定向测试；
4. 运行测试，记录预期失败；
5. 实现最小功能；
6. 运行定向测试；
7. 运行 `npm run typecheck`、`npm run build`、`cargo check -p cockpit-tools`；
8. 做阶段对应的人工验收；
9. 检查日志中没有敏感数据；
10. 查看 `git diff`，确认没有无关改动；
11. 写阶段报告；
12. 一个阶段一个提交。

### 10.2 测试隔离

Rust 测试优先设置：

```powershell
$env:TRAE_WORK_CN_SWITCHER_TEST_DATA_DIR = Join-Path $env:TEMP 'trae-work-cn-switcher-test'
```

测试禁止访问：

```text
%APPDATA%\TRAE SOLO CN
%APPDATA%\TRAE Work CN
%USERPROFILE%\.trae_work_cn_switcher
真实 GitHub Secrets
```

进程测试使用 fake process adapter，不得启动或关闭真实 TRAE Work CN。

### 10.3 日志安全

日志中禁止出现：

- 完整 JWT；
- refresh token；
- privateKeyPEM；
- GitHub PAT；
- 传给 `gh secret set` 的 stdin；
- 官方 storage 的完整明文。

允许记录：

- UID；
- 槽位号；
- 账号备注；
- 文件路径；
- Token 到期时间；
- secret 名；
- Token 的 SHA-256 短指纹。

### 10.4 Git 安全

- 不执行 `git reset --hard`；
- 不执行 `git checkout -- <文件>` 丢弃未知改动；
- 不修改或提交真实账号数据；
- 不使用 `npm audit fix` 自动升级固定基线依赖；
- 提交前检查 `git diff --check`；
- 未经用户明确要求，不 push、不创建 PR、不发布安装包。

---

## 11. 后续阶段顺序

### 阶段 1：应用壳最终验收

完成当前工作区的构建、启动、人工验收、阶段报告和提交。

### 阶段 2：Work CN 新旧名称和安装发现

让 `TraeSoloCn` 同时识别：

- `TRAE Work CN`；
- `TraeWork CN (User)`；
- `TRAE SOLO CN`；
- 新旧 EXE 名和 AppData 目录；
- 用户手动选择的 EXE。

新增只读安装检测 Tauri command，前端显示检测结果。

### 阶段 3：完整账号快照导入

从官方 storage 导入完整认证快照，保存在专用加密账号库。相同 UID 为更新，不占新槽；最多 4 个不同 UID。

### 阶段 4：事务式一键切换和回滚

实现单个后端命令，包含锁、当前会话回收、关闭、备份、注入、启动、UID 验证、成功提交和失败回滚。

### 阶段 5：积分查询和 UI

读取积分包，展示总积分、已用、剩余或无限。查询失败不影响切号。本地不 claim。

### 阶段 6：GitHub Secrets 同步

使用已登录的 `gh` CLI，通过 stdin 更新：

```text
TRAE1_TOKEN / TRAE1_DEVICE_ID
TRAE2_TOKEN / TRAE2_DEVICE_ID
TRAE3_TOKEN / TRAE3_DEVICE_ID
TRAE4_TOKEN / TRAE4_DEVICE_ID
```

GitHub 失败只标记待同步，不回滚已经成功的本地切号。

### 阶段 7：后台会话同步和四账号闭环

每 60 秒检查当前官方 storage 的修改时间；仅在变化时同步最新 token，并触发相应槽位的 GitHub 更新。切换锁占用时跳过。

### 阶段 8：清理和发布

稳定后再删除不再使用的页面、命令和依赖。最后生成当前用户安装包。不要在前面阶段大规模删除上游模块。

详细任务和接口见 `docs/CODEX_TRAE_WORK_CN_IMPLEMENTATION_PLAN.md`。

---

## 12. 阶段提交建议

```text
阶段 1：chore: specialize app shell for TRAE Work CN
阶段 2：feat: detect TRAE Work CN installations
阶段 3：feat: import complete Work CN account snapshots
阶段 4：feat: switch Work CN accounts transactionally
阶段 5：feat: display Work CN credit balances
阶段 6：feat: sync Work CN credentials to GitHub
阶段 7：feat: complete four-account Work CN workflow
阶段 8：chore: prepare Work CN switcher release
```

阶段 1 提交前可以新增标签：

```text
work-cn-stage-1
```

只有用户同意后再创建标签或 push。

---

## 13. Definition of Done

不能因为界面出现 4 个账号卡片就称为完成。最终完成必须同时满足：

- 能识别新旧命名的 Work CN 安装；
- 能导入 4 个不同账号的完整快照；
- A/B/C/D 任意顺序切换无需重复扫码或密码；
- 切换前回收当前账号最新 token；
- 切换后验证实际 UID；
- 任一步失败自动恢复原账号和原运行状态；
- 显示积分总量、已用、剩余或无限；
- 本地代码没有签到 claim；
- 8 个 GitHub Secrets 可正确同步；
- GitHub 失败不影响本地切换；
- 账号详情加密保存，索引和日志无敏感数据；
- 双账号连续切换至少 5 轮；
- 四账号完整切换至少 1 轮；
- 断网、GitHub 未登录、目标账号吊销等场景有明确结果；
- 最终安装包不包含任何开发者账号数据。

---

## 14. 可直接复制到新 Codex 对话的启动提示词

```text
请接手开发“TRAE Work CN 账号切换器”，始终使用简体中文回复。

项目目录：
C:\Users\10156\Desktop\脚本\切换应用\source

已安装的原 Cockpit Tools 仅供参考，禁止在其中开发：
C:\Users\10156\AppData\Local\Cockpit Tools

请先完整读取以下文件，再执行任何代码修改：
1. docs/CODEX_TRAE_WORK_CN_DEVELOPMENT_HANDOFF.md
2. docs/TRAE_WORK_CN_SWITCHER_GLM52_DEVELOPMENT_GUIDE.md
3. docs/STAGE0_BASELINE_REPORT.md
4. docs/CODEX_TRAE_WORK_CN_IMPLEMENTATION_PLAN.md

当前分支应为 codex/trae-work-cn-switcher，HEAD 仍是上游 v1.3.16 的 e1ef55ce。阶段 1 改动尚未提交，绝对不要 reset 或丢弃工作区。

当前先只完成“阶段 1 最终验收”：
- 检查 git status 和全部差异；
- 运行 node --test scripts/tests/work-cn-stage1.test.mjs；
- 运行 npm run typecheck；
- 运行 npm run build；
- 设置 COCKPIT_SKIP_CLIPROXY_BUILD=1 后运行 cargo check -p cockpit-tools；
- 启动 npm run tauri dev，验收 Dev 标题、仅 Work CN 页面、4 个空槽位和专用开发数据目录；
- 新增 docs/STAGE1_REPORT.md，准确记录命令、退出码、已知 warning、rustfmt 缺失和完整 cargo test 的基线异常。

验收通过后先向我报告，不要擅自 push 或发布。若我同意继续，则按 docs/CODEX_TRAE_WORK_CN_IMPLEMENTATION_PLAN.md 从阶段 2 开始，以 TDD、小提交方式开发。

永久约束：
- 用户可见名是 TRAE Work CN；内部继续使用 TraePlatformKind::TraeSoloCn 和 trae_solo_cn；
- 不新增 trae_work_cn 平台枚举；
- 桌面实际代码在 src-tauri/src/modules 和 src-tauri/src/commands；
- 数字 icube-dc DeviceID 与 telemetry.devDeviceId UUID 不能混用；GitHub 用 UUID；
- 本地绝不调用 /trae/api/v2/ug/checkin_credits/claim；
- 切换必须由后端单命令事务完成并可回滚；
- 测试不得读取真实 TRAE storage 或真实 GitHub Secrets；
- 日志不得输出 token、refresh token、私钥或 gh stdin；
- 不要同时运行原 Cockpit Tools 和开发版进行真实切号。
```

---

## 15. 一句话交接结论

项目方向和总体方案已经确定；现在最正确的接手动作不是重新设计，而是先把尚未提交的阶段 1 做完真实验收，再从“兼容 Work CN 新旧安装名称和路径”开始，一阶段一阶段实现完整快照、事务切换、积分和 GitHub 同步。
