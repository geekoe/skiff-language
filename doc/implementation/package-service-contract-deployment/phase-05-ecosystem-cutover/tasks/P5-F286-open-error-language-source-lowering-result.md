# P5-F286 Open error language/source/lowering result

状态：`PARTIAL_CHECKPOINT`；non-generic language consumer通过，fully-instantiated generic nominal
等待 F294 shared DTO 后续接。

## Exact candidate

- implementation commit：
  `a63cb4233a410bc729147348ac64fffcc928b2cb`
- 直接父任务：
  `P5-F286-open-error-language-source-lowering.md`
- generic gap与owner：
  `P5-F292-generic-nominal-type-ref-gap-result.md`
  →
  `P5-F293-generic-nominal-type-ref-owner-audit-result.md`

## 已完成

- source建立唯一 `CatchLeaves` owner，statement/expression/test-effect throw、catch与rethrow共用；
- primitive、literal、anonymous record/container、interface、unknown、function、nullable、
  unconstrained type parameter及mixed union在source phase失败关闭；
- nominal record、representation、named-union branch、transparent alias与合法anonymous nominal union
  按权威语义检查；
- rethrow要求 `Exception<E>`并复用原envelope；
- source不再读取 callable/operation closed error set，test-effect throw不读取declared set；
- 五种 declaration和named-union concrete/synthetic/literal branch已lower；
- source-authored throw/call写入真实site，generated fixture使用有限synthetic reason，
  catch type为required；
- type-closure、external/publication ref与rewrite traversal迁移新 declaration/branch/site shape；
- F285 owner-aware dependency signature rehydration与F290 effect语义保留。

本任务production owner反向搜索中，closed error-set、旧union `variants`、optional catch和无site
call/throw constructor均已清零。

## 验证

```text
skiff-compiler-core --lib       41/41
skiff-compiler-source --lib    306/306
skiff-compiler-lowering --lib   46/46
git diff --check                 PASS
```

`package_imports`在枚举前被当时范围外的旧consumer遮挡：

- `runtime/loader/src/runtime_assembly.rs`仍引用closed error contract；
- compiler projection的 api exports、callable normalization、schema、visible types仍读取旧union
  `variants`。

这些是后续 combined consumer/probe owner；不能把未执行的package-imports记为通过。

## 未完成与续接边界

source可以识别 fully-instantiated generic nominal，但 A1 `TypeRefIr`无法保存普通nominal arguments；
lowered construct/throw/catch仍会丢失或拒绝这些参数。不得从source text恢复或把
`Box<string>`降为bare declaration address。

F294将先冻结 `AppliedNominal` strict DTO与identity generation；随后新的language continuation只适配
本结果的 compiler owner并补 generic正负例。该续接完成、combined probe和独立A2验收通过前，本结果不是
完整language PASS，也不解除runtime实现。
