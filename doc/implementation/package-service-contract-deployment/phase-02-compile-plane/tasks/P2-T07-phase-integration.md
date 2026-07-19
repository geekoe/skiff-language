# P2-T07：Phase Integration Gate

## 目标

在既有terminal任务以及T03A–J、T03H1–H2、T04A–D、T05C13、R10H全部合入，且同一候选上的R10I与F09D均PASS、
旧 integration tail 从未进入新 branch ancestry后，运行一次阶段gate、修复纯机械fixture、记录精确证据。
不得新增语义或顺手重构。

## 依赖与 worktree

- 直接在Phase02 integration worktree执行，不另建task worktree。
- 依赖phase-plan列出的全部terminal任务，特别是T03A–J、T03H1–H2、T04A–D、T05C13、R10H，以及同一最终候选上
  PASS的R10I/F09D高风险证据。
- 波次9j已在`2bb5d3e`唯一运行foundation与compiler总gate：foundation 281/0/1 PASS；compiler因canonical `/`
  fixture和T03J source blocker FAIL。fixture修复已提交为`e3cbffd`；T03J不触及foundation范围，因此恢复时不得
  重跑foundation或compiler总gate，只运行受影响的exact compiler repair probe与尚未执行的结构gate。
- 波次9m已在`3b34570`运行exact runtime_slots repair probe与identity self-test/production scan并PASS；boundary
  首次运行因checker仍冻结T04A–D之前shape而9-DENY。T05C13只改boundary checker，因此波次9o不得重跑上述
  PASS证据，只重新执行boundary并继续尚未执行的DAG/public-API/rustfmt/diff。

## 完成态

1. 确认integration候选只包含终态 compile-plane 交付、工作树clean、无未解释冲突。
2. 运行phase-plan指定的foundation/compiler、结构gate和必要workspace check；每个昂贵命令只运行一次。
3. 只允许修复由新canonical wire引起的机械fixture/API拼写；语义失败退回对应 owner。
   旧 service publication harness、空/fake contract 或批量删除 test targets 不属于机械修复。
4. 运行结构反向搜索与checker self-test，证明旧compiler owners和所有compatibility adapter/allowlist归零。
5. 记录`phase-result.md`：commit、命令、owner、结果、覆盖、baseline与证据失效规则。
6. targeted rustfmt覆盖全部phase修改Rust文件；full rustfmt失败必须在main同环境复现才可标baseline。
7. 结构/typed证据证明source contract facts只有一个owner、lowering operation index归零、projection不再blanket
   Local，并且provider/consumer E2E未读取provider/deployment。
8. slash dependency address、all-executable exact source facts和opaque File IR execution representation均有反向
   证据：无dot compatibility、无旧AST owner、无contract ServiceSymbol/display fallback。
9. interface exact facts在source按ContractTypeId完成conformance，interface/executable共同使用opaque
   execution projection；canonical PackageArtifact public-instance path不生成或比较legacy OperationAbiRef/File IR
   semantic signature，compiled/projection-input也不从File IR/TypeResolutionModel重算interface conformance。
10. source-declared、typed package、compiler-known与invalid interface使用单一owner分类，std public-path
    normalization只有一个production owner。
11. expression exact projection直接消费resolved IR与完整PackageTypeRef sidecar；LocalType debug文本不再回流
    source parser，Map keys/for-in派生类型与contract identity均保真。

## Gate

```bash
# 9j/9m已唯一运行，恢复时引用已有证据，不重跑：
# node scripts/verify.mjs --only foundation
# node scripts/verify.mjs --only compiler
# cargo test -p skiff-compiler --test runtime_slots map_keys_and_for_in_lower_to_typed_slots -- --exact --nocapture
# node scripts/check-artifact-identity-single-source.mjs --self-test
# node scripts/check-artifact-identity-single-source.mjs
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
node scripts/check-crate-public-api.mjs --all-configured
git diff --check
```

## 回报

给出最终候选commit、gate表、机械修复、暂时不可用的下游与未运行的live命令及理由。
不得以任务级旧commit证据替代受影响的阶段最终gate。
