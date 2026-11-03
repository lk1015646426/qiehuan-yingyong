# TRAE 连续对话账号池

## 交付目标

在原 TRAE 客户端选择账号池模型，继续同一对话。账号额度不足时，本地服务携带完整消息和工具历史尝试下一账号，模型名称保持一致。客户端负责保存对话、执行工具并回传结果。

首次配置：在切换应用的 TRAE 页面添加模型 → 保存模型并启动 → 注册到当前 TRAE 账号 → 确认注册。在 TRAE 的模型选择器中选择「TRAE 账号池 · 模型名称」。

模型注册会写入当前 TRAE 账号的自定义模型配置：本机回环地址与本地连接密钥。账号登录令牌仍由切换应用的加密账号库管理。注册动作有独立确认，启动恢复不自动改写云端模型。

## 实现边界

- `trae_model_pool.rs`：监听 127.0.0.1:18790，要求本地 Bearer 密钥，提供 models、流式及非流式 chat/completions；加载加密账号快照、官方模型目录并按账号轮换。
- `trae_pool_protocol.rs`：完整消息与原生工具协议转换、增量 SSE 读取、错误/额度判断。SOLO 的工具 schema 需要 JSON 字符串；调用历史中的 function 映射到 function_call。返回值恢复为标准 OpenAI 工具事件。
- `commands/trae_pool.rs`：异步命令边界；网络工作放在 blocking worker 中。
- `TraePoolPanel.tsx`：官方目录、选择/保存/启停、运行计数与原客户端注册。
- 原有 `trae_pool.rs` 只保留独立实例管理辅助代码，不作为连续对话池入口。

本地配置：`trae-model-pool.json`；本地连接密钥：`trae-model-pool.key`；注册收据：`trae-model-pool-registration.json`。均位于现有用户数据目录。所选模型及启用状态原子保存，程序重新启动时恢复启用的池。

## 请求与轮换规则

1. 校验本地认证、所选模型、消息格式和 64 MiB 请求上限。
2. 对候选账号查询原生目录，确认它具备所选的精确模型；保留模型大小写与标识。
3. 使用各账号自己的登录身份请求 SOLO 消息端点。全量消息与工具调用/结果关联保留。
4. 上游明确返回额度不足且尚未输出内容时，冷却当前账号并尝试下一账号。
5. 已输出文本、思考或工具事件后，报错/断流会结束当前请求，禁止自动重放。
6. 未取得正常结束事件时，返回错误，不伪造完成或成功的用量数据。
7. 停止时停止接收新工作，等待进行中的请求结束后释放监听端口。

只有一个账号时可正常使用；实际跨账号轮换需要至少两个有可用额度的账号。失效账号会被跳过，可通过已有的刷新凭证/重新导入流程恢复。

## 已验证的上游协议

模型目录：`POST /api/ide/v1/get_detail_param`，function=solo_work_lite。
对话：`POST /api/agent/v3/llm_utils_chat`，相同 function。
兼容请求指纹：IDE version 0.1.61 / version code 20260820；目录与对话采用同一组指纹。模型目录会受版本码影响，因此只展示服务端当前提供且可见、启用的模型，没有硬编码替代模型。

`llm_raw_chat` 的 solo_agent 通道受 Agent 内部调用约束，不能把它的 HTTP 200 或流内 4011 当作对话成功；此交付使用已通过真实请求验证的 SOLO 通道。

参考协议核对资料：
- https://github.com/aimod-cc/Agent2API/tree/main/desktop-tauri/src-tauri/server/src/server/core/providers/trae
- https://github.com/autumnsentiment/Trae2api-cn
- https://github.com/LeenHawk/gproxy/tree/main/crates/gproxy-channel/src/channels/trae

实现复用本项目已存在的 Rust 依赖，没有引入额外运行时。第三方代码的本机真实调用结果另行验证，源码中的测试结论不替代本项目验收。

## 验证证据（2026-11-03）

- Rust 全库回归：325 passed，0 failed，9 ignored；忽略项包含需要真实网络/人工运行的诊断。
- 前端/脚本测试：57 passed，0 failed。
- 真实本地服务验收：两轮上下文、原生工具生成、工具结果往返、流式输出、流式/非流式用量，以及未认证请求拒绝。输出文件位于 outputs/trae-pool-live-acceptance-20261103.json。
- 真实验收发现并修复：后续工具片段的空 ID/type 不能覆盖首次有效值；无 ID 的流为每个工具索引建立稳定本地 ID。回归覆盖分片参数与工具结果关联。
- 模拟独立上游：账号 A 额度不足后转给 B，完整请求一致；A 已输出后再报额度错误时，B 不会收到重放请求。

仍需以本轮最终产物记录为准：最终安装包哈希、安装状态、原 TRAE 客户端中的注册与接入验收。上述未确认项全部完成前，不将完成品目标标为完成。
