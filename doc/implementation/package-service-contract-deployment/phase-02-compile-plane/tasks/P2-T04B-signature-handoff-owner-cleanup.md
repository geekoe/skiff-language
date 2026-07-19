# P2-T04B：Signature Handoff Owner Cleanup

## 目标

在T03F改变source signature owner后刷新T04A证据，并清理独立验收发现的compiled/projection重复owner与
panic式handoff。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”“Compiler 与 Projection
流水线”章节。

## 依赖与写域

- 依赖T03F、T04A。
- 独占`compiler/compiled` exact signature mapping、`projection-input` key/normalization leaf、
  `compiler/projection/src/package_artifact`窄consumer与直接测试。
- 不修改source、lowering或integration fixtures。

## 完成态

1. 从source executable/public view到projection executable key的mapping拆出职责明确的小模块；不继续膨胀
   `compiled/src/projection_input.rs`。
2. public path scope/normalization只有一个canonical owner，compiled与projection不再各自复制
   `scoped_public_path`规则。
3. missing binding/module/executable/signature返回结构化compile/projection error，不使用`panic!`。
4. exact Local/Contract/container/nullable、public-instance receiver trimming、missing/duplicate/extra/target mismatch
   行为保持；projection仍不读File IR signature。
5. 刷新T04A全部聚焦证据，证明T03F source owner变化没有产生第二条handoff或seed fallback。

## 聚焦验收

- compiled/projection-input/projection direct tests和相关crate check、反向搜索、`git diff --check`。
- 不运行Phase gate。

## 执行合同

- DAG：波次9b、与T03G并行；完成后解除R10I。风险：中高；并入typed-contract production批次验收。
- worktree：`/Users/geek/workspace/skiff-p2-t04b-signature-owner-cleanup`；分支：
  `codex/p2-t04b-signature-owner-cleanup`；从含T03F的integration HEAD创建。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；source/public signature facts、projection key、
  compiled/projection-input handoff或PackageArtifact projection变化即失效。

