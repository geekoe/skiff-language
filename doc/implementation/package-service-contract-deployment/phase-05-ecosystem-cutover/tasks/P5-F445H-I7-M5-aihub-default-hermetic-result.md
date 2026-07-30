# P5-F445H I7 M5 AIHub 默认隔离测试结果

## 范围与基线

- Skiff：`b4bdbddb8761bcf053258eef5b87b778c3299b7a`
  （tree `7d81c6ef01cb47c2a7904cdc48ccd8f4d11a9ed7`）
- Internals：`9c3bdc82c4a43e575ea627357c05f54dbc0400a8`
  （tree `c3f159a397cd3c2b316a502ce945d8a935a9c2c3`）
- 官方 packages：`b06d7aaf16b6914837de1f74920fd3f626040472`
  （tree `fb9db28a7d1bd3babafd1dfa7a23687e393ff856`）
- 目标：`aihub/service-tests` 目录默认发现的 51 个测试。
- 排除：带 `defaultRun false` 的 `gemini.live.test.skiff`；未使用真实 API key，
  未访问外网。

执行环境从空环境开始，只保留 `PATH`、`HOME`、`USER`、`LANG`、`TMPDIR`
和本次验证需要的 Skiff 路径、离线及诊断开关。Cargo 使用离线模式。测试配置只使用
`config.skiff-test.yml` 中的 placeholder/fake 绑定。隔离栈使用临时 Mongo、动态端口和
临时目录。

## 结论

默认 51 个测试完整执行：**15 pass，36 fail，0 skip**。因为 AIHub 未全绿，按门禁未继续
执行 Agine 默认 170 个测试。

失败只有两类：

| 分类 | 数量 | 首个测试 |
| --- | ---: | --- |
| `unsupported native target std.json.encode` | 34 | `provider selection prefers body provider` |
| `unknown Stream value` | 2 | `codex relay chat projects completed RPC response without cross-service stream` |

所有 case 共用运行时 assembly：
`skiff-runtime-assembly-v3:sha256:58033468e63c9d45571e58fa96d6fbd1dd5417c4019f9a3f609041f46c9d0ead`
（generation `1`）。每行的部署为
`test.skiff/package/agine.ai/aihub-tests/case-<序号>`；因此下面的逐 case 结果同时绑定了
case、部署和 assembly 身份。

## 逐 case 去敏记录

| 序号 | case | 结果 |
| ---: | --- | --- |
| 0 | provider selection prefers body provider | E：`std.json.encode` |
| 1 | provider selection reads header and query | E：`std.json.encode` |
| 2 | provider selection infers from model catalog | E：`std.json.encode` |
| 3 | provider selection rejects unknown and removed providers | E：`std.json.encode` |
| 4 | metadata routes respond with providers and models | E：`std.json.encode` |
| 5 | aihub catalog follows the llm-providers allowlist | PASS |
| 6 | public provider catalog exposes aihub builtin provider model api formats | E：`std.json.encode` |
| 7 | model catalog ids are globally unique | E：`std.json.encode` |
| 8 | llm request uses deepseek managed request from model inference | E：`std.json.encode` |
| 9 | llm request uses gemini managed request from model inference | E：`std.json.encode` |
| 10 | llm request rejects missing model from explicit provider | E：`std.json.encode` |
| 11 | llm request maps deepseek native disabled reasoning level | E：`std.json.encode` |
| 12 | llm request maps deepseek native max reasoning effort | E：`std.json.encode` |
| 13 | llm request uses codex relay responses body from model inference | E：`std.json.encode` |
| 14 | responses body uses upstream llm tool call id metadata | PASS |
| 15 | llm request maps gpt-5.5 native xhigh reasoning effort | E：`std.json.encode` |
| 16 | llm request rejects unsupported reasoning level for catalog model | E：`std.json.encode` |
| 17 | llm request rejects empty model instead of defaulting | E：`std.json.encode` |
| 18 | llm request rejects explicit provider and model mismatch | E：`std.json.encode` |
| 19 | llm request maps qwen native thinking toggle | E：`std.json.encode` |
| 20 | llm request maps function tools and tool call messages | E：`std.json.encode` |
| 21 | llm request flattens gemini historical tool messages into text | E：`std.json.encode` |
| 22 | llm host preflight rejects unsupported request apiFormat before transport | PASS |
| 23 | llm host preflight uses provider apiFormat when request omits apiFormat | PASS |
| 24 | codex relay preflight treats catalog base URL as display-only | PASS |
| 25 | codex relay preflight still requires the configured relay key | PASS |
| 26 | codex relay chat builds unary service dependency request without router selectors | PASS |
| 27 | codex relay exact service operation returns completed output | E：`std.json.encode` |
| 28 | codex relay chat projects completed RPC response without cross-service stream | S：`unknown Stream value` |
| 29 | codex relay tagged failures project stable non-empty error finishes | S：`unknown Stream value` |
| 30 | codex relay remains a service dependency transport | PASS |
| 31 | codex relay chat validation is static and does not require admin state | E：`std.json.encode` |
| 32 | gemini provider request targets OpenAI-compatible endpoint | PASS |
| 33 | web search backend selection prefers responses models unless disabled | PASS |
| 34 | web search uses purpose configuration or a compatible explicit Agent model | PASS |
| 35 | openai web search request targets codex relay responses endpoint | PASS |
| 36 | gemini web search request targets native google search endpoint | PASS |
| 37 | web search maps openai responses output steps and citations | E：`std.json.encode` |
| 38 | web search maps gemini interactions steps and citations | E：`std.json.encode` |
| 39 | local provider is no longer accepted | E：`std.json.encode` |
| 40 | chat rejects unsupported provider | E：`std.json.encode` |
| 41 | chat completions rejects a missing model as invalid input | E：`std.json.encode` |
| 42 | chat completions rejects a missing provider and model without selecting a global default | E：`std.json.encode` |
| 43 | chat events rejects a missing model as invalid input | E：`std.json.encode` |
| 44 | chat events HTTP route returns structured event body | E：`std.json.encode` |
| 45 | chat event stream preserves per-item chunk order and full event projection | E：`std.json.encode` |
| 46 | chat event stream keeps emitted items before each post-start failure | E：`std.json.encode` |
| 47 | chat event stream returns finite JSON errors before successful stream start | E：`std.json.encode` |
| 48 | chat event stream consumer break cancels the provider ancestor chain | E：`std.json.encode` |
| 49 | cors options is public | PASS |
| 50 | managed Gemini transport follows provider endpoint and keeps credential outside body | PASS |

## E：泛型 JSON encode 运行时链

临时、仅由环境开关启用的插桩为 34 个 E 失败捕获到一致信息：

```text
target="std.json.encode"
binding="std.json.encode"
unit=Package(5)
file=LoadedFileIndex(7)
executable_index=0
file_ir_identity="skiff-file-ir-v9:sha256:93ff258556a8df91178e7b6a9e33c2f54ece450e8e430c4bf7b95318b619330c"
module_path="std.json"
symbol="std.json.encode"
type_args={"T0": TypeParam { name: "T" }}
type_substitutions={"T": TypeParam { name: "T" }}
```

精确链路：

1. `resolve_runtime_native_invocation_in_type_view` 无法从自指替换
   `T -> TypeParam(T)` 编译 exact native plan；
2. 该函数对 `std.json.encode` 的这一错误显式降级为 `plan = None`；
3. `eval_native_prepared_call` 在进入 JSON dispatcher 之前无条件读取
   `invocation.return_plan()`；
4. `return_plan()` 调用 `require_plan()`，因此生成
   `unsupported native target std.json.encode`；
5. JSON dispatcher 本身已有 `plan.is_none()` 的无计划 encode 分支，但执行流到不了该分支。

HTTP 结果只返回上述错误字符串，没有返回语言堆栈；这里的“栈”是由调用点插桩和对应代码路径
共同确认的运行时链，不包含任何运行时值或配置值。

## S：Stream 运行时链

两个 S 失败都发生在 Codex Relay 完成响应投影。错误源是
`StreamRuntime::next_with_cancellation`：传入值能够解析出 Stream id，但该 id 在当前
`StreamRuntime` registry 中不存在，registry lookup 返回 `unknown Stream value`。

此路径不是 native 调用，所以：

- target：`StreamRuntime::next_with_cancellation`
- binding：不适用
- type substitutions：不适用
- executable：测试部署 `case-28` / `case-29` 内的 Codex Relay 响应投影链
- 错误位置：`runtime/host/src/capability_context/stream_runtime.rs` 的 registry `get(id)`

HTTP 结果同样只返回错误字符串，没有返回语言堆栈。现有证据能确认“跨
`StreamRuntime` registry 使用了 Stream id”，但不能仅凭本轮证据判断 id 是被提前移除，
还是由另一个 registry 创建；修复前应单独跟踪 stream 创建 registry、消费 registry 和
scope/owner 关闭事件。

## 清理要求

本结果不包含任何 secret 值。临时插桩不属于候选实现，记录完成后必须撤回。隔离运行结束后
必须确认 Mongo、Router、Runtime、动态端口和 `skiff-test-runtime-*` 目录均已清理；保留的
临时 Cargo 构建目录也必须删除。
