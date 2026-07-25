# P5-F293 Generic nominal TypeRef owner audit result

状态：审计完成；推荐一个唯一 `appliedNominal` 形状。它可以在不改写 F286 当前 dirty
compiler 文件的前提下先形成 shared DTO/identity checkpoint。Package/Public schema 的 generic
instantiation 本轮明确 fail closed，不允许以丢弃 arguments 的方式进入 boundary。

## Verdict

1. `TypeRefIr` 必须新增唯一 `AppliedNominal` variant；不应把 optional arguments 分别加到
   `LocalType`、`PublicationType`、`ServiceSymbol`、`PackageSymbol` 和 `PackageSchema`。
2. `AppliedNominal.arguments` 是按 declaration `type_params` 顺序排列的 required、non-empty
   `Vec<TypeRefIr>`。零参数引用继续使用原 nominal variant；同一个实例不能有第二种合法表示。
3. base 使用独立的 closed enum，只能表达上述五类 nominal locator。primitive/container、
   structural type、`AnyInterface`、transparent alias、interface declaration、actor、DB object 和
   File IR中的伪造 raw address 都不能成为 applied base。linked counterpart只在完成 exact owner
   resolution后允许内部 `Address` base。`PackageSchema` 虽可由共享 DTO 无歧义表达，本轮
   projection/admission 仍必须显式拒绝。
4. `NamedUnionBranchIr::ConcreteNominal.type_arguments` 必须删除。branch 的完整参数只保存在
   `nominal_type: TypeRefIr` 的 `AppliedNominal.arguments`；enclosing union 的参数只保存在 applied
   union owner。旧 map 和新 list 不得共存。
5. F288 刚切换的 File IR、PackageArtifact 和相关 identity generation 必须再次 bump。不能在
   `v6/v4` File IR 或 `v4` PackageArtifact 的名字下静默改变 wire/preimage。
6. 最早不可逆损失已经发生在
   `compiler/source/src/type_resolution_model.rs::resolve_named_type`：函数完成了 argument resolution
   和部分 arity 检查，却返回不带 arguments 的 nominal ref。独立的
   `compiler/lowering/src/type_lowering.rs` 路径随后又拒绝 generic local type，并对 external nominal
   丢弃 arguments。这两处必须一起关闭，不能从 `source_text` 恢复。
7. 当前 `LinkedTypeRef::Address` 是第二个不可逆损失点；当前 runtime slot、heap、container 和 call
   仍保存裸 `RuntimeValue`，是 actual identity 的第三个损失点。只修 compiler/File IR 不足以满足
   catch identity。

## 唯一 canonical shape

推荐冻结以下语义等价的 Rust shape；字段名也是 wire 合同：

```rust
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum NominalTypeRefBaseIr {
    LocalType { type_index: u32 },
    PublicationType { module_path: String, type_index: u32 },
    ServiceSymbol { symbol: ServiceSymbolRef },
    PackageSymbol { symbol: PackageSymbolRef },
    PackageSchema {
        package_id: String,
        stable_schema_key: String,
        package_schema_type_id: PackageSchemaTypeId,
    },
}

pub enum TypeRefIr {
    // existing variants remain
    AppliedNominal {
        base: NominalTypeRefBaseIr,
        // required on the wire and rejected when empty
        arguments: Vec<TypeRefIr>,
    },
}
```

示例 wire：

```json
{
  "kind": "appliedNominal",
  "base": {
    "kind": "packageSymbol",
    "symbol": {
      "package": { "kind": "dependency", "dependencyRef": "errors" },
      "symbolPath": "ResultBox",
      "abiExpectation": "..."
    }
  },
  "arguments": [
    { "kind": "builtin", "name": "string" }
  ]
}
```

`arguments` 不能使用 `#[serde(default)]` 或 `skip_serializing_if`。derive serde 外再加
`deserialize_non_empty_type_arguments` 或等价 custom deserializer，使 missing、`null`、empty、unknown
field 均在 decode 时失败。语义 validator 再执行 owner/descriptor/arity 检查；serde 不负责解析外部
owner。

### Structural 与 contextual invariant

| 层级 | 必须检查 |
| --- | --- |
| DTO/serde | base 只能来自 `NominalTypeRefBaseIr`；`arguments` required 且 non-empty；unknown field/variant 拒绝；plain nominal 上伪造 `arguments` 因 `deny_unknown_fields` 被拒绝。 |
| declaration validation | plain nominal 只能指向 `type_params.is_empty()` 的 declaration；`AppliedNominal` 必须指向 `type_params` 非空的 nominal `Record`、`Representation` 或 named `Union`，且 argument 数量精确相等。 |
| illegal target | `Alias` 先展开且不能成为 applied base；`Interface`、actor、DB object、primitive/container、anonymous record/union、literal、nullable、function、`TypeParam` 和 `AnyInterface` 均不能成为 base。 |
| ordering | source position `arguments[i]` 对应 declaration `type_params[i]`。构造 substitution 时才 zip 为 name-to-value map；map 不是 artifact identity 输入。 |
| owner | local index 必须在 owning file；publication ref 必须在同一 exact publication/file graph；service/package symbol 必须解析到 exact dependency/artifact owner及 symbol；不得按短名、后缀或 shape 选择。 |
| type parameters | generic type/executable descriptor及body中的 applied arguments可以含 enclosing scope实际绑定的 `TypeParam` placeholder；scope validator拒绝未绑定名称。call-site/runtime type plan完成 substitution后，执行中的 construct、throw payload、catch/pattern actual type不得残留 `TypeParam`。 |
| public boundary | 当前 generation 对 applied `PackageSchema`、从 service API materialized 为 `PackageSymbol` 的 generic schema，以及任何进入 PackageSchema closure 的 generic declaration/use 全部 fail closed。 |

`PackageSymbolRef::abi_expectation` 和 dependency/package binding 是 owner 证据的一部分；canonical rebinding
必须先解析 exact package owner，再递归 canonicalize ordered arguments。不能把 `symbol_path` 单独当 owner。

### Linked shape 与 runtime argument identity

`runtime/linked-program` 应镜像同一 shape：

```text
LinkedTypeRef::AppliedNominal {
  base: LinkedNominalTypeRefBase {
    LocalType | PublicationType | ServiceSymbol | PackageSymbol |
    PackageSchema | Address
  },
  arguments: Vec<LinkedTypeRef>
}
```

pre-link base 先按 owner 解析，arguments 递归 link，最后只把 base 改成 `Address`；不能把整个
`AppliedNominal` 替换成裸 `Address`。plain non-generic nominal 仍可变成现有 `Address`。post-link validator
必须重新检查 target descriptor、arity 和 unresolved parameter。

当前 `InstantiatedTypeArgumentIdentity(String)` 的任意 public string constructor 不足以证明 canonical。
本 checkpoint 应把它冻结为 typed recursive identity（或只允许从该 typed preimage 构造的 opaque hash）：

```text
CanonicalRuntimeTypeIdentity
  = Builtin(name, ordered arguments)
  | Nominal(exact LocalExecution or PackageSchema identity)
  | Record(sorted field name -> identity)
  | Union(canonical ordered identities)
  | Nullable(identity)
  | Literal(canonical literal)
  | AnyInterface(exact interface ABI identity, ordered arguments)
  | Function(ordered parameter identities, return identity)
```

其中 nested nominal 再递归保存自己的 ordered arguments。`TypeParam`、unlinked symbol、alias spelling、
display/source text 和 raw address 没有 canonical argument identity。`TypeAddr` 只可作为 exact linked
image 内 `LocalExecutionTypeIdentity` 的 declaration-owner 部件，并必须与 ordered recursive arguments
结合；不能用它替代 applied identity，也不能跨 activation 当 ABI identity。

### 被否决的备选

| 备选 | 否决原因 |
| --- | --- |
| 给五个 existing nominal variant 各加 `arguments` | 同一 invariant、traversal 和 validation 被复制五次；所有零参数 wire 也改变；F286 fixture 冲突面最大。 |
| `Applied { base: Box<TypeRefIr>, ... }` | model 本身允许 `Array<T>`、primitive、nullable、interface、alias 或另一个 applied wrapper 成为 base；每个 consumer 都要重复否定。 |
| declaration-param keyed `BTreeMap<String, TypeRefIr>` | parameter rename 会无谓改变 identity；map 掩盖 declaration order/arity；也是当前 named-union 重复输入的来源。 |
| 继续使用 `Builtin { name, args }` 表达用户 nominal | owner 丢失；同名、同 shape 或跨 package type 会错误相等。 |
| 只在 `NamedUnionBranchIr` 保存 arguments | 普通 generic nominal、enclosing generic union、construct/throw/pattern/container 均仍丢参数。 |
| 从 display/source text、short name 或 runtime shape 恢复 | 这些都不是 canonical owner 事实；source text 在 lowering 后不可用，shape 违反 nominal identity。 |
| post-link 只保存 `Address` | `Box<string>` 与 `Box<number>` 会退化成同一 declaration address；跨 activation 也不是 ABI identity。 |
| 暂时拒绝所有 generic nominal | 与现有静态语义允许 fully-instantiated generic nominal/pattern 冲突；只允许对尚未定义的 public schema boundary 明确 fail closed。 |

## Named union 与 actual identity

`ConcreteNominal` 应收敛为：

```rust
pub enum NamedUnionBranchIr {
    ConcreteNominal { nominal_type: TypeRefIr },
    SyntheticDiscriminator { /* unchanged */ },
    Literal { /* unchanged */ },
}
```

例如 `type Outcome<T> = Ok<T> | { kind: "retry" } | "cancelled"`：

- declaration descriptor 内的 concrete branch 保存
  `AppliedNominal(Ok, [TypeParam("T")])`；
- actual `Outcome<string>` 的 enclosing owner保存 `[string]`；
- closure/linker以 enclosing substitution 把 branch 变为 `Ok<string>`；
- 没有独立 `typeArguments: {"T": ...}`，因而不存在 list/map 顺序冲突或双方不一致。

| 类型情形 | actual identity |
| --- | --- |
| ordinary generic record | `CatchIdentity::Nominal(LocalExecutionTypeIdentity { declaration owner, ordered canonical argument identities })`。跨 public boundary 尚不开放 generic PackageSchema identity。 |
| generic representation | 与 ordinary nominal 相同，identity 是外层 representation declaration owner + arguments；primitive/record payload shape不参与替代外层 identity。 |
| generic named union concrete branch | `CatchIdentity::NamedUnionBranch { union: enclosing applied union identity, branch: ConcreteNominal { identity: applied concrete branch identity } }`。 |
| generic named union synthetic branch | 同一 applied union owner + canonical discriminator field/value；owner arguments参与 identity。 |
| generic named union literal branch | 同一 applied union owner + canonical literal；owner arguments参与 identity。 |
| anonymous union | 不创建 enclosing identity，沿用 actual selected branch identity。 |
| alias | 在 File IR/contract usage descriptor和 runtime identity产生前透明展开；alias本身没有 identity。当前 alias 无 type parameters，任何 applied alias 都是错误。 |

因此即使 concrete branch 相同，`Outcome<string>` 与 `Outcome<number>` 仍因 enclosing owner arguments
不同而不相等；即使两个 representations payload shape相同，也因 declaration owner不同而不相等。

## Source 到 runtime 的唯一链

| 阶段 | canonical 动作 | 当前缺口 |
| --- | --- | --- |
| source resolution | resolve exact declaration、递归 resolve arguments、按 declaration order验证 arity，直接构造 plain/applied `TypeRefIr` | `resolve_named_type` 已计算 `resolved_args`，但 Local/Service/Package 返回值丢弃它；interface traversal还从 `source_text` 重解析 arguments。 |
| lowering | 只消费 structured resolution或使用同一 nominal constructor；所有 TypeRef-bearing IR site共享此结果 | `lower_named_type` 对 generic local报 unsupported，对 package/service path返回 bare symbol。 |
| File IR | 所有 declaration、signature、construct、throw/catch/pattern、DB、container nested ref保存同一 DTO | 当前 nominal variants没有 arguments；named union另存 map。 |
| artifact identity/rebinding | canonical JSON直接包含 applied base + ordered arguments；rebind base owner及每个 argument | 当前 hash只能忠实 hash 已经丢失的 bare ref。 |
| linked program | 保留 applied wrapper，递归 link args，只替换 base locator | 当前 `code_linker` 把 nominal ref整体替换成 bare `Address`。 |
| type plan | descriptor以 declaration params到arguments substitution实例化；产生 typed runtime argument identities和 `CatchIdentity` | 当前 plan/traversal只认识 bare Address/legacy descriptor。 |
| runtime value | construct产生 `RuntimeValueCarrier`；slot、assignment、field、array/map、call/return/stream、throw/rethrow和pattern按 move/clone保存 carrier | 当前 `RuntimeObjectFields`、`RuntimeMap`、`HeapNode::Array`、eval slots和 call args使用裸 `RuntimeValue`，已冻结 carrier尚未接入。 |
| throw/catch | throw读取 actual carrier identity；catch leaves来自 fully-instantiated linked type，按 exact `CatchIdentity`比较 | 当前 `runtime/eval/src/exceptions.rs`仍用旧 `TypeIdentity`并从 static type/shape重建。 |

`TypeDescriptorIr` 的五种 canonical kind和 `NamedUnionBranchIr` 也必须原样进入 linked-program。当前
`LinkedTypeDescriptor` 仍只有 `Record/Alias/Union { variants }`，会丢失 representation、interface和
named-union branch context；runtime lane必须与 applied ref 一起迁移，不能继续把 representation当 alias。

File IR/linked traversal必须显式覆盖全部 TypeRef-bearing site，而不是只覆盖 declaration：

- `TypeDeclIr.descriptor/implements`、interface operation params/return/self、constant和 executable
  params/return/self；
- `StmtIr::ForIn.item_type`、statement/expression/test-effect throw `payload_type`、test-effect
  request/value/item type、actor self field type；
- `PatternIr::Type`、`ExprIr::Construct.type_ref`、required catch type、interface box arguments；
- DB target/result/key/field type和 `CallIr.type_args`；
- 上述任何 ref 内的 builtin/container、record/union、nullable、function、interface及 applied nested
  children。

## Identity / version matrix

推荐的本轮 generation如下。数值以当前 F288 已提交 generation为起点：

| Domain | 当前 | 本轮 | 结论与原因 |
| --- | --- | --- | --- |
| File IR schema | `skiff-file-ir-v6` | `skiff-file-ir-v7` | 必须 bump；新增 strict wire variant并删除 branch map。 |
| File IR format | `skiff-file-ir-format-v4` | `skiff-file-ir-format-v5` | 必须 bump；execution type representation改变。 |
| File IR identity prefix | `skiff-file-ir-v6:sha256` | `skiff-file-ir-v7:sha256` | 必须 bump；ordered arguments进入 canonical preimage。 |
| opcode table | `skiff-opcode-table-v1` | 保持 | 没有 opcode encoding改变。 |
| PackageArtifact schema | `skiff-package-artifact-v4` | `skiff-package-artifact-v5` | 必须 bump；Local ABI/implementation signatures可携带新 `TypeRefIr`。 |
| PackageArtifact Local ABI marker | `skiff-package-artifact-local-abi-identity-v2` | `...-v3` | 必须 bump；public package callable type identity可区分同 declaration不同 arguments。 |
| PackageArtifact Local ABI prefix | `skiff-package-local-abi-v4:sha256` | `...-v5:sha256` | 必须 bump。 |
| PackageArtifact Build marker | `skiff-package-artifact-build-identity-v3` | `...-v4` | 必须 bump；build包含 files、implementation signatures和 Local ABI。 |
| PackageArtifact Build prefix | `skiff-package-build-v5:sha256` | `...-v6:sha256` | 必须 bump。 |
| legacy Package Unit build/local ABI domains | 既有 v2 domains | 保持 | 不是当前 PackageArtifact identity owner，不借机重开。 |
| PackageSchema type marker/prefix | v1 / v1 | 保持 | 本轮 generic PackageSchema fail closed，`ContractTypeRef`和canonical descriptor ref shape不变。 |
| PackageSchema index marker/prefix | v1 / v1 | 保持 | 同上；不得让 generic declaration/use进入index/closure。 |
| ServiceContract schema | `skiff-service-contract-v4` | 保持 | public generic ref未开放。 |
| ServiceContractDefinition schema | `...-v3` | 保持 | authoring/contract DTO未改变。 |
| ServiceProtocol marker/prefix | v4 / v4 | 保持 | generic public operation/schema被拒绝，protocol preimage无新shape。 |
| ContractOperation identity | v1 / v1 | 保持 | operation stable key算法与signature无关。 |
| package/service human version label | 非identity | 保持 | 仍不进入任何type identity。 |

同 logical non-generic artifact也只由新 strict writer产生新 File IR/PackageArtifact generation；不保留旧
reader、serde alias、default arguments或 dual hash。

如果未来选择支持 public generic schema，则必须另开公共设计节点，同时：

- 为 `ContractTypeRef::PackageSchema`、`PackageTypeRef::PackageSchema` 和
  `PackageSchemaTypeIdentity` 增加唯一 ordered arguments；
- 明确 `PackageSchemaTypeId` 是 generic declaration identity，applied identity是
  declaration id + ordered argument identities；
- 让 `ServiceErrorEnvelope::PublicTypedError` 携带 exact applied identity，而不是只传 declaration id；
- 同批重新审计/bump PackageSchema Type/Index、ServiceContract/Definition和 ServiceProtocol domains。

这些变化不能偷渡到当前保持项。

## Production owner 与写入边界

### Shared DTO / artifact identity owner

最小 shared checkpoint 的 production 写入只落在：

- `artifact-model/src/types.rs`：base enum、`AppliedNominal`、non-empty strict decode、
  `ConcreteNominal`去重和 co-located structural validation；
- `artifact-model/src/lib.rs`：唯一 public export；
- `artifact-model/src/schema.rs`：File IR/PackageArtifact strict generation；
- `artifact-model/src/cross_package_identity.rs`：递归 rebind base与ordered arguments；
- `artifact-model/src/actor_declaration.rs`：actor-ref rejection递归进入 applied arguments并拒绝actor base；
- `artifact-model/src/file_ir.rs`、`artifact-model/src/file_ir/service_calls.rs`：File IR admission/traversal
  接入 canonical validator；
- `artifact-identity/src/constants.rs`：上述 identity marker/prefix；
- `artifact-identity/src/file_ir.rs`：generation/admission及 argument mutation identity evidence；
- `artifact-identity/src/semantic.rs`：canonical semantic helpers不得漏过 applied arguments；
- `artifact-identity/src/package_artifact.rs`、
  `artifact-identity/src/package_artifact/projection.rs`、
  `artifact-identity/src/package_artifact/validation.rs`：Local ABI/build generation与validation；
- `artifact-identity/src/package/projection/implementation_links.rs`：implementation signature中的
  TypeRef保持完整；
- `artifact-identity/src/lib.rs`、`artifact-identity/src/ecosystem_paths.rs`：新 prefix唯一导出/路径验证。

`artifact-model/src/executable.rs`、`package_artifact.rs`、`package_unit.rs`、`publication_abi.rs`、
`recoverable.rs`、`service_unit.rs` 和 `contract_types.rs` 是 TypeRef-bearing/admission surface；如果没有
exhaustive match或schema字段变化，不应为了 fixture churn改 production shape。所有这些 surface仍必须由
shared traversal测试证明 nested applied ref不丢失。

### Language consumer owner

以下是一个不可拆开的 compiler owner；由正在运行的 F286 在 shared checkpoint 后续接：

- `compiler/core/src/type_ref.rs`；
- `compiler/core/src/type_closure/{mod.rs,path.rs,source.rs}`；
- `compiler/core/src/{type_graph.rs,db_projection.rs,package_interface_methods.rs,spawn_targets.rs}`；
- `compiler/source/src/type_resolution_model.rs`及
  `type_resolution_model/{shape_assignability.rs,catch_leaves.rs}`；
- `compiler/source/src/{expression_type_model.rs,runtime_type_projection.rs,semantic/interface.rs,source_file_facts.rs}`；
- `compiler/source/src/expression_type_model/{expression_assignability.rs,db_projection.rs,contract_call_typing.rs,contract_call_typing/type_projection.rs}`；
- `compiler/source/src/contract_type_resolution/{types.rs,interfaces/substitution.rs}`；
- `compiler/source/src/{callable_effects/transfer.rs,resolved_call_targets/builder.rs,lib.rs}`；
- `compiler/lowering/src/type_lowering.rs`、`declaration_lowering.rs`、
  `source_file_lowering.rs`、`function_lowering.rs`；
- `compiler/lowering/src/function_lowering/{object_literal.rs,object_literal/fact_validation.rs}`；
- `compiler/lowering/src/{lowered.rs,publication_local_refs.rs,executable_type_projection.rs,external_refs.rs,db_lowering.rs,type_inference.rs}`；
- `compiler/lowering/src/{entrypoint_abi.rs,entrypoint_abi_model.rs,executable_declaration_lowering.rs}`；
- `compiler/lowering/src/file_ir/{identity.rs,types.rs}`。

该 owner必须让 `map/walk/type_ref_children/substitute` 递归 `AppliedNominal.arguments`，增加稳定 visit-path
segment；type closure以 base declaration建立 substitution并实例化 descriptor。所有当前
`resolved_type_arg_texts` 语义读取必须删除；source text只可保留诊断显示。

### Package/public compiler consumer owner

- `compiler/compiled/src/{package_callable_signatures.rs,projection_input.rs}`；
- `compiler/projection-input/src/{lib.rs,package_callable_signatures.rs}`；
- `compiler/projection/src/package_artifact/{api_exports.rs,visible_types.rs,schema.rs}`；
- `compiler/projection/src/package_artifact/boundary/types.rs`；
- `compiler/projection/src/package_artifact/callables/normalization.rs`；
- `compiler/projection/src/package_artifact/export_links/public_instances/{mod.rs,interfaces.rs}`；
- `compiler/driver/source_compile/canonical_dependencies.rs`；
- `artifact-identity/src/contract.rs`的PackageSchema/contract loaded-artifact fail-closed admission。

该 lane递归保存 package-local applied refs，并在 PackageSchema/public/service/error boundary显式
fail closed。`PackageSchemaCanonicalDescriptor.type_params` 的存在不能被误读为 applied ref已经有 wire
identity。generic boundary callable应得到确定的 unavailable/diagnostic，不得被降成 bare
`PackageSchema`。

### Link/type-plan owner

- `runtime/linked-program/src/{linked.rs,types.rs,type_params.rs,package_unit.rs,lib.rs}`；
- `runtime/linker/src/linker/{file_conversion.rs,call_semantic_validation.rs,link_diagnostics.rs}`；
- `runtime/linker/src/assembly_execution/code_linker.rs`和`runtime/linker/src/lib.rs`；
- `runtime/linked-type-plan/src/{type_plan.rs,native_call_plan.rs,http_plan.rs}`。

这一 lane同时拥有 linked descriptor parity、所有 declaration/signature/body TypeRef site、recursive
substitution、pre/post-link validation和 runtime canonical argument identity生成。不能只补 enum match 后把
wrapper丢掉。

### Runtime value/catch owner

- canonical model：
  `runtime/model/src/{service_error.rs,value.rs,type_plan.rs,error.rs,request_heap.rs,runtime_value.rs,runtime_value_graph.rs,callback_projection.rs}`；
- slots/construct/throw/catch/pattern：
  `runtime/eval/src/{env.rs,eval_context.rs,exceptions.rs,program_ir.rs,flow_completion.rs,program_mutation.rs,mutable_path.rs}`；
- field/container/value operations：
  `runtime/eval/src/{runtime_ops.rs,runtime_value_view.rs,type_descriptor.rs,type_projection.rs,program_types.rs}`；
- call/return/stream：
  `runtime/eval/src/{program.rs,program_execution.rs,program_invocation.rs,program_stream.rs,invocation.rs,invocation_builder.rs,native_invocation.rs,receiver_methods.rs,http_adapter.rs,websocket_adapter.rs}`；
- actor/service/boundary handoff：
  `runtime/eval/src/{actor_dispatch.rs,actor_executor.rs,actor_instance.rs,service_dispatch.rs,binary_http_boundary.rs,capabilities.rs,entrypoint.rs,db_command.rs,db_eval.rs,program_db.rs,spawn_ops.rs,recoverable_behavior.rs,recoverable_spawn_payload.rs,test_effect_registry.rs,lib.rs}`以及
  `runtime/eval/src/assembly_execution/{mod.rs,ordinary.rs,ingress.rs,boundary_materialization.rs,callback_native.rs,async_stream_cancel.rs,projection.rs,websocket_contract_plan.rs,websocket_ingress.rs,websocket_response.rs}`；
- nested DB/recoverable refs：
  `runtime/boundary/src/db.rs`和`runtime/service-db/src/metadata.rs`。

`RuntimeValueCarrier` 要成为 slot、object field、map key/value、array element、call argument/return和
stream item的唯一身份传递单元；map内部可以使用专用 key projection，但从 representation key投影和取回时
必须保存carrier identity。ordinary helpers不能调用 `into_value` 后遗失 identity。construct创建外层
identity，assignment/container/call只 move/clone；projection/unbox才显式改变外层 carrier。throw以
actual carrier为权威，static `payload_type`只用于 artifact validation；nominal pattern/catch比较 exact
carrier identity。

### F286 并行冲突

审计时 F286 worktree `/Users/geek/workspace/skiff-p5-f286-error-language` 位于 `a608d43e26d5`，dirty
范围正是 `compiler/core`、`compiler/source`、`compiler/lowering`和一个 compiler integration fixture，
包括最早损失文件 `type_resolution_model.rs`。因此：

```text
compiler/core/src/
  spawn_targets.rs
  type_closure/{mod.rs,path.rs,tests.rs}
compiler/source/src/
  lib.rs
  contract_dependency_test_fixture.rs
  contract_type_resolution.rs
  expression_type_model.rs
  expression_type_model/{contract_call_typing.rs,contract_call_typing/tests.rs,
                         expression_assignability.rs}
  runtime_type_projection.rs
  type_resolution_model.rs
  type_resolution_model/{shape_assignability.rs,catch_leaves.rs (untracked)}
compiler/lowering/src/
  db_lowering.rs
  declaration_lowering.rs
  executable_declaration_lowering.rs
  external_refs.rs
  file_ir/identity.rs
  function_lowering.rs
  lowered.rs
  publication_local_refs.rs
  source_file_lowering.rs
  source_file_lowering/interface_execution_tests/contract_fixture.rs
compiler/tests/std_package_imports.rs
```

- shared checkpoint不得修改任何 `compiler/**`；
- package/public lane和runtime lane不得修改 F286 dirty文件；
- F286在 shared checkpoint合入后 rebase/merge该 checkpoint，并独占所有 language consumer适配；
- 广域 fixture更新不得提前进入任一 production lane。

## 最短实现 DAG

```text
S0 shared DTO + artifact identity generation
├── S1 F286 language/core/source/lowering continuation
│   └── S2 package-local ABI + projection + explicit public-schema fail-close
└── S3 linked-program + linker + linked-type-plan
    └── S4 runtime carrier + construct/pattern/throw/catch

(S2, S4) ──> S5 single mechanical fixture/golden refresh ──> A combined acceptance
```

### S0 — shared checkpoint

只修改上列 `artifact-model`/`artifact-identity` owner。冻结 wire、branch去重、strict validators和所有新
generation。它可以先于 F286完成且没有文件重叠。S0合入后，所有 consumer都只适配这一种 shape。

### S1 — F286 continuation

F286先续接 structured arguments，再完成 CatchLeaves/lowering；不得新增临时 wrapper或从 source text
重建。S1独占所有 compiler core/source/lowering production与其 focused tests。

### S2 — package/public consumer

在 S1 后验收 package-local callable、cross-package owner和projection。实现工作可在独立 worktree中基于
S0与S1并行准备，但最终必须 rebase S1再验收。当前 policy是 generic PackageSchema/public error
fail closed。

### S3/S4 — runtime branch

S3可与 S1并行，因为没有 production文件重叠；它先保住 linked arguments和descriptor kind。S4随后把
carrier接入所有 value handoff，并删除旧 `TypeIdentity`/shape reconstruction路径。S4不能在 S3前用
display或 Address补洞。

### S5 — 唯一机械 owner

S2/S4 production稳定后，由一个 owner统一刷新：

- S0未覆盖的跨crate schema string、strict wire、mutation和identity golden；
- compiler source/lowering/projection及 `compiler/tests/**` fixtures；
- runtime linker/linked-program/eval/host/loader/package-test fixtures；
- 所有 schema string、File IR id、Package build/local ABI golden。

S0自身必须先更新并通过其 co-located model/identity focused tests；S5不回写这些测试的语义断言，只在
combined graph稳定后刷新剩余机械预期。S5不得修改 production语义；若编译暴露漏掉的 exhaustive
match，退回对应 production lane修复。

## 最小验收矩阵

### Positive

1. 同一 declaration `Box<string>` 与 `Box<number>` 的 File IR bytes、File IR identity、linked argument
   identity及 runtime `CatchIdentity`均不同。
2. `Outer<Box<string>, Array<Id>>` round-trip、rebind、link、substitute后保留完整递归顺序。
3. `type Token<T> = string` 作为 representation时，`Token<string>`保留外层 owner，且不等于
   `string`、`Token<number>`或另一个同payload representation。
4. generic named union同时覆盖 concrete `Branch<T>`、synthetic discriminator和literal branch；
   `U<string>`与`U<number>`的所有branch identity均因 enclosing owner不同。
5. 两个package导出相同 `symbol_path` 和 shape的 generic nominal，exact dependency/package owner不同，
   rebind/link/catch identity不同。
6. construct后的值依次经过 slot assignment、record field、array/map、local call return和throw，carrier
   identity不变；matching exact applied catch成功，另一个 argument的catch不匹配。
7. alias到 applied nominal时只得到target identity，alias spelling不进入 File IR actual identity或runtime
   catch identity。

### Negative

1. missing、`null`、empty、excess arguments；plain generic nominal；applied non-generic nominal。
2. unresolved argument、完成link后残留 `TypeParam`、unknown local index、unresolved package/service
   owner、ABI expectation不匹配。
3. primitive/container/record/union/nullable/literal/function/`AnyInterface`/alias/interface/actor/DB object
   作为 applied base。
4. 当前 generation 的 applied `PackageSchema`、generic package/public schema closure、generic
   `PublicTypedError` export。
5. tampered wire：旧 `ConcreteNominal.typeArguments`、新 branch同时带重复map、plain base附加
   `arguments`、applied base附加unknown field、arguments reorder。reorder是另一 identity而非同义 wire。
6. tamper任一 base owner或nested argument后，旧 File IR identity、Package Local ABI和Package Build
   identity均验证失败；旧 generation prefix/schema也失败。
7. 同shape不同 nominal owner、同 Address不同 arguments、同 concrete branch不同 enclosing generic union
   均不相等。
8. runtime slot/container/call路径若剥离 carrier，focused handoff test必须立即失败，而不是从 static
   payload type重建。

## Focused commands

每个 lane使用真实 crate名；本审计不执行这些命令。

```bash
# S0
cargo test -p skiff-artifact-model --lib --no-fail-fast
cargo test -p skiff-artifact-identity --lib --no-fail-fast

# S1
cargo test -p skiff-compiler-core --lib --no-fail-fast
cargo test -p skiff-compiler-source --lib --no-fail-fast
cargo test -p skiff-compiler-lowering --lib --no-fail-fast
cargo test -p skiff-compiler --test file_ir_execution_type_representation --no-fail-fast
cargo test -p skiff-compiler --test package_imports --no-fail-fast

# S2
cargo test -p skiff-compiler-compiled --lib --no-fail-fast
cargo test -p skiff-compiler-projection-input --lib --no-fail-fast
cargo test -p skiff-compiler-projection --lib --no-fail-fast
cargo test -p skiff-compiler --test test_artifact_identity --no-fail-fast

# S3/S4
cargo test -p skiff-runtime-model --lib --no-fail-fast
cargo test -p skiff-runtime-linked-program --lib --no-fail-fast
cargo test -p skiff-runtime-linker --lib --no-fail-fast
cargo test -p skiff-runtime-linked-type-plan --lib --no-fail-fast
cargo test -p skiff-runtime-eval --lib --no-fail-fast
cargo test -p skiff-runtime-boundary --lib --no-fail-fast
cargo test -p skiff-runtime-service-db --lib --no-fail-fast

# combined
cargo fmt --all -- --check
git diff --check
```

combined acceptance还必须反向搜索确认：

- production中没有 `ConcreteNominal.type_arguments`；
- applied arguments没有 serde default/legacy alias/dual read；
- source/compiler没有以 `resolved_type_arg_texts` 参与语义；
- post-link generic nominal没有退化为 bare `Address`；
- value handoff没有把 identified carrier静默转回裸 `RuntimeValue`。

## 公共语义决策状态

存在一个真实但不阻塞 F286 的公共语义缺口：当前 reference/architecture说明 Package API与Service API应
复用 generic type system，`PackageSchemaCanonicalDescriptor`也已有 `type_params`，但没有定义 applied
`PackageSchema` 的 reference wire、`PackageSchemaTypeId`与instantiation的关系，以及 generic
`PublicTypedError` 如何携带argument identity。

本结果选择任务明确允许的最短安全策略：**本轮 fail closed，内部/package-local fully-instantiated
generic nominal继续实现；任何 generic nominal进入 PackageSchema、ServiceProtocol或public error
envelope都拒绝。** 因而没有必须在 S0/S1 前等待用户回答的 blocker。

若产品要求本轮同时开放 public generic schema，则需要用户先确认该范围；届时必须在实现前另行冻结上述
public wire/identity语义并扩大 version matrix，不能由 consumer自行选择，也不能通过丢 arguments或
InternalError以外的 fallback冒充支持。

## 审计验证

本任务只执行 `rg`、shell文件存在性/内容读取、`git status`/`rev-parse`等只读检查；未运行
build/test，未修改 production、test、reference，未操作 stable、未 push。
