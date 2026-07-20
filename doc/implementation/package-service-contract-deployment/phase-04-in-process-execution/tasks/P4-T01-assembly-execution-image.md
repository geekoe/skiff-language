# P4-T01：Canonical Assembly Execution Image

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2.5、§6.1、§6.2、§9、§10、§12、§14。
- 风险/验收组：高风险code/call target边界；与T02/T03合流后由R01验收。
- 当前成熟度：planning document checkpoint；完成后只是immutable execution-image implementation checkpoint。
- 有效证据：本任务clean commit及调度时exact Phase 04 doc checkpoint。linked-program/linker/type-plan public surface、
  File IR call variants、Cargo edge或本任务测试变化会使证据失效。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：P4-D01 PASS；可与T02并行。
- 解锁：T03；T01/T02均合流后才可开始。
- branch：`codex/p4-t01-assembly-execution-image`。
- worktree：`/Users/geek/workspace/skiff-p4-t01-execution-image`。
- 启动后五分钟内产生第一个真实代码edit；此前不跑测试、不重做设计。无法在不使用legacy aggregate的情况下
  形成execution image时，回报`TASK_NOT_EXECUTABLE`与精确缺口。

## 目标与写入范围

独占`runtime/linked-program/**`、`runtime/linker/**`、`runtime/linked-type-plan/**`中的assembly execution
projection与直接测试；必要时修改对应Cargo manifest。不得修改activation/boundary/model/eval/request/host/router。

## 完成态

1. 从`SharedPackageLinkedImage`形成assembly-wide immutable executable/type view；每个`PackageBuildId`只有一个
   code/file/type owner，不构造service-specific program。
2. canonical `PackageCallable`按caller build + requirement alias + exact local ABI解析到package direct target，
   保留独立call kind；canonical `ServiceCall`只成为`ActivationRelativeServiceCall`，不选择provider executable。
3. local type/executable地址在shared image内确定性解析；tampered file ref/index/callable/protocol/requirement fail closed。
4. public handoff不含`ServiceUnit`、`PackageUnit`、publication ABI、provider deployment或route，不把canonical call
   转成legacy symbol/display target。
5.为T03提供稳定、只读的entry executable lookup、package direct lookup、service-call instruction和type-plan view。

## 最早探针与唯一验证 ownership

- 两activation共享同一execution image/code Arc；不同caller的slot 0 service instruction仍不绑定provider。
- package diamond只产生一个dependency code owner；package direct与service call的linked kind可区分。

```bash
cargo test -p skiff-runtime-linked-program -p skiff-runtime-linker -p skiff-runtime-linked-type-plan assembly_execution
node scripts/check-runtime-crate-dag.mjs
git diff --check
```

只格式化本任务Rust文件；不得运行完整runtime gate。

## 回报

提交一个commit，回报public API索引、canonical call映射表、legacy反向搜索、命令与自验收矩阵：
`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`。
