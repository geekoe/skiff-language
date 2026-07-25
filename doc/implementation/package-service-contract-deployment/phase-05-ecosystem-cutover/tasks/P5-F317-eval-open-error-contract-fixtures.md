# P5-F317 Eval open error contract fixture batch

状态：Completed。结果见
`P5-F317-eval-open-error-contract-fixtures-result.md`。

## 直接父节点

- `P5-F314-eval-platform-catch-fixture-closure-result.md`
- open artifact/contract结果：
  `P5-F288-open-error-artifact-contract-consumers-result.md`
- open effect结果：
  `P5-F290-open-error-effect-consumer-result.md`

## Finding wave与范围

同一eval selector在F314直接小修后继续暴露第二个旧`BoundaryErrorContract`构造；已对
`runtime/eval/**`完整反搜。唯一写入范围：

- `runtime/eval/src/assembly_execution/websocket_contract_plan.rs`
- `runtime/eval/src/assembly_execution/ordinary/tests.rs`
- `runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs`

只允许test module/fixture修改；禁止修改production、其它crate、representation consumer或service wire。

## 完成标准

- 删除剩余`BoundaryErrorContract` imports与`BoundaryOperationContract.errors` fields；
- WebSocket/ordinary fixtures其它字段与断言保持；
- source inline effect fixture保留：
  - error Package schema requirement；
  - effect先throw public `Failure`、调用方exact catch、随后response的完整行为；
  - open operation contract不再声明closed typed error；
- 将该fixture/helper中的“typed contract”措辞改为“open service error channel”或只描述typed payload，
  不暗示operation拥有closed error set；
- eval production/test反搜`BoundaryErrorContract|errors: BoundaryErrorContract`为零；
- 不新增replacement field、compat或error set。

## 验证owner

```bash
cargo test -p skiff-runtime-eval --lib -- --list
cargo test -p skiff-runtime-eval --test catch_fixture_closure --no-fail-fast
git diff --check
```

若F315/F316 representation consumer尚未合入而遮挡list，运行最窄fixture compile或记录精确首错；
不得越界。不运行root/request/host/workspace/stable/live。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f317-eval-open-error-fixtures`
- branch：`codex/p5-f317-eval-open-error-fixtures`
- 新的一次性Agent，5分钟内修改，提交并返回完整反搜与验证；
- 不push、不承接其它节点。
