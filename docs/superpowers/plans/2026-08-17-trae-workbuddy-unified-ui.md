# TRAE 与 WorkBuddy 统一界面及云端签到整合实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 统一 TRAE 与 WorkBuddy 的账号管理视觉结构，并在云端签到页以产品标签和筛选同时管理两类账号。

**Architecture:** 保留 `useWorkCnStore` 与 `useWorkBuddyStore` 的独立业务边界，在签到页建立只负责展示和筛选的联合视图模型。TRAE 继续使用现有槽位 Secret、workflow 与本地签到；WorkBuddy 复用现有聚合 Secret 同步和按账号 workflow 触发命令。共享 `wc-*` 设计 token 和卡片结构，不合并两个产品的数据模型。

**Tech Stack:** React 19、TypeScript、Zustand、Vite、Tauri 2、lucide-react、现有 CSS token。

## Global Constraints

- 所有用户可见文案使用简体中文，并明确标注 TRAE 或 WorkBuddy 来源。
- 不修改两类账号的持久化格式、GitHub Secret 格式或签到协议。
- 不回退工作树中已有的用户修改；只修改本功能涉及文件。
- WorkBuddy 状态查询必须按账号独立显示加载和错误状态。
- 账号筛选只影响渲染，不删除或重置任一 store 的状态。

### Task 1: 统一侧栏命名和导航图标

**Files:**
- Modify: `src/App.tsx`
- Test: `npm run typecheck`

**Interfaces:**
- Consumes: `traeCnIcon`、现有 `PageKey` 与侧栏 state。
- Produces: 侧栏 `TRAE` 导航项，使用 TRAE 图标并继续渲染 `WorkCnSwitcherPage`。

- [ ] 将 `ArrowLeftRight` 从 TRAE 导航项移除，保留 `CloudUpload` 用于云端签到。
- [ ] 将按钮文本从“账号切换”改为“TRAE”，图片使用 `traeCnIcon`，尺寸与 WorkBuddy 导航图标一致。
- [ ] 更新侧栏注释和必要的空白区域，确保 active 状态仍由 `page === 'switcher'` 控制。
- [ ] 运行 `npm run typecheck`，预期无 TypeScript 错误。

### Task 2: 抽取签到产品筛选纯函数和类型

**Files:**
- Modify: `src/types/checkin.ts`
- Create: `src/utils/checkinProducts.ts`
- Create: `src/utils/checkinProducts.test.ts`

**Interfaces:**
- Produces: `CheckinProduct = 'all' | 'trae' | 'workbuddy'`、`filterCheckinProducts`、`checkinProductLabel`、`checkinProductIconAlt`。

- [ ] 在 `src/types/checkin.ts` 增加产品联合类型，并定义带 `product` 字段的联合签到卡片模型，分别承载 `WorkCnAccountView` 和 `WorkBuddyAccountView`。
- [ ] 在 `src/utils/checkinProducts.ts` 实现纯函数：`filterCheckinProducts(items, product)` 保持输入顺序；`checkinProductLabel` 返回“全部 / TRAE / WorkBuddy”；未知筛选值回退到 `all`。
- [ ] 添加 Node `node:test` 测试，覆盖全部筛选、TRAE 筛选、WorkBuddy 筛选、空列表和稳定顺序。
- [ ] 运行 `node --test src/utils/checkinProducts.test.ts`，预期全部通过。

### Task 3: 统一 WorkBuddy 账号页视觉结构和文案

**Files:**
- Modify: `src/pages/WorkBuddyPage.tsx`
- Modify: `src/styles/pages/workbuddy.css`
- Modify: `src/styles/pages/work-cn.css`

**Interfaces:**
- Consumes: 现有 WorkBuddy store、安装检测、WorkBuddy 命令服务。
- Produces: 与 TRAE 相同的页面标题、状态区、账号卡片层级和产品识别。

- [ ] 保持 WorkBuddy 特有字段，但把账号卡片标题区改为“图标 + WorkBuddy 标签 + 账号名 + GitHub 状态”的统一结构。
- [ ] 将按钮文案统一为“切换并打开”“刷新状态”“立即签到”“备注”“删除”，并为自动签到禁用状态提供明确 title。
- [ ] 将页面级“同步 GitHub”改为“同步 WorkBuddy”，将安装状态和 GitHub 反馈文案带上 WorkBuddy 产品名。
- [ ] 调整 `workbuddy.css` 使用与 `wc-slot` 同等的最小高度、列宽、卡片间距和响应式换行规则，避免 WorkBuddy 卡片比 TRAE 卡片跳动明显。
- [ ] 保留现有 30 秒 session watcher，但确保安装检测 refresh 不阻塞账号列表首次渲染。

### Task 4: 将 WorkBuddy 账号接入云端签到联合列表

**Files:**
- Modify: `src/pages/CheckinPanelPage.tsx`
- Modify: `src/styles/pages/checkin.css`
- Modify: `src/services/workBuddyService.ts`（仅在现有导出不足时）

**Interfaces:**
- Consumes: `useWorkCnStore`、`useWorkBuddyStore`、`filterCheckinProducts`、WorkBuddy 图标和现有签到服务。
- Produces: 云端签到页的 `全部 / TRAE / WorkBuddy` 筛选、带产品标签的联合卡片和独立操作反馈。

- [ ] 在页面 state 中增加 `productFilter: CheckinProduct`，默认 `all`，渲染前分别将 TRAE 与 WorkBuddy 账号映射成带 `product` 的联合条目，再调用纯函数过滤。
- [ ] 挂载时并行调用 `loadAccounts`、`loadGitHubConfig`、`refreshGitHubCliStatus`、`loadRuns` 以及 WorkBuddy `loadAccounts`；WorkBuddy 状态详情不得在页面挂载时串行阻塞 TRAE 卡片。
- [ ] 把现有 `CredentialCard` 保持为 TRAE 专用卡片，并新增 `WorkBuddyCheckinCard`：显示产品图标、账号名、token 到期、积分、今日奖励、连续签到、自动签到状态和 GitHub 同步状态。
- [ ] WorkBuddy 卡片接入 `refreshAccountStatus`、`syncGitHub` 和 `triggerCheckin`，每个按钮按对应账号 ID 独立禁用；未启用自动签到时禁用立即签到并显示原因。
- [ ] 页面级按钮改成“验证 TRAE 签到”“同步 TRAE”“同步 WorkBuddy”，避免共享按钮造成产品混淆；WorkBuddy 签到只使用卡片级操作。
- [ ] 运行历史保留共享列表；根据可识别的标题标记 TRAE/WorkBuddy，无法识别时显示“通用任务”，不猜测来源。
- [ ] 添加 CSS：分段筛选、产品图标/标签、联合卡片空状态、窄屏换行和产品色仅作辅助识别。

### Task 5: 自动化验证和构建

**Files:**
- Modify: only files required by failing checks.

**Interfaces:**
- Consumes: Tasks 1-4 的组件、纯函数和现有 Rust 命令。
- Produces: 可构建的前端和 Tauri 安装包前置验证结果。

- [ ] 运行 `node --test src/utils/checkinProducts.test.ts src/utils/checkinPresentation.test.ts src/utils/tokenRotation.test.ts`，预期全部通过。
- [ ] 运行 `npm run typecheck`，预期无错误。
- [ ] 运行 `npm run build`，预期 Vite 构建成功。
- [ ] 在 `src-tauri` 运行 `cargo test`，确认 WorkBuddy 聚合 Secret 和按账号 `account_filter` 测试通过。
- [ ] 运行 `npm run release:preflight`；若环境缺少打包依赖，记录具体阻塞，不修改无关配置。
- [ ] 使用 `git diff --check` 检查空白错误，并汇总变更文件与未能运行的验证项。
