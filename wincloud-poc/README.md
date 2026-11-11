# WorkBuddy Windows 云电脑：真实客户端 POC（2026-11-11）

**目的：**让安卓 DSH Go 经私人 HTTPS 访问云端 Windows 上实际运行的官方 WorkBuddy 和「切换应用」v0.1.22 账号池，电脑关机后仍可使用。

**目前未完成的部分：**尚未创建云 VM、登录 WorkBuddy、进行真实模型请求或安卓真机联调。Windows CI 测试的是脚本和虚构机器信息，不能称为云部署成功。

## 第一关：低成本云电脑

| 方案 | 已核实的新用户优惠 | 注意 |
|---|---|---|
| 腾讯云轻量服务器 | 个人试用页有 2核4GB / 1个月资源 | 以控制台领取资格、Windows 镜像和价格为准；优先做首次 POC |
| 阿里云 ECS | 个人新用户 300元抵扣额度，试用有效期最多3个月，每小时优惠上限0.833元 | 不是免费连续运行3个月；Windows 镜像费用、流量和超额扣费须核对 |
| Azure Windows B1s | 符合资格的新账户有每月750小时权益 | B1s约1GB内存，初次测试 Windows 图形客户端不推荐 |

权威来源：
- 腾讯试用：https://cloud.tencent.com/act/pro/free
- 腾讯 Windows 实例：https://cloud.tencent.com/document/product/1207/44549/
- 阿里 ECS 免费规则：https://help.aliyun.com/zh/ecs/user-guide/ecs-free-trial
- Azure：https://learn.microsoft.com/en-us/azure/cost-management-billing/manage/create-free-services

**领用条件**：先确认 Windows 镜像能在试用实例选择；不允许就暂停。关闭自动续费，设费用告警，记录到期日。不要误选按量付费规格代替免费试用。

## 第二关：验证真正的 Windows 桌面

WorkBuddy 官方说明支持 Windows 10及以上 x64，但**不承诺 Windows Server 2019/2022**，因此必须在云 VM 实测：
https://cloud.tencent.cn/document/product/1831/134387

一旦拥有一台临时 Windows 云电脑：

1. 用云厂商控制台提供的远程桌面或安全远程连接登录。
2. 从 https://www.workbuddy.cn/ 下载官方 Windows x64 安装包；在云电脑**正常完成个人登录**。
3. 从用户自己仓库的 v0.1.22 Release 安装「切换应用」，**在云电脑新建正常会话**。不迁移本机 Cookie、密钥或登录快照。
4. 在云电脑运行： powershell -File .\wincloud-poc\Test-WinCloudPreflight.ps1
5. 检查账号池在 127.0.0.1:18789 可启动、模型可选择；用真实的本地池 API Key 从该云电脑内发送**一条**普通模型流式请求。不要把 Key 交给 GitHub 或 ChatGPT。
6. **断开 RDP（Disconnect）而不是注销（Log off）**，测试手机是否仍能访问。微软文档说明断开通常保留用户会话进程，而注销会终止会话进程；但是 WorkBuddy 应用本身是否能稳定保持在线仍需实测。
7. 重启 VM 后再验证图形界面应用如何恢复。未登录图形会话不能假定自动启动；不推荐用明文密码实现不安全的自动登录。

RDP 相关资料：https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/tsdiscon

## 第三关：私人 HTTPS，不需要自己买域名

推荐 Tailscale Serve。默认私人 tailnet，不能误开成公网 Funnel。

Android DSH Go → 安卓 Tailscale VPN → 私人 HTTPS（主机名 .ts.net）→ Windows Tailscale Serve → 127.0.0.1:18789 → WorkBuddy 账号池。

1. 在云 Windows 安装 Tailscale：https://tailscale.com/download ，本人登录。
2. 安卓安装同一个 Tailscale 账号，开启同一 tailnet；按其提示启用 MagicDNS、HTTPS 证书。
3. 云 Windows 管理员 PowerShell **预览命令，不改变配置**：

       .\wincloud-poc\Enable-PrivateTailnet.ps1

4. 本地账号池已启动，并确认连接属于私人 tailnet 后，人工同意执行：

       .\wincloud-poc\Enable-PrivateTailnet.ps1 -Apply

   核心映射是 Tailscale Serve 后台 HTTPS 443 → HTTP loopback 18789。不会修改 WorkBuddy 的外部监听地址，不开放公网 18789。
5. DSH Go 配置，以下均是占位信息，需来自实际云电脑：
   - 类型：OpenAI Chat Completions / 兼容。
   - Base URL：https://云电脑主机名.你的tailnet.ts.net/v1
   - 模型 ID：账号池界面实际显示的 workbuddy-pool-... 模型。
   - API Key：该云电脑本地账号池生成的 Key，用户本人输入，绝不公开。
   - 开启流式输出：本地 v0.1.22 请求要求 stream=true。

Tailscale Serve：https://tailscale.com/docs/features/tailscale-serve
命令官方文档：https://tailscale.com/docs/reference/tailscale-cli/serve

**特别注意 Clash**：Android 通常同时只能运行一个系统 VPN；手机正在运行 Clash VPN 时，Tailscale 往往无法同时启动。需要实际检查手机网络环境；不要因为云电脑侧成功就误称 Android 已可用。
https://tailscale.com/docs/reference/faq/other-vpns

## 本项目脚本

- Test-WinCloudPreflight.ps1：只读检查 OS 类型、CPU、内存、磁盘、图形会话、客户端进程、loopback 18789 监听和 Tailscale 是否存在。不读取任何 WorkBuddy 账号、登录文件、Token 或密钥。
- Enable-PrivateTailnet.ps1：**默认只预览**；明确 -Apply 且经过确认后才设置私人 Tailscale Serve，绝不使用公网 Funnel。
- fixtures/*.json：虚构的 Windows 11、Windows Server、Azure 低内存实例数据，用于云端 CI 演练。
- Test-WinCloudPoc.Tests.ps1：PowerShell fixtures/安全默认值的回归测试。
- Windows GitHub Actions 仅运行测试，不使用任何账号和云资源。

## 验收标准（必须全部满足）

- [ ] 合规的试用 Windows VM 已开通，资费与到期续费清楚。
- [ ] WorkBuddy 在所选 Windows 镜像上真实启动并由用户本人成功登录。
- [ ] 账号池实际生成一次正常流式模型回复。
- [ ] RDP 断开后服务没有停，重启恢复行为已验证。
- [ ] 安卓 DSH Go 通过私人 HTTPS 实测成功且 VPN 不冲突。
- [ ] 账号使用符合平台条款；账号池 Key、登录信息不在任何公网日志或 GitHub 仓库内。

当前示例虚构机型返回 READY_FOR_INTERACTIVE_SMOKE_TEST_ONLY，仅意味着基础资源条件较好，**不等于正式云端可用**。
