# WorkBuddy 官方授权可行性闸门（2026-11-11）

**用户需求**：手机/平板上的 OpenAI 兼容客户端；电脑关闭时仍可调用；主要在中国大陆网络使用，几个月、总支出尽量为 0；优先考虑原有 WorkBuddy 多账号池能力。

## 已有可靠证据

参照官方文档（2026-11-11 复核）：

- https://open.workbuddy.cn/docs/third-party-app
- https://open.workbuddy.cn/docs/openapi

### 开放平台应用类型

1. “硬件接入”被描述为面向硬件设备厂商和合作伙伴，能按业务申请云端任务权限 user.task.invokable、user.task.readable。
2. “积分兑换”只提供积分权益相关权限，**不能用来取得云端任务权限**。
3. Open API 还提及 Buddy 应用，但公开文档未给出足以确认“个人开发者能否申请云端任务权限”的完整资格规则。
4. OAuth 授权需要已注册并审核通过的第三方应用，申请时必须配置回调地址；官方样例为 HTTPS。
5. 用于云端任务的 ACP token 由创建任务或查询任务接口返回；注意这是**官方 Open API 的云端任务会话票据**，不是 PC 本地账号池令牌。
6. Open API 文档并未承诺官方云端任务可复用现有 Windows WorkBuddy 多账号模型池的积分，也未承诺其免费调用次数。

### 哪些事情不能用编码解决

- 申请方实际是否具备硬件接入或 Buddy 应用注册资格（以后台界面与官方答复为准）。
- 是否能获批 user.task.invokable / user.task.readable。
- 官方云端任务是否计入现有账号的普通模型积分、是否存在额外收费或数量限制。
- 第三方应用能否在没有自有 ICP 域名的条件下使用平台允许的 HTTPS 回调。
- 云端运行地域是否适合中国大陆网络、是否有固定免费测试额度。

## 给官方支持/审核人员的咨询清单（不自动发送）

**咨询场景**：个人学习与开发的短期模型 API 集成验证，使用 WorkBuddy 官方 OAuth/Open API 云端任务能力，将 ACP 的普通文本输出适配到仅本人使用的 OpenAI Chat Completions 客户端。不会让第三方账号共享或自动工具审批，不使用非公开客户端身份数据。

待核实问题：

1. 个人开发者是否可以创建“硬件接入”或 “Buddy 应用”，并申请云端任务 Scope（user.task.invokable、user.task.readable）？如不行，有无适用于个人合法测试的申请通道？
2. 申请材料是否必须包含公司主体、已备案网站、真实硬件产品，或者可否只提供短期测试场景？
3. 官方云端任务 API 产生的任务与 ACP 对话是按什么方式计费/消耗积分？可以使用用户 WorkBuddy 现有积分吗？与 PC 客户端多账号池额度是同一套吗？
4. 能否申请用于私人测试的低额度/免费沙盒配额？如有，期限、请求数、并发与模型限制是什么？
5. Open API 注册的 OAuth callback 是否支持平台托管域名？没有自有ICP备案域名能否申请用于私人开发测试？
6. 通过审核后，官方是否允许把云端任务的**纯文本**回复适配为本人客户端使用的 OpenAI Chat Completions 风格 API？有哪些场景限制？

**重要说明**：不要给官方发送真实 WorkBuddy 登录 Cookie、电脑端本地密钥、refresh_token、账号池导出、ChatGPT/GitHub 凭证。可提供脱敏架构图、接口名单与测试代码 PR。

## 已完成代码和验证

独立分支：codex/workbuddy-official-cloud-p0
- OAuth：授权 URL/状态防护、token 换取和刷新模型（模拟测试）。
- 官方任务：POST/GET 请求与响应结构、ACP ticket 只保留内存并隐藏 repr。
- ACP：SSE GET 与 JSON-RPC POST 通过 Acp-Connection-Id 绑定。
- OpenAI 文本桥接：只映射 agent_message_chunk；工具执行/审批不自动批准。
- 测试：Windows 已运行过 33 项；Ubuntu CI 增加双通道 HTTP 模拟测试后通过（须以最新 Actions 明确统计为准）。

## 下一阶段正确顺序

1. 等待申请资格/Scope 与积分费用确认（**当前阻塞的真实授权和费用信息**）。
2. 授权成功后，使用平台发放的 OAuth 应用凭据；在安全的后台临时完成最小真实云端任务，严禁把 Key 放进仓库。
3. 对 ACP SSE 消息做脱敏采样，校验其在真实场景与离线合约是否一致，必要时修协议差异。
4. 设计具有 HTTPS 入口、安全会话存储和授权审批处理的私用网关，核实云服务的费用/备案。
5. 安卓 DSH Go 小流量联调。

**不要把当前 Mock/GitHub Actions 成功解释为：WorkBuddy 多账号池已经迁到云端、免费额度可以使用、个人审核必过或 DSH Go 已真实联网。**
