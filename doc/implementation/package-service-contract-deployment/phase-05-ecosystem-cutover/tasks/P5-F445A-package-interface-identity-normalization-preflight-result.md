# P5-F445A Package interface identity normalization preflight result

结论：`PREFLIGHT_COMPLETE / TASK_EXECUTABLE`

预检已经形成单一实现路径。F444C 的四个 `any interface` 参数错误不是 Agine
call spelling、source cast、interface 复制或 `packages/agent/**` 发布错误，也不是
artifact projection、dependency rehydration、linker 或 runtime 的错误。唯一生产 owner 是：

```text
compiler/source/src/type_resolution_model.rs
  TypeResolutionModel::canonicalize_type_ref
  TypeRefIr::AnyInterface arm
```

该 arm 递归规范化 `canonical_type_args`，却把序列化在
`InterfaceInstantiationRef.interface_abi_id` 中的 `TypeRefIr::PackageSymbol` 当成不透明
字符串。因此，同一个 package id、symbol path 和 exact ABI expectation，在 consumer
上下文中分别以 `PackageRefIr::Dependency { dependency_ref }` 与
`PackageRefIr::PackageId { package_id }` 表示时，最终被 exact structural comparison
误判为不同。修复必须在 source compiler semantic comparison 前规范化这段嵌入 identity，
不能把相同 ABI hash 无条件视为相等。

## 输入与边界

| 输入 | 实际读取状态 | 结论 |
| --- | --- | --- |
| Skiff integration | `/Users/geek/workspace/skiff-phase-05-integration`，`e31ad3e4b2bb`，clean | 任务声明基线为 `c81266f3`；`c81266f3..e31ad3e4` 只新增 F445A/F445B task 文档，无 production/test 差异，因此对本预检 production-equivalent |
| 当前 task worktree | `/Users/geek/workspace/skiff-p5-f445a-interface-identity-preflight`，初始 `e31ad3e4b2bb`，clean | 只新增本 result |
| Internals integration | `/Users/geek/workspace/internals-phase-05-integration`，`19d41001f048`，clean | 只读 `packages/agent/**`、`packages/llm-api/**` 与直接消费关系 |
| F444C draft | stash commit `91f3cc32e9d6ce0b14b4145d3d94815ab1a52420` | 只用 `git show` / `git grep` 读取；未 apply/pop/drop |
| skiff-packages integration | `/Users/geek/workspace/skiff-packages-phase-05-integration`，`19cfab5dfc82`，clean | 无写入、无 owner |

未运行 Internals canonical graph、stable/live/network，也未启动本地 service/runtime。

## F444C 症状

F444C canonical type-check 在 `runtimeBindingsWithSubagent` 的四个参数处停止：

| 参数 | 声明 package | artifact ABI |
| --- | --- | --- |
| `LlmClient` | `agine.ai/llm-api` | `65de703a…` |
| `AgentEventReceiver` | `agine.ai/agent` | `02c07451…` |
| `ToolProvider` | `agine.ai/agent` | `02c07451…` |
| `SubagentDelegate` | `agine.ai/agent` | `02c07451…` |

每一处的 package、symbol 和 ABI expectation 相同，显示诊断甚至是相同的
`any LlmClient`；内部差别仅是：

```text
expected: PackageRefIr::Dependency { dependency_ref: ... }
found:    PackageRefIr::PackageId { package_id: ... }
```

Internals source 也支持这个判断：

- `packages/agent/tools.skiff` 的 `AgentRuntimeBindings` 与
  `runtimeBindingsWithSubagent` 正常使用 canonical interface 声明；
- `packages/agent/api.yml` 正常公开这些 interface、record 与 callable；
- `packages/agent/package.yml` 以 `llmApi` 依赖 `agine.ai/llm-api`；
- Agine service 同时直接依赖 `agine.ai/agent` 和 `agine.ai/llm-api`，所以 consumer
  有精确 dependency binding；
- agent package 中公开 callable 直接暴露 `any interface` 的入口只有
  `runtimeBindings` 与 `runtimeBindingsWithSubagent`；`LlmClient` 是通过 provider
  public signature 暴露的 dependency-owned interface，正好覆盖 transitive 风险。

所以修改 `packages/agent/**`、给参数加 cast、改变 call spelling 或复制 interface 都会掩盖
compiler identity bug，禁止作为实现。

## 唯一 owner 证据链

### 1. Source 声明产生 canonical package identity

`compiler/source/src/type_resolution_model.rs::resolve_package_interface` 为 source 中导入的
package interface 生成：

```text
PackageSymbolRef {
  package: PackageRefIr::PackageId { package_id },
  symbol_path: <public canonical path>,
  abi_expectation: <selected artifact exact ABI>
}
```

现有测试
`contract_type_resolution::tests::interface_signatures::package_interface_conformance_stays_owned_by_canonical_package_facts`
验证了这条规则。本预检聚焦运行结果：`1 passed`。

### 2. Provider PackageArtifact 保留精确 public callable signature

链路为：

```text
compiler/source/src/contract_type_resolution/types.rs
  ContractAwareTypeResolver::resolve_expanded_ir
    decode AnyInterface.interface_abi_id into exact PackageTypeRef
      ↓
compiler/compiled/src/package_callable_signatures.rs
      ↓
compiler/projection/src/package_artifact/callables/normalization.rs
  normalize_public_signature
      ↓
PackageArtifact.package_local_abi.public_symbols
```

projection 会递归规范化 public signature，但不会把 package owner identity 擦除。这里保留
provider/canonical `PackageId` 是正确行为；没有证据要求 republish agent 或 llm-api。

### 3. Consumer rehydration 有意产生 dependency-local identity

`compiler/driver/source_compile/canonical_dependencies.rs::package_callable_analysis_from_symbols`
读取已选定的 exact `PackageArtifact` 并校验 artifact identity。
`compiler/source/src/expression_type_model.rs` 在 package callable typing 中调用
`TypeResolutionModel::rehydrate_package_signature_type_for_dependency`，随后通过
`bind_package_type_refs_to_dependency` 生成 consumer-local expected type。

`compiler/source/src/type_resolution_model/shape_assignability.rs::
rehydrate_package_signature_local_type` 对 builtin、applied nominal、record、union、nullable、
`AnyInterface` 与 function 都递归处理，将 provider-owned：

```text
PackageId(package_id, symbol_path, exact ABI)
```

重绑定成：

```text
Dependency(dependency_ref, symbol_path, exact ABI)
```

这是 caller-local link form，不是错误。现有测试
`type_resolution_model::tests::package_signature_exact_symbols_rehydrate_and_ownerless_slots_fail_closed`
明确断言了 nested `AnyInterface`、array、nullable 和 record 的 dependency-local 结果；本预检
聚焦运行结果：`1 passed`。该行为来自既有修复
`46371e57 fix(compiler): rehydrate dependency callable local types`，不得删除或逆转。

### 4. Semantic comparison 漏掉嵌入 identity

package call 由
`compiler/source/src/expression_type_model/contract_call_typing/type_projection.rs` 的
`contract_source_assignability` /
`contract_source_assignability_with_projections` 进入
`TypeResolutionModel::assignable`。后者先分别调用
`canonicalize_type_ref`，再做 exact structural `type_assignable`。

`canonicalize_type_ref` 已正确处理普通 `TypeRefIr::PackageSymbol`：

- 由 `package_dependencies` 将 `Dependency` 解析成 canonical package id；
- 由 `package_artifact_identities` 补齐 exact ABI expectation；
- 保留并规范化 public symbol path；
- 因而不同 package、symbol 或 ABI 仍不相等。

但它的 `TypeRefIr::AnyInterface` arm 当前只做：

```text
interface_abi_id: interface.interface_abi_id.clone()
canonical_type_args: recursively canonicalized
```

`interface_abi_id` 实际是 canonical JSON 编码的 `TypeRefIr`。其中的
`PackageRefIr::Dependency` 没有经过同一 canonicalization，最终与 source argument 中的
`PackageRefIr::PackageId` 做字符串/结构精确比较并失败。这是 F444C 四个错误共同且唯一的
production owner。

### 5. Lowering、linker 与 runtime 不是 owner

下游链路保留 exact identity：

```text
compiler/lowering/src/executable_type_projection.rs
  AnyInterface exact identity
      ↓
runtime/linker/src/linker/file_conversion.rs
  LinkedInterfaceInstantiationRef
      ↓
runtime/linker/src/assembly_execution/address_resolver.rs
  resolve_package_ref
      ↓
runtime/linked-program/src/assembly_execution.rs
  indexes by package id and dependency ref
      ↓
runtime/linked-type-plan/src/type_plan.rs
  both forms resolve to the same exact type slot
```

`address_resolver` 分别支持 `Dependency`（按 caller requirement 精确绑定）和 `PackageId`
（按已加载 package id 唯一解析），并在 type address 处继续检查 ABI expectation。F444C
在 source type-check 已失败，根本没有到达这条链路；runtime/linker/linked-type-plan 不进入
写集。

`artifact-model/src/cross_package_identity.rs` 虽有跨 package alias canonicalizer，但没有
production call site，而且同样未规范化嵌入的 interface id。把修复放进 artifact-model 会
扩大 publication/link 范围，当前没有证据支持。

## 独立 RED fixture

实现任务应新增与 Agine 无关的 package-project integration test。最小 direct fixture：

```yaml
# provider/package.yml
id: example.com/interface-provider
version: 1.0.0
```

```yaml
# provider/api.yml
Handler: api.Handler
accept: api.accept
echo: api.echo
```

```skiff
// provider/api.skiff
interface Handler {
  function handle(self: Self) -> string
}

function accept(handler: any Handler) -> string {
  return "ok"
}

function echo(handler: any Handler) -> any Handler {
  return handler
}
```

```yaml
# consumer/package.yml
id: example.com/interface-consumer
version: 1.0.0
packages:
  - id: example.com/interface-provider
    version: 1.0.0
    alias: provider
```

```skiff
// consumer/main.skiff
import provider

function forward(handler: any provider.Handler) -> string {
  return provider/accept(handler)
}

function roundTrip(handler: any provider.Handler) -> any provider.Handler {
  return provider/echo(handler)
}
```

RED 必须断言内部 identity，而不只匹配易混淆的 display text：

```text
same package id
same public symbol path
same exact abi_expectation
expected owner = Dependency("provider")
actual owner   = PackageId("example.com/interface-provider")
compile rejects before fix
```

为覆盖 F444C 的 `LlmClient` 形态，同一个 test module 再加三 package fixture：

```text
example.com/interfaces
  declares Handler
       ↓ dependency alias iface
example.com/provider
  public callable accepts/returns any iface.Handler
       ↓ consumer depends on both exact packages
example.com/consumer
  passes any interfaces.Handler through provider callable
```

这能证明 dependency-through-signature，而不引用 Agine 源码或 artifacts。

## 正确 GREEN 与实现方式

只在 `TypeResolutionModel::canonicalize_type_ref` 的 `AnyInterface` arm：

1. 将 `interface.interface_abi_id` 反序列化为 `TypeRefIr`；
2. 对该 identity 递归调用现有 `canonicalize_type_ref`；
3. 使用已导入的
   `skiff_artifact_identity::interface_instantiation_ref(canonical_identity, canonical_args)`
   重建 canonical JSON；
4. 继续递归规范化 `canonical_type_args`；
5. 解析失败不得退化成 “ABI 相同即相等”、去掉 owner、猜 package 或忽略版本；无效 artifact
   必须沿既有 validation fail closed。

该做法只把两种合法引用形式在当前 consumer 的 exact selected dependency 下转换到同一
canonical package identity。它仍保留 package id、public symbol path、exact ABI expectation
和 generic args，所以不会放宽不同 package/version/build。

`canonicalize_type_ref_for_module` 与 `transparent_alias_ir` 也存在不透明复制 interface id
的代码形态，但本 RED/F444C 链路不经过它们。实现任务不得无证据顺手扩写；若新增独立 RED
证明另一 comparison path 同样失败，再单独上报 scope。

## 实现 DAG 与精确写集

```text
F445A implementation
├── RED: independent direct + transitive package fixtures
├── production: canonicalize embedded AnyInterface identity
├── GREEN: positive/negative identity matrix
├── artifact/receipt invariance proof
└── integration merge
    ├── unblocks F444C identity diagnostics
    └── does not unblock F444C timeout syntax/runtime blocker (F445B owns it)
```

允许的最小实现写集：

| 文件 | 修改 |
| --- | --- |
| `compiler/source/src/type_resolution_model.rs` | 修改 `canonicalize_type_ref` 的 `AnyInterface` arm；如需 helper，只在同文件增加一个 private helper |
| `compiler/tests/package_interface_identity.rs` | 新增独立 direct/transitive fixtures 与正负矩阵 |
| `compiler/Cargo.toml` | 注册新的 focused integration test target |

明确排除：

- `artifact-model/**`
- `compiler/projection/**`
- `compiler/driver/**` 的 dependency rehydration
- `compiler/lowering/**`
- `runtime/linker/**`
- `runtime/linked-program/**`
- `runtime/linked-type-plan/**`
- Internals `packages/agent/**`、Agine service source
- artifact schema、receipt schema、版本或 build selector

若 implementation RED 不能由上述三文件写集转绿，必须停止并以新证据上报 scope expansion，
不得靠 source cast 或降低 exact identity 检查通过。

## 验收矩阵

| 场景 | 预期 |
| --- | --- |
| provider 自有 interface，public callable direct 参数；dependency ref 对 package id | GREEN |
| provider 自有 interface，public callable 返回值再赋给/传给 consumer 的同 interface | GREEN |
| `any Interface?` | GREEN，owner 与 exact ABI 保留 |
| `Array<any Interface>` | GREEN，递归元素 identity 被规范化 |
| record field 中 direct / nullable / array `any Interface` | GREEN |
| provider public signature 暴露 dependency-owned interface，consumer 直接依赖同一 exact package | GREEN；覆盖 `LlmClient` 形态 |
| alias 与 package id 不同表示，但 package id + symbol + ABI + type args 全相同 | GREEN |
| package id 不同，即使 symbol text 或 ABI 字符串被构造成相同 | RED |
| package id/symbol 相同但 ABI expectation 不同 | RED |
| generic interface 的 canonical type args 不同 | RED |
| dependency alias 未绑定、绑定歧义或 exact artifact selection 不成立 | RED / fail closed |
| malformed `interface_abi_id` artifact | artifact validation RED；不得在 comparison 中降级接受 |
| exact version/build 不满足 dependency requirement | 既有 artifact selection/identity validation RED |
| 已有 nested rehydration baseline | 继续 GREEN |
| direct fixture pre-fix | RED，显示 Dependency 与 PackageId 的唯一内部差异 |
| direct/transitive fixture post-fix | GREEN，且不依赖 cast、call spelling 或 duplicate interface |

implementation 至少运行：

```bash
cargo test -p skiff-compiler-source \
  package_interface_conformance_stays_owned_by_canonical_package_facts -- --nocapture

cargo test -p skiff-compiler-source \
  package_signature_exact_symbols_rehydrate_and_ownerless_slots_fail_closed -- --nocapture

cargo test -p skiff-compiler \
  --test package_imports \
  dependency_callable_local_parameter_preserves_schema_result_field_types \
  -- --exact --nocapture

cargo test -p skiff-compiler \
  --test package_interface_identity -- --nocapture
```

前三个命令已在本预检运行，分别为 `1 passed`、`1 passed`、`1 passed`。第三个也证明现有
package-project harness 能覆盖 nested owner-local `any` 类型。新 test 必须先在未修 production
代码时证实 RED，再在修复后 GREEN。

## Artifact identity / receipt 影响

预期是 compiler semantic-only 修复：

- provider `PackageArtifact` public signature bytes 不变；
- `agine.ai/agent` 与 `agine.ai/llm-api` 的 package build id、package local ABI 和 receipt
  identity 不变；
- artifact schema / receipt schema 无版本变化；
- lowering、linked type plan、assembly identity 无变化；
- 原先被错误拒绝的 consumer 会首次成功产生其正常 artifact；这是 acceptance 变化，不是
  provider identity 变化。

implementation 必须对独立 provider fixture（以及集成时的 agent/llm-api receipts）做
before/after identity 对比。如果既有可接受 fixture 的 provider artifact、local ABI 或 receipt
发生变化，说明修复泄漏到 projection/publication，必须停止并上报 `TASK_SCOPE_EXPANDED`。

## F444C 恢复条件

1. F445A implementation 在 Skiff integration 合入上述 RED/GREEN 与负向矩阵。
2. 证明 `agine.ai/agent`、`agine.ai/llm-api` 的 exact artifact/receipt identity 未因本修复改变。
3. 独立 owner F445B 完成 timeout syntax/runtime blocker；F445A 不吸收该范围。
4. 之后才可在 F444C worktree 恢复其 stash，并重跑 canonical Agine service type-check。
5. 四个 interface identity diagnostics 必须全部消失；不得出现 cast/call-spelling workaround。
6. F444C 再继续其 Node receipt/architecture tests、`.test.skiff` matrix、reverse-search 与 final
   candidate 验收。

在 F445A 与 F445B 都满足前，F444C 仍不可宣告 executable-complete。
