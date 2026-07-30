# P5-F315 Representation wrap compiler producer

状态：Completed。结果见
`P5-F315-representation-wrap-compiler-producer-result.md`。

## 直接父节点

- shared model acceptance：
  `P5-F308-representation-wrap-model-acceptance-result.md`
- handoff audit：
  `P5-F306-representation-constructor-carrier-audit-result.md`
- compiler type producer：
  `P5-F296-applied-nominal-compiler-consumer-result.md`

## DAG位置与范围

- 节点：representation carrier S1；与S2 linked consumer并行。
- 完成后与S3 eval consumer共同进入combined probe。

唯一production范围：

- `compiler/lowering/src/function_lowering.rs`
- `compiler/lowering/src/external_refs.rs`
- `compiler/lowering/src/publication_local_refs.rs`
- `compiler/lowering/src/file_ir/identity.rs`
- 仅必要generation/assertion适配：
  `compiler/lowering/src/source_file_lowering.rs`

允许co-located lowering tests。禁止修改source/core、artifact-model/identity、compiled/projection、
runtime/std或权威文档。

## 完成标准

- `lower_representation_constructor_call`不再丢弃validated target；
- 每个显式source representation constructor产生：

```text
ExprIr::RepresentationWrap {
  value: lowered payload ref,
  type_ref: validation.target.ir
}
```

- plain/generic target保留exact owner与ordered arguments；
- nested explicit constructors产生nested wraps，不做隐式wrap/coercion；
- payload expression只求值一次，expression keys/side effects顺序不变；
- direct `throw R("x")`的value指向wrap，`payload_type`与required throw site保持exact；
- external-ref collection与publication-local rewrite递归处理wrap target与child；
- 旧File IR v7/v5 compiler goldens只刷新到F308冻结的v8/v6，不修改其它generation；
- 不新增display/static throw fallback、record fields、compat或named-union promotion。

## 验证owner

至少覆盖plain/generic/nested representation constructor、direct throw、payload side-effect once、
external package owner/rewrite与wrong target既有source rejection。

```bash
cargo test -p skiff-compiler-lowering --lib -- --list
cargo test -p skiff-compiler-lowering --lib --no-fail-fast
git diff --check
```

selector非零。不运行compiler integration（由combined owner）、runtime/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f315-representation-compiler`
- branch：`codex/p5-f315-representation-compiler`
- 风险：中高；一次性Agent，5分钟内修改；
- 提交并返回producer/side-effect/site/generation矩阵与验证；
- 不push、不承接linked/eval。
