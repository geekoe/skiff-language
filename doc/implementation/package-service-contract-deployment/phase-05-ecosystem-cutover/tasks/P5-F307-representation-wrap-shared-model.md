# P5-F307 Representation wrap shared File IR model

状态：Ready。

## 直接父节点与权威链

- handoff审计：
  `P5-F306-representation-constructor-carrier-audit-result.md`
- strict applied nominal shared model：
  `P5-F295-applied-nominal-model-acceptance-result.md`
- runtime carrier结果：
  `P5-F299-runtime-local-exception-carrier-implementation-result.md`

父链继续引用唯一权威type/error设计。

## DAG位置与边界

- 节点：representation carrier DAG的S0 shared DTO/generation checkpoint。
- 与platform catch R1/R2并行，文件不重叠。
- 完成并独立验收后解除compiler producer与linked/runtime consumers。
- 当前是高风险实现检查点，不是稳定候选。

## 唯一production范围

- `artifact-model/src/executable.rs`
- `artifact-model/src/file_ir.rs`
- `artifact-model/src/schema.rs`
- `artifact-model/src/lib.rs`仅必要re-export
- `artifact-identity/src/constants.rs`
- `artifact-identity/src/file_ir.rs`

允许上述owner的co-located tests。禁止修改compiler/runtime、PackageArtifact/contract DTO、std、生态或
权威文档。

## 完成标准

### 1. 唯一strict表达式

新增：

```rust
ExprIr::RepresentationWrap {
    value: ExprRefIr,
    type_ref: TypeRefIr,
}
```

- serde wire精确为required `kind: representationWrap`、`value`、`typeRef`；
- 无default、optional、alias、legacy shape、display、field map或instruction site；
- expression/type visitors递归访问child ref与完整`type_ref` arguments；
- File IR semantic admission验证child ref存在，target是plain/applied nominal且exact declaration kind为
  `Representation`；wrong arity、alias/interface/record/union/primitive、unresolved base与残留
  `TypeParam`全部fail closed；
- applied `PackageSchema`继续按F295拒绝。

### 2. Generation与identity

- schema：`skiff-file-ir-v8`
- format：`skiff-file-ir-format-v6`
- identity prefix：`skiff-file-ir-v8:sha256`
- opcode table保持v1；
- representation target owner、argument及child变化进入File IR identity；
- 旧schema/format/prefix严格拒绝；
- PackageArtifact schema、Local ABI/Build marker与ServiceProtocol等其它generation保持。

### 3. 非目标

- 不生产wrap，不新增linked/eval variant；
- 不用record `Construct`或reserved field模拟；
- 不实现named-union promotion；
- 不新增compat reader、dual writer或fallback。

## 验证owner

```bash
cargo test -p skiff-artifact-model --lib -- --list
cargo test -p skiff-artifact-model --lib --no-fail-fast
cargo test -p skiff-artifact-identity --lib -- --list
cargo test -p skiff-artifact-identity --lib --no-fail-fast
git diff --check
```

至少覆盖strict wire正负、plain/generic representation、nested arguments、所有非法target、visitor与identity
mutation。selector必须非零，不运行compiler/runtime/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f307-representation-wrap-model`
- branch：`codex/p5-f307-representation-wrap-model`
- 风险：高；后续必须由独立F308验收；
- 新的一次性开发Agent，5分钟内开始production修改；
- 提交并返回commit、generation matrix、strict admission/identity证据；不push、不承接consumer。

