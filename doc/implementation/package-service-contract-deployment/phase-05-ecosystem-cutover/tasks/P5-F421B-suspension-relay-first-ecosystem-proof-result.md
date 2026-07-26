# P5-F421B Suspension Relay-first fresh ecosystem proof result

状态：Relay-first exact verdict 通过；同一 sibling wave 在真实 AIHub production authoring 上发现
current CLI incompatibility，按任务合同停止。

```text
TASK_SCOPE_EXPANDED
N5_FAIL
```

本节点没有把已接受的 task-only Skiff 提交判成 input 漂移。fresh rebuild 已真实执行到
Relay-only assembly 和全部依赖已满足、与 AIHub 独立的 sibling；没有修改任何 source、
test、fixture、manifest 或 oracle。

## 1. 精确输入与 fresh root

| 输入 | commit | tree | 结果 |
| --- | --- | --- | --- |
| Skiff production/executable 冻结锚点 | `9f39580655ecbd433235cdb7de19d823d670d4a9` | `d20cd4ccd8f11042a1f4bc6dac69d3ccda1116b9` | checkout ancestor；排除 phase-05 task 目录后到 checkout 零 diff |
| Skiff task base | `bc23b84850155045c5e08532186466e243ebf536` | `ac39e5cd2b41c0b64f19b1fe43cebd5c8ad33765` | task checkout parent 精确匹配 |
| Skiff task checkout / integration root | `1847cd455e35b309193606a6def9371e409474f3` | `e8498c1e79449382ff901941c63e5721fd944356` | 相对 parent 只新增 F421B task；clean；这是修正后允许的 task-only checkout |
| Internals integration | `baf0c907ee26e48a5fb4c153825c233bde3a6234` | `13f2f6e604fedbad80e0390e5408507430e28f8c` | exact；clean |
| skiff-packages integration | `0972e65604cd4cfd45bcdb289cfe5019f57dc265` | `1849f97a1f1217b95e6e349bc529eaaf220a62f4` | exact；clean |

N4 executable candidate `29419bc999d441b78f1e452a454c2b24e6e30a87` 是当前 Skiff
checkout 的 ancestor；candidate 到 checkout 排除 phase-05 task/result 文档后 production diff
为空。启动与停止快照均未发现三个 integration root 的 index lock 或并发 production writer。

唯一 task-owned 系统临时根保留在：

```text
/private/tmp/p5-f421b-ecosystem-proof.39XTpZ
```

三个 source mirror 均由 exact integration Git archive 直接生成，没有 patch：

| mirror | tree | archive SHA-256 |
| --- | --- | --- |
| Skiff | `e8498c1e79449382ff901941c63e5721fd944356` | `6736a0af823b978439143c6f0a635b34ccff840ab0261d279c343a085e70f306` |
| Internals | `13f2f6e604fedbad80e0390e5408507430e28f8c` | `f054a9ae18c6977a11542eb9196ab063ea38cf272e5d4073c457639cd9938963` |
| skiff-packages | `1849f97a1f1217b95e6e349bc529eaaf220a62f4` | `90cca5ba7ba2eb731e4e07443445e01cd4701b67e4e69d8c7fc19fb829c2485a` |

复用了任务允许的
`/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target`；fresh 对象是 source
mirror、artifact store 和全部生成 records。没有复制 Cargo target。

完整 command ledger、每条 stdout/stderr 原文、record path/SHA/schema/identity、input/lock
inventory 和边界声明在：

```text
/private/tmp/p5-f421b-ecosystem-proof.39XTpZ/final-receipt.json
SHA-256 cde06780e0175c53ac75e6d0ed54493fba2407871158648ec0f918c499464cf4
```

## 2. Relay-first rebuild 与 exact verdict

canonical 顺序实际为：

```text
std bootstrap
  -> llm-api publish
  -> llm-providers publish
  -> Relay publish
  -> Relay-only assembly build
  -> canonical Relay proof checker
```

六步均 exit `0`。关键 fresh refs：

| record | exact ref |
| --- | --- |
| std package | `skiff-package-build-v10:sha256:0dec996a2d6388245539fb000a0284a1561dc21ac3cc6e88ed3fbe0eadfe3d43` / `skiff-package-local-abi-v7:sha256:ce09dc5902ce992d7b362f48ce1ea5466e12fc0e950d4fa90ec99ba46b86db9e` |
| llm-api package | `skiff-package-build-v10:sha256:1182691e2b41ff3a121e33e19422804a85870638a8ae181d3812077d9448b9b6` / `skiff-package-local-abi-v7:sha256:65de703a4648ea8a72693a5a2b57bd38140c2e224ce1306ab596955f0302ab3c` |
| llm-providers package | `skiff-package-build-v10:sha256:a057d0c055260a88b10794d99e29b02c49aedb2ca780aa1a2bec4c7c735cbd62` / `skiff-package-local-abi-v7:sha256:9343176844141e5203a38e4a3ff112ef664123d4bc75c0c8f5f703d6e6906d55` |
| Relay package | `skiff-package-build-v10:sha256:49da35d8aeded55f6315043c3f2df5e14583fd0d2edd82e17f53e4030be90249` / `skiff-package-local-abi-v7:sha256:a1b4e087b917cbfc9f73c6946da61f113f4edbbf2db94d0f70e0b9d0875eca5a` |
| Relay contract | `skiff-service-protocol-v5:sha256:c1e3f8be3d63b3b2864eb7f36d5f92dad4014005915a5a1bda3590e1ea6649fa` |
| Relay deployment | `skiff-deployment-artifact-v2:sha256:410d05aeae32cfb6e4c87e897a18839031f649aa9ec32c17a8ffa40fd9c46a47` |
| Relay-only assembly | `skiff-runtime-assembly-v2:sha256:56fd535335f026795c100ba8f92c1eeaec92b39f0ca52b3f9b221b7262623e59` |

task-owned Rust checker 通过 current
`CanonicalArtifactStore::{read_package_artifact,read_file_ir,read_service_contract,
read_service_deployment,read_runtime_assembly}` 重新读取 exact record paths，并得到：

- operation set 精确只有 `relayProxy.responsesCompleted` 与
  `relayProxy.responsesCompletedResult`；
- operation IDs 精确分别为
  `skiff-contract-operation-v1:sha256:b62d89d553cc0607b2627b047d2a5ab4665c70f05f900babbce249def47099ef`
  与
  `skiff-contract-operation-v1:sha256:51fa082dd0d33b09f45e4900805c28801cb3108b4eac813697e66e5f8a6b007d`；
- 两个 `PackageCallableSignature.maySuspend` 均为 `true`；
- 两个 concrete executable
  `relay.CodexRelayProxy.responsesCompleted`（index `14`）和
  `relay.CodexRelayProxy.responsesCompletedResult`（index `15`）的
  `ExecutableIr.maySuspend` 均为 `true`；
- hydrate 的一个 interface record 递归无 `maySuspend`；
- ServiceContract operations 递归无 `maySuspend` 和 `cancellation`；
- generation 精确为 PackageArtifact v9、Local ABI v7、Package build v10、
  ServiceContract v5、ServiceProtocol v5、ServiceDeployment v2、RuntimeAssembly v2；
- Relay 的 `llmApi`、`llmProviders`、`std` 三条 requirement /
  deployment binding / assembly link exact refs 一致，三条
  `collectionNameMapping` 都被显式 hydrate 和记录为 `{}`，没有因 wire 省略而跳过。

Relay exact receipt：

```text
/private/tmp/p5-f421b-ecosystem-proof.39XTpZ/receipts/relay-verdict.stdout.json
```

## 3. 同一 sibling wave

AIHub 失败后，按任务要求只继续运行依赖已满足且与它独立的 sibling。实际结果为：

| sibling | exit | fresh record / 结果 |
| --- | --- | --- |
| Agent | `0` | Package build v10 `2ce553927fb0a5753f98c3d5f5127f4cd94c286fa8bed0fe455d95f8ae1b3734` |
| http-session | `0` | Package build v10 `dbc29aeca9d067a81a9e5bff82b23a86378975be5d4a38b7726f373689f01f8e` |
| AIHub | `1` | production `service.yml` 被 current canonical reader 拒绝；无 AIHub record |
| Registry | `0` | protocol v5 `d8825672efdce323ae716e8f78152b14ec5b915f9a1eb08637be1c9b7fbc238c`；deployment v2 `0685de7351907068e374404e0539a60563b6f22b09371b6ce0c1216ed865194f` |
| track | `0` | 在 http-session 成功后执行；Package build v10 `ef3d76192d38f61e9096eec8d03effd0bee2de91c5dc92587e59b34f41f86faa` |
| Account | `0` | 在 http-session 成功后执行；protocol v5 `62e6eb806c368ec041b3a8d74318d3fb068fd50356142d9326668490efd1e7cf`；deployment v2 `4da2b7848a94228de2c142e33c373171a4e33c3a712bd5d0b19ef987a08d6ad2` |

因此同一 sibling wave 的独立 production compatibility error 精确只有 AIHub 一项；没有把
warning 当作第二个 production error。

## 4. 精确 blocker 与 first canonical command

first failing canonical command 是：

```bash
node /Users/geek/workspace/skiff-phase-05-integration/scripts/skiff.mjs \
  package publish \
  /private/tmp/p5-f421b-ecosystem-proof.39XTpZ/source/internals/aihub/service \
  --artifact-root /private/tmp/p5-f421b-ecosystem-proof.39XTpZ/artifacts \
  --json
```

exit code 为 `1`，stdout 为 0 bytes。精确 terminal error 是：

```text
failed to parse service source control file /private/tmp/p5-f421b-ecosystem-proof.39XTpZ/source/internals/aihub/service/service.yml: http.routes: invalid type: sequence, expected struct HttpGatewayEntryAuthoring at line 9 column 5
```

fresh mirror 中 `aihub/service/service.yml` 的 `http` 仍使用：

```yaml
http:
  routes:
    - method: GET
      path: /health
      operation: handleAihubHttp
```

current canonical `ServiceManifestAuthoring.http` 要求由唯一 gateway entry key 索引的
`BTreeMap<GatewayEntryKey, HttpGatewayEntryAuthoring>`；这里的 `routes` value 是 sequence，
因此在任何 AIHub PackageArtifact、ServiceContract 或 ServiceDeployment 写入前严格拒绝。

最小 successor owner 是 **Internals AIHub service authoring owner**，范围以
`aihub/service/service.yml` 的 current canonical HTTP gateway authoring 收敛及随后重新运行
fresh N5 为起点。本文不声称 parser 停止点之后的 AIHub source/authoring 已通过，也不授权本
gate owner 修改它。

## 5. 实际 records、停止点与遮挡节点

fresh store 实际生成：

| record class | count |
| --- | ---: |
| PackageArtifact | 9 |
| FileIR | 116 |
| static resource | 0 |
| PackageSchemaIndex | 9 |
| PackageSchemaTypeRecord | 194 |
| ServiceContract | 3 |
| ServiceDeployment | 3 |
| RuntimeAssembly | 2（std bootstrap、Relay-only） |
| canonical record files 合计 | 336 |
| pointer JSON / lock | 15 / 15（均为本次 fresh store 新建） |
| command ledger entries | 12 |
| package publish attempts / success / failure | 9 / 8 / 1 |

最后成功阶段是同一 sibling wave 的独立闭合；AIHub failure 已收集后立即停止。被遮挡节点为：

```text
AIHub PackageArtifact / ServiceContract / ServiceDeployment
  -> Agine publish（未启动）
  -> complete RuntimeAssembly（未启动）
  -> full ecosystem pair / callback / mapping / consumer census（未启动）
  -> canonical mutation negatives（未启动）
  -> full reverse search（未启动）
```

因此 full ecosystem pair、callback、mapping、consumer 和 canonical mutation negative
实际执行数均为 `0`；这些是“未启动”的计数，不是生态中不存在对应记录的断言。Relay-only
阶段已独立验证的 mapping 数为 `3`。

## 6. 写入与环境边界

三个 production tree 在任务结束时仍为启动时 exact commit/tree 且 clean。没有使用
stable、live、instance、watch、MongoDB、旧 artifact store、旧 receipt、旧 lock 或 validator
waiver；没有修改 source mirror；没有派子 Agent；没有 merge、rebase 或 push。

唯一 tracked 写入是本文。本结果不写
`PHASE_05_ECOSYSTEM_PROOF_COMPLETE`，也不自行承接 successor。

## Appendix A：AIHub canonical command 完整 stderr

```text
error: package publish failed: warning: unused import: `super::*`
 --> compiler/source/src/package_rules/reserved_validation.rs:4:5
  |
4 | use super::*;
  |     ^^^^^^^^
  |
  = note: `#[warn(unused_imports)]` on by default

warning: unused import: `skiff_syntax::lexer::*`
 --> compiler/source/src/shared/lexer.rs:1:9
  |
1 | pub use skiff_syntax::lexer::*;
  |         ^^^^^^^^^^^^^^^^^^^^^^

warning: unused import: `skiff_compiler_core::package_export_resolver::*`
 --> compiler/source/src/shared/package_export_resolver.rs:1:9
  |
1 | pub use skiff_compiler_core::package_export_resolver::*;
  |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: unused import: `function_type_validation::collect_user_function_type_violations`
 --> compiler/source/src/source_rules.rs:4:9
  |
4 | pub use function_type_validation::collect_user_function_type_violations;
  |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: unused imports: `collect_stream_emit_expression_call_violations` and `collect_stream_emit_type_violations`
 --> compiler/source/src/source_rules.rs:6:5
  |
6 |     collect_stream_emit_expression_call_violations, collect_stream_emit_type_violations,
  |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: methods `with_module_path` and `with_source_types` are never used
  --> compiler/source/src/runtime_type_projection.rs:48:12
   |
37 | impl RuntimeBindings {
   | -------------------- methods in this implementation
...
48 |     pub fn with_module_path(mut self, module_path: &str) -> Self {
   |            ^^^^^^^^^^^^^^^^
...
53 |     pub fn with_source_types(mut self, ast: &SourceFile) -> Self {
   |            ^^^^^^^^^^^^^^^^^
   |
   = note: `#[warn(dead_code)]` on by default

warning: function `file_runtime_bindings` is never used
  --> compiler/source/src/runtime_type_projection.rs:68:8
   |
68 | pub fn file_runtime_bindings(ast: &SourceFile, policy: ProviderRuntimePolicy) -> RuntimeBindings {
   |        ^^^^^^^^^^^^^^^^^^^^^

warning: function `type_ref_descriptor` is never used
  --> compiler/source/src/runtime_type_projection.rs:72:8
   |
72 | pub fn type_ref_descriptor(ty: &TypeRef, runtime_bindings: &RuntimeBindings) -> TypeRefIr {
   |        ^^^^^^^^^^^^^^^^^^^

warning: function `collect_user_function_type_violations` is never used
 --> compiler/source/src/source_rules/function_type_validation.rs:6:8
  |
6 | pub fn collect_user_function_type_violations(
  |        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_operation_function_type_violations` is never used
  --> compiler/source/src/source_rules/function_type_validation.rs:48:4
   |
48 | fn collect_operation_function_type_violations(
   |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_function_type_name_violations` is never used
  --> compiler/source/src/source_rules/function_type_validation.rs:59:4
   |
59 | fn collect_function_type_name_violations(path: &str, ty: &str, violations: &mut Vec<String>) {
   |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_block_function_type_violations` is never used
  --> compiler/source/src/source_rules/function_type_validation.rs:72:4
   |
72 | fn collect_block_function_type_violations(path: &str, block: &Block, violations: &mut Vec<String>) {
   |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_stmt_function_type_violations` is never used
  --> compiler/source/src/source_rules/function_type_validation.rs:78:4
   |
78 | fn collect_stmt_function_type_violations(path: &str, stmt: &Stmt, violations: &mut Vec<String>) {
   |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_expr_function_type_violations` is never used
   --> compiler/source/src/source_rules/function_type_validation.rs:145:4
    |
145 | fn collect_expr_function_type_violations(path: &str, expr: &Expr, violations: &mut Vec<String>) {
    |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_db_operation_function_type_violations` is never used
   --> compiler/source/src/source_rules/function_type_validation.rs:227:4
    |
227 | fn collect_db_operation_function_type_violations(
    |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_db_query_function_type_violations` is never used
   --> compiler/source/src/source_rules/function_type_validation.rs:275:4
    |
275 | fn collect_db_query_function_type_violations(
    |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_stream_emit_expression_call_violations` is never used
  --> compiler/source/src/source_rules/stream_emit/mod.rs:35:8
   |
35 | pub fn collect_stream_emit_expression_call_violations(
   |        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_stream_emit_type_violations` is never used
  --> compiler/source/src/source_rules/stream_emit/mod.rs:53:8
   |
53 | pub fn collect_stream_emit_type_violations(
   |        ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_source_stream_emit_type_violations` is never used
   --> compiler/source/src/source_rules/stream_emit/mod.rs:154:4
    |
154 | fn collect_source_stream_emit_type_violations(
    |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_function_stream_emit_type_violations` is never used
   --> compiler/source/src/source_rules/stream_emit/mod.rs:195:4
    |
195 | fn collect_function_stream_emit_type_violations(
    |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: struct `StreamEmitTypeChecker` is never constructed
   --> compiler/source/src/source_rules/stream_emit/mod.rs:215:8
    |
215 | struct StreamEmitTypeChecker<'a> {
    |        ^^^^^^^^^^^^^^^^^^^^^

warning: multiple methods are never used
   --> compiler/source/src/source_rules/stream_emit/mod.rs:226:8
    |
225 | impl StreamEmitTypeChecker<'_> {
    | ------------------------------ methods in this implementation
226 |     fn check_block(&mut self, body: &Block) {
    |        ^^^^^^^^^^^
...
232 |     fn check_stmt(&mut self, stmt: &Stmt) {
    |        ^^^^^^^^^^
...
295 |     fn check_emit(&mut self, value: &Expr, value_key: &ExpressionKey) {
    |        ^^^^^^^^^^
...
327 |     fn check_expr(&mut self, expr: &Expr) {
    |        ^^^^^^^^^^
...
440 |     fn next_key(&mut self) -> ExpressionKey {
    |        ^^^^^^^^
...
446 |     fn peek_key(&self) -> ExpressionKey {
    |        ^^^^^^^^
...
454 |     fn check_db_query(&mut self, query: &crate::shared::ast::DbQueryBlock) {
    |        ^^^^^^^^^^^^^^

warning: function `collect_emit_expression_call_violations` is never used
  --> compiler/source/src/source_rules/stream_emit/types.rs:12:15
   |
12 | pub(super) fn collect_emit_expression_call_violations(
   |               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_emit_expression_call_violations_in_block` is never used
  --> compiler/source/src/source_rules/stream_emit/types.rs:95:15
   |
95 | pub(super) fn collect_emit_expression_call_violations_in_block(
   |               ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_emit_stmt_violations` is never used
   --> compiler/source/src/source_rules/stream_emit/types.rs:105:4
    |
105 | fn collect_emit_stmt_violations(
    |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_emit_db_operation_violations` is never used
   --> compiler/source/src/source_rules/stream_emit/types.rs:180:4
    |
180 | fn collect_emit_db_operation_violations(
    |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

warning: function `collect_emit_db_query_violations` is never used
   --> compiler/source/src/source_rules/stream_emit/types.rs:228:4
    |
228 | fn collect_emit_db_query_violations(
    |    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^

error: failed to parse service source control file /private/tmp/p5-f421b-ecosystem-proof.39XTpZ/source/internals/aihub/service/service.yml: http.routes: invalid type: sequence, expected struct HttpGatewayEntryAuthoring at line 9 column 5
```
