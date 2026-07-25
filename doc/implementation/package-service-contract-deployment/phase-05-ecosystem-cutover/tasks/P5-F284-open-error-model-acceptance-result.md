# P5-F284 Open error model A1 acceptance result

状态：`PASS`；A1 strict-model DTO冻结通过，无blocking finding。

## Exact candidate

- production candidate commit：
  `a052f02a4e5d52c96d01849fa7df076f00df0d94`
- production candidate tree：
  `4817f583de886d0b31e3fcf1141ac92532340bb0`
- 实际验收merge candidate HEAD：
  `8be32284c33d7881cac9964caf1063fda039c41e`
- 实际验收merge candidate tree：
  `7dbe77f14e0b743e79b22e6a1ba21490549003fd`
- merge parents：
  `cfb8568f8bbdf5d106f2ff0fdb65e059b4396f50`
  与
  `a052f02a4e5d52c96d01849fa7df076f00df0d94`

`git diff --name-status a052f02a..8be32284`只有F282 result、F283 task和F284 task三份文档；
排除该task目录后的diff为空。因此实际merge candidate相对production candidate没有额外production
变化。

## 独立验收矩阵

### 1. 设计与owner

| 检查项 | 结论 | 代码与测试证据 |
| --- | --- | --- |
| File IR declaration kind | PASS | `artifact-model/src/types.rs:151-163`以互斥variant区分`Record`、`Representation`、`Union`、`Alias`和`Interface`；`types::tests::declaration_descriptors_distinguish_all_canonical_kinds`覆盖五种wire kind。 |
| named union branch/context | PASS | `artifact-model/src/types.rs:165-191`定义concrete nominal、synthetic discriminator、literal三种branch输入；enclosing `TypeDeclIr`是named-union owner，branch保存fully-instantiated type argument map。`types::tests::named_union_preserves_all_branch_identity_inputs`和`tests::type_decl_ir_round_trips_named_union_branch_identity_input`覆盖strict round-trip。 |
| required throw/call site与required catch type | PASS | `artifact-model/src/executable.rs:120-149`定义required source/synthetic site与有限synthetic reason；statement throw、expression throw和`CallIr`分别在`:221-225`、`:385-389`、`:729-740`要求`site`；catch在`:393-398`要求非optional `catch_type`；rethrow仍只有既有exception slot。三个`executable::tests`覆盖round-trip、missing site、synthetic伪造/unknown reason及missing/null catch。 |
| runtime actual identity | PASS | `runtime/model/src/service_error.rs:6-143`以typed enum区分local execution、Package schema、platform builtin及named-union owner/branch；anonymous union直接沿用actual selected branch identity，不创建额外union identity。`runtime/model/src/value.rs:94-132`定义值与catch identity的canonical carrier。equal-shape nominal、primitive-backed representation、different enclosing union和clone handoff均有独立测试。 |
| request-local与opaque cause | PASS | `runtime/model/src/service_error.rs:298-327`严格decode并保存原始envelope bytes；`:329-419`把materialized local value与opaque service cause分开，并为当前request保存source、stack、correlation；opaque cause不暴露local catch identity。 |
| 唯一fixed envelope owner | PASS | 全仓production搜索只有`runtime/model/src/service_error.rs:145-296`定义public `ServiceErrorEnvelope`；shape精确为`PublicTypedError`、nested-payload `InternalError`和`PlatformError`。private `ServiceErrorEnvelopeWire`只用于同模块strict deserialize，不是第二个public owner。 |
| generic runtime diagnostic保持独立 | PASS | `runtime/model/src/error.rs:26-49`仍保留`RuntimeErrorPayload { code, message, status, details }`与generic `WirePayload`；字符串code `"InternalError"`没有被改成service catch type。 |

`PlatformBuiltinErrorIdentity`与F280审计出的有限registry一致，并明确不把
`std.resource.ResourceError`误归入platform allowlist。`InternalErrorPayload`只有
`message/traceId/errorId`，没有private type、display或arbitrary details入口。

### 2. 严格删除与版本

| 检查项 | 结论 | 代码与测试证据 |
| --- | --- | --- |
| 删除closed throw-set DTO | PASS | `artifact-model/src/package_artifact.rs:31-35`的`PackageCallableSignature`只剩parameters/return/maySuspend；`artifact-model/src/boundary/operation.rs:113-121`的operation没有`errors`，且`BoundaryErrorContract`定义已删除。限定`artifact-model/src/**`与`runtime/model/src/**`反向搜索不存在`throw_types`或`BoundaryErrorContract`。 |
| 旧field与optional shape strict reject | PASS | `package_artifact::tests::package_callable_signature_rejects_closed_throw_set_field`拒绝`throwTypes`；`boundary::tests::available_projection_wire_is_contract_agnostic_and_descriptor_is_strict`拒绝operation `errors`；executable负例拒绝missing/null catch、missing throw/call site、伪造source字段和unknown synthetic reason。新字段没有serde default、alias、legacy variant或dual read/write。 |
| envelope malformed strict reject | PASS | `ServiceErrorEnvelopeWire`及nested `InternalErrorPayload`均`deny_unknown_fields`，custom deserialize执行non-empty owner/payload/correlation校验；`service_error_envelope_strictly_rejects_invalid_wire`覆盖unknown variant、extra/missing field、empty owner、unknown platform identity与internal extra/missing字段。 |
| schema generation | PASS | `artifact-model/src/schema.rs:10-18`一次切到File IR v6、format v4、PackageArtifact v4、ServiceContract v4、ServiceContractDefinition v3；`schema::tests::open_error_channel_schema_versions_have_one_strict_generation`断言唯一新常量，authoring parser负例拒绝definition v2。没有旧version常量、compat reader或fallback在本节点新增。 |
| identity marker/prefix owner | PASS | candidate没有修改`artifact-identity/**`。当前File IR、PackageArtifact build/local ABI和ServiceProtocol marker/prefix仍分别是既有v5、v2/v1与v3 generation；本节点没有偷偷bump，也没有宣称新identity可生成。marker/prefix、preimage mutation tests及artifact admission必须由后续唯一artifact/contract consumer与删除旧consumer同批完成。 |

File IR、PackageArtifact与ServiceContract的raw model DTO保存schema string；canonical writer只生成上述新
generation，Package/contract的semantic admission继续由消费current constant的validator负责。File IR
identity/admission以及全部identity marker/prefix同步属于上一行明确的后续唯一consumer，不在A1 model
checkpoint内增加第二套version reader。

### 3. Scope与下游handoff

| 检查项 | 结论 | 证据 |
| --- | --- | --- |
| production scope | PASS | candidate的production变化只落在F281授权的`artifact-model`和`runtime/model`。`artifact-model/src/websocket_ingress.rs`仅删除旧error import/check；`boundary.rs`、`ecosystem_authoring.rs`、`file_ir/package_calls/tests.rs`、`file_ir/service_calls.rs`、`service_contract.rs`、`tests.rs`和`websocket_ingress/tests.rs`的变化均位于co-located test/fixture hunk。没有syntax/compiler、artifact-identity、deployment、其它runtime crate、router、telemetry、std/prelude、scripts或生态仓库写入。 |
| F278 facts保持 | PASS | `CallableProvenanceSummary::Analyzed.throw_origins`、`CallableMayEffects.throws_caller_alias`、`BoundaryEffectGuarantee.detached_error`与`no_same_heap_identity`仍存在；candidate没有修改其production定义或含义。`StmtIr`/`ExprIr`/`TestEffectOutcomeIr`的throw `payload_type`也仍保留。 |
| 下游break清单 | PASS | 反向搜索暴露真实旧consumer，没有compat层隐藏：language/lowering仍需构造declaration kind/branch、required site与catch type；artifact/contract仍有closed-set producer/normalizer/validator/schema-root和identity preimage；runtime仍需接入value carrier、identity index、catch/channel/codec/opaque forwarding；compiler/deployment/runtime/test-runner fixture仍需机械迁移。 |
| 单次适配能力 | PASS | declaration/branch/site/catch、catch identity、exception cause与fixed envelope均已有唯一canonical owner；后续consumer不需要重新决定公共DTO。 |

精确handoff如下：

1. language/std/lowering consumer必须实现`CatchLeaves`、throw/rethrow legality、五种declaration lowering、
   named/anonymous union actual branch、required throw/call site及required catch type，并删除
   `ErrorPayload` marker；不能恢复optional site/catch或旧descriptor。
2. artifact/contract consumer必须删除compiler projection/input/compiled、artifact-identity、
   compiler-contract、deployment与test-runner中的`throw_types`/`BoundaryErrorContract`生产、归一化、
   validation和schema-root读取；同一owner同步bump File IR、Package build/local ABI与
   ServiceProtocol identity marker/prefix并补mutation/admission证据。
3. runtime identity/channel consumer必须把`RuntimeValueCarrier`接入真实slot、heap/container、assignment与
   call路径，构建assembly-owned双向`ServiceErrorTypeIndex`，并以同一channel实现local catch、
   public/private export、linked import与opaque原样转发。当前`RuntimeObjectFields`/`HeapNode`仍使用旧
   `RuntimeValue`是已暴露的W2-R consumer break，不是本节点的heap实现范围，后续不得以shape/display或
   fallback补洞。
4. transport/host/router/telemetry consumer随后把service response切到fixed envelope并保留generic
   control/internal error路径；compiler、deployment、runtime和test-runner中的大量手写fixture需按新
   constructor机械迁移。

## 独立探针

实际执行：

```text
cargo test -p skiff-artifact-model --lib -- --list
  143 tests, 0 benchmarks

cargo test -p skiff-runtime-model --lib -- --list
  76 tests, 0 benchmarks

cargo test -p skiff-artifact-model --lib --no-fail-fast
  143 passed; 0 failed

cargo test -p skiff-runtime-model --lib --no-fail-fast
  76 passed; 0 failed

git diff --check
  PASS（无输出）

git diff --check a052f02a^ a052f02a
  PASS（无输出）
```

没有运行workspace/compiler/runtime完整测试、`pnpm verify`、fmt、instance、live、生态publish或chat
smoke。两个model crate完整suite既覆盖指定高风险shape，也确认列表非零；fmt沿用candidate冻结前已有证据，
本次production未变化，未机械重跑。

## Verdict与设计缺口

`PASS`。没有blocking finding，没有失效的developer evidence，也没有新增用户设计决策。

尚未迁移的heap/container、compiler/artifact/runtime/wire consumer与identity marker是F280/F281已明确分配
的实现handoff，不是共享DTO设计缺口。后续consumer必须适配本结果冻结的shape；若需要改变这些公共DTO，
应退回A1 checkpoint重新验收，不能在下游引入dual path、legacy adapter、display/shape inference或
fallback。
