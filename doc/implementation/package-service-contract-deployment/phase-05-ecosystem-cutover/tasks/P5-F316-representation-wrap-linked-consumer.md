# P5-F316 Representation wrap linked consumer

状态：Completed。结果见
`P5-F316-representation-wrap-linked-consumer-result.md`。

## 直接父节点

- shared model acceptance：
  `P5-F308-representation-wrap-model-acceptance-result.md`
- handoff audit：
  `P5-F306-representation-constructor-carrier-audit-result.md`
- linked applied nominal checkpoint：
  `P5-F297-applied-nominal-linked-consumer-result.md`

## DAG位置与范围

- 节点：representation carrier S2；与S1 compiler producer并行。
- 完成后解除S3 eval consumer。

允许production：

- `runtime/linked-program/src/linked.rs`
- `runtime/linker/src/linker/file_conversion.rs`
- `runtime/linker/src/assembly_execution/code_linker.rs`
- `runtime/linked-type-plan/**`仅target-kind proof或必要type-plan tests

允许上述owner co-located tests/fixtures。禁止修改artifact/compiler/eval/model/boundary/capability/native/
request/host/std。

## 完成标准

- 新增唯一：

```text
LinkedExprIr::RepresentationWrap {
  value: ExprRefIr,
  type_ref: LinkedTypeRef
}
```

- file conversion逐字段保留child与完整plain/applied target；
- code linker递归link base与arguments，完成后不退化为bare Address；
- assembly admission验证linked target exact declaration kind为Representation，wrong record/union/
  alias/interface、wrong owner/arity、残留TypeParam全部fail closed；
- generic `R<string>`与`R<number>`、nested owner及external package owner保持不同；
- linked type plan证明target可产生Representation plan与exact identity输入，不实现eval carrier；
- 不借用record Construct、不推断display/shape/static throw type、不新增compat path。

## 验证owner

```bash
cargo test -p skiff-runtime-linked-program --lib -- --list
cargo test -p skiff-runtime-linked-program --lib --no-fail-fast
cargo test -p skiff-runtime-linker --lib -- --list
cargo test -p skiff-runtime-linker --lib --no-fail-fast
cargo test -p skiff-runtime-linked-type-plan --lib -- --list
cargo test -p skiff-runtime-linked-type-plan --lib --no-fail-fast
git diff --check
```

至少覆盖strict linked wire、plain/generic/nested/external target、target kind负例与F297 arguments保持。
selector非零，不运行eval/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f316-representation-linked`
- branch：`codex/p5-f316-representation-linked`
- 风险：高；一次性Agent，5分钟内修改；
- 提交并返回linked/kind/arguments矩阵与验证；
- 不push、不承接eval。
