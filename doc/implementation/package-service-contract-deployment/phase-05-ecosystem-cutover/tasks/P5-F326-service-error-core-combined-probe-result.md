# P5-F326 Service error core combined probe result

状态：**PASS**。只解除 F327 独立 R0 验收的前置阻塞；不代表 A5 完成，也不解除 R1–R3。

## 精确候选与范围

- candidate commit：`49d9ab300f331f7662abfe8e6a0345f93c97f816`
- candidate tree：`ec596389abb5583fcfffc198205a657df5d4f616`
- F321 `fb2737ede6b0b76d63593a2186c8be9f6a012f08`、F322
  `bdfb8890acd4b1c1ae2482d14d59b1eed6926cea`、F324
  `8338442277d2d5cb1330dd512f9d0c0d97864e5c` 均为 candidate ancestor。
- 本探针只读 production；除本 result 外没有修改代码、fixture 或设计。未运行完整 eval、
  workspace/root、stable/live；generic WebSocket 两个既知失败不在本探针选择器内。

## 接线与 ownership

- `runtime/eval/Cargo.toml` 对 `skiff-runtime-model` 和 `skiff-runtime-boundary` 都有真实依赖；
  `service_error_channel.rs` 无条件进入 `assembly_execution`，`cargo check` 会编译该 production
  module，不是测试内复制模型。
- F321 的 imported cause 被 production 直接消费：export 先读
  `RuntimeError::fixed_service_failure()` 和 `RequestException::fixed_service_error()`，import 使用
  `RequestException::imported(...)` 保存同一个 `OpaqueServiceError`；三跳测试验证原始 bytes 不变。
- F322 的 selected codec 被 production 直接消费：eval 从 boundary 导入
  `ServiceValueSelection`，并调用唯一的 `ServiceValuePlan::encode_binary_selected` /
  `decode_binary_selected`；record/representation 使用 `Root`，named union 使用精确 ordinal。
- `RuntimeError::FixedServiceFailure` 及 accessor 只定义在 `runtime/eval/src/error.rs`；
  `export_provider_failure` / `import_caller_failure` 只定义在
  `runtime/eval/src/assembly_execution/service_error_channel.rs`；provider stack scope/reset 只定义在
  `runtime/eval/src/program_execution.rs`。未发现第二个 production owner。
- `ServiceErrorEnvelope`（含同 owner 内的严格私有 wire decoder）与
  `PlatformBuiltinErrorIdentity` 只定义在 `runtime/model/src/service_error.rs`；
  `ServiceErrorTypeIndex` 只定义在 `runtime/linked-program/src/service_error_index.rs`。eval 没有新增
  DTO 或基于字符串的 platform allowlist；其 platform payload match 是对 model enum 的穷尽 codec，
  不重新决定 identity membership。
- `CanonicalServiceErrorChannel`、两个 context 及 export/import API 的全仓 production 反搜没有
  module 外 caller；当前只有同 module 的 13 个聚焦测试调用。故 R1/R2/R3 尚未接入，符合本
  checkpoint 的预期状态。

## 正负矩阵

| 面 | production / 测试证据 | 结果 |
| --- | --- | --- |
| public record、representation、named union、dependency owner | index row、Package graph、schema record和 selected codec 联合校验；`record_linked...` 与 `dependency_representation...` 覆盖 Root、ordinal `1` 和 dependency package owner | PASS |
| linked / unlinked / opaque raw forward、三跳 | linked import恢复local carrier；unlinked/unknown owner保持`local_value=None`；provider→caller→relay第三次export仍逐字节相等 | PASS |
| private、nonclosed、encode failure、Internal once | private、带类型实参的nonclosed、schema encode mismatch均转固定Internal；普通runtime fault只分配一次correlation；InvalidArtifact不被吞成Internal | PASS |
| exact local / imported Internal | exact local `std.service.InternalError`固定为Internal；caller按exact std record恢复三字段；再次export原样forward，未二次包装 | PASS |
| platform exact / Resource package path | enum identity选择严格payload codec；File round-trip恢复exact identity；generic `ResourceError`转Internal，而公开`std.resource.ResourceError`走Package public typed envelope | PASS |
| owner / key / id / build / ordinal / payload mutation | known-owner key/owner/type-id、payload及union ordinal变异返回Protocol；未知caller build返回InvalidArtifact；完全未知owner仍合法opaque | PASS |
| local rethrow / remote new stack / provider reset | local rethrow对象整体相等，保留cause/source/stack/correlation；remote import只从caller-local stack新加一个`RemoteBoundary`；provider reset清空local frame且共享sequence不变 | PASS |
| private payload / source / display / message不进fixed bytes | Internal构造只含固定`Internal service error`与correlation；`FixedServiceFailure` display固定；测试确认private value/type和runtime diagnostic文本不在bytes，source wrapper不改raw carrier | PASS |

## 结构与反搜

- core production：
  `runtime/eval/src/assembly_execution/service_error_channel.rs` **1157 行**。宏观职责分段为：
  API/context与export/import编排（1–175）、local分类及三类import（176–546）、selected codec/schema
  校验（547–635）、Package graph/index/identity一致性（636–871）、context/fixed envelope校验
  （872–967）、artifact identity与platform payload codec（968–1148）。
- co-located tests：
  `runtime/eval/src/assembly_execution/service_error_channel/tests.rs` **1435 行**。13个矩阵用例位于
  1–869，通用assert/helper位于870–930，assembly/package fixture builder位于931–1435。
- 文件长度是明确的 F327 review risk；graph/index validator 与 platform payload codec 从物理上可拆。
  但宏观反搜没有显示它们已经造成重复classifier、module/Cargo循环依赖或多个owner，因此本cheap
  probe不把“可拆分”本身升级为blocker，最终结构判断留给F327。
- core production 对 `shape|display|static type|fallback|from_symbol|actual_payload_type|
  RuntimeErrorPayload` 反搜为零；`message`只出现在固定Internal、精确Internal schema或按enum选择的
  platform payload字段，`code`只表示Package code slot/局部变量，没有message/code推断identity。
- eval core 内没有任何platform symbol字符串表；Resource只在model registry说明及负测试中出现，
  production `RuntimeError::catch_projection` 对`ResourceError`显式返回`None`。
- `ServiceErrorEnvelope`三个variant都没有source/stack/display字段；stack只在caller-local import时由
  `caller_stack_at_site + RemoteBoundary`新建。反搜没有stack序列化、callee stack进入
  `encoded_payload`或canonical bytes的路径。

## 命令证据

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-model --lib --no-fail-fast` | PASS，84 passed / 0 failed |
| `cargo test -p skiff-runtime-boundary --lib --no-fail-fast` | PASS，181 passed / 0 failed |
| `cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel -- --list` | PASS，selector 13 tests / 0 benchmarks，非零 |
| `cargo test -p skiff-runtime-eval --lib assembly_execution::service_error_channel --no-fail-fast` | PASS，13 passed / 0 failed / 155 filtered out |
| `cargo check -p skiff-runtime-eval --lib` | PASS |
| `git diff --check` | PASS |

精确失败：无。model/boundary命令无warning；eval test构建报告既有
`skiff-compiler-source` 27 warnings、`skiff-runtime-linker` 32 warnings，以及test-only
`runtime/eval/src/test_effect_registry.rs:671`一个`unused_mut` warning；`cargo check`仅报告
`skiff-runtime-linker`的32个warning。上述warning不来自本只读探针改动，也没有改变命令退出状态。

## 结论

F321 imported cause、F322 selected codec 与 F324 canonical core 在精确候选上真实合流，正负矩阵和
owner/反搜均通过。**P5-F326 PASS**，可以进入F327独立R0验收；R1–R3仍须后续接线。
