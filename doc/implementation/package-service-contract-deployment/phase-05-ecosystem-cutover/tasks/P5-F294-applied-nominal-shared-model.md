# P5-F294 Applied nominal shared model and identity

状态：Ready for contract review。

## 直接父节点与权威链

- owner 审计结果：
  `P5-F293-generic-nominal-type-ref-owner-audit-result.md`

父结果继续引用 F292/F291、A1 shared model与唯一权威语言/架构。启动时只读本任务；需要依据时沿父链向上读取。

## DAG 位置、基线与并行边界

- 节点：F293最短DAG的 `S0 shared DTO + artifact identity generation`。
- 当前 integration 已包含 F288 identity generation；本任务从该新代际继续做一次严格 bump。
- F286 正在修改 `compiler/core`、`compiler/source`、`compiler/lowering`；本任务禁止修改任何
  `compiler/**`，因此可以并行。
- 本任务只冻结 applied nominal 的 canonical wire、strict model、artifact traversal与identity generation；
  不实现 source producer、linked type或runtime value carrier。
- 完成后解除：
  - F286 fully-instantiated generic nominal continuation；
  - package/public generic fail-closed consumer；
  - linked/runtime identity consumer。
- 当前是实现检查点，不是稳定候选。

## 唯一 canonical shape

在 `artifact-model` 新增并唯一导出：

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

TypeRefIr::AppliedNominal {
    base: NominalTypeRefBaseIr,
    arguments: Vec<TypeRefIr>,
}
```

固定规则：

- `arguments`在 wire 上 required、non-null、non-empty，按 declaration `type_params`顺序；
- `arguments`不得使用 serde default、alias或`skip_serializing_if`；empty不能被省略成另一种wire；
- 零参数 nominal只能使用既有 plain variant，同一实例不存在两种合法表示；
- base只能是上述 closed enum，不能是 builtin/container/structural/interface/alias/actor/DB object、
  `TypeParam`或另一个 applied wrapper；
- source/link层之后按 exact owner验证 declaration kind与arity；shared DTO层不得按display/shape猜；
- `NamedUnionBranchIr::ConcreteNominal`只保留`nominal_type: TypeRefIr`，删除独立
  `type_arguments` map；
- generic branch参数进入 `nominal_type` 的 ordered arguments；enclosing generic union参数属于
  applied union owner，不重复存储。

不增加 serde default、alias、legacy field、dual read/write或旧 artifact compatibility。

## 唯一 production 写入范围

`artifact-model`：

- `src/types.rs`
- `src/lib.rs`
- `src/schema.rs`
- `src/cross_package_identity.rs`
- `src/actor_declaration.rs`
- `src/file_ir.rs`
- `src/file_ir/service_calls.rs`

仅在存在真实 exhaustive traversal/admission需要时允许：

- `src/executable.rs`
- `src/package_artifact.rs`
- `src/package_unit.rs`
- `src/publication_abi.rs`
- `src/recoverable.rs`
- `src/service_unit.rs`
- `src/contract_types.rs`

这些“仅允许”文件不得改变公共 shape；只可接入 nested applied-ref traversal/validation。

`artifact-identity`：

- `src/constants.rs`
- `src/file_ir.rs`
- `src/semantic.rs`
- `src/package_artifact.rs`
- `src/package_artifact/projection.rs`
- `src/package_artifact/validation.rs`
- `src/package/projection/implementation_links.rs`
- `src/lib.rs`
- `src/ecosystem_paths.rs`

允许上述 owner 的 co-located tests/fixtures/goldens。禁止修改 compiler、runtime、deployment、
test-runner、router、std、生态仓库或权威文档。

## 完成标准

### 1. Strict DTO 与 traversal

- missing、null、empty `arguments`、unknown base/field、plain nominal附加arguments、旧 branch
  `typeArguments`全部 strict reject；
- Rust内构造空 arguments也必须被唯一 semantic/admission validator拒绝，不能只依赖deserialize；
- 所有 canonical type-ref walk/rebind/hash/actor-ref rejection递归进入 ordered arguments；
- base owner rebinding保持 exact package/dependency/ABI expectation；nested arguments分别递归；
- branch不再有第二份 map，序列化中只能出现 `nominalType`。

### 2. Contextual shared admission

- local/plain nominal的零参数与 applied/nonzero参数形状可由 File IR declaration context验证时，精确检查：
  - declaration存在；
  - kind是 record、representation或named union，不是alias/interface；
  - plain/applied与`type_params` arity一致；
  - arguments中的`TypeParam`只允许引用当前合法scope；
- 外部 owner在 shared层无法取得descriptor时保留exact locator，交由 dependency/link consumer验证；
  不得当作零参数或按路径后缀放行；
- applied `PackageSchema` DTO可以被结构化表达；本任务只在授权的 artifact-model /
  artifact-identity admission直接遇到它时fail closed。compiler projection中的PackageSchema closure、
  public export与`PublicTypedError` fail-close明确延后给F293 `S2 package/public consumer`，本任务不得
  修改`artifact-identity/src/contract.rs`或compiler owner。

### 3. Strict generation 与 identity

从 F288 当前代际一次切换：

| Domain | 当前 | 本任务 |
| --- | --- | --- |
| File IR schema | v6 | v7 |
| File IR format | v4 | v5 |
| File IR identity prefix | v6 | v7 |
| PackageArtifact schema | v4 | v5 |
| Local ABI marker / prefix | v2 / v4 | v3 / v5 |
| Build marker / prefix | v3 / v5 | v4 / v6 |

保持：

- opcode table；
- legacy Package Unit build/local ABI；
- PackageSchema Type/Index identity；
- ServiceContract/Definition；
- ServiceProtocol v4；
- ContractOperation、Operation ABI、Publication ABI；
- package/service human version label继续是非identity，任何label-only变化不得改变上述identity。

必须以mutation test证明：

- 同base的`Box<string>`与`Box<number>`改变File IR、Local ABI、Build identity；
- nested argument变化与argument reorder改变identity；
- tamper base owner或argument后旧identity验证失败；
- 旧schema/prefix/marker拒绝；
- non-generic artifact也只由新writer产生新generation；
- 保持项的算法/常量未被顺手bump；
- package/service human version label-only mutation不改变identity。

### 4. 非目标

- 不实现 compiler source/lowering producer。
- 不实现 linked `AppliedNominal`或runtime catch identity/carrier。
- 不开放 generic PackageSchema、ServiceContract或`PublicTypedError`。
- 不改变用户语法、named union语义、ErrorPayload/InternalError或开放错误envelope。
- 不刷新范围外跨crate fixture；下游断编译是预期consumer handoff。

## 最小验证 owner

先以`--list`确认非零：

```bash
cargo test -p skiff-artifact-model --lib -- --list
cargo test -p skiff-artifact-model --lib --no-fail-fast
cargo test -p skiff-artifact-identity --lib -- --list
cargo test -p skiff-artifact-identity --lib --no-fail-fast
cargo fmt --all -- --check
git diff --check
```

至少有独立test覆盖：

- structural round-trip与所有 strict负例；
- local declaration arity/kind/scope正负例；
- named-union branch map删除与nested applied ref；
- cross-package exact owner递归rebind；
- actor/非法base fail closed；
- identity/version mutation matrix。

不得运行 workspace、compiler/runtime、生态、stable、live或chat smoke。

## 风险、worktree 与交付

- 风险：高；验收组：`A1b-applied-nominal-model`，实现后安排独立只读验收。
- worktree：`/Users/geek/workspace/skiff-p5-f294-applied-nominal-model`
- branch：`codex/p5-f294-applied-nominal-model`
- 不push、不操作stable。
- 启动到第一次production修改不超过5分钟；不可执行时返回
  `TASK_NOT_EXECUTABLE`、精确缺口与最小前置。
- 完成后提交并返回commit、wire/identity矩阵、反向搜索、自验收和所有下游遮挡；
  不自行承接compiler/runtime。
