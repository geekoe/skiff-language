# P3-T08A：Terminal Runtime Consumer Seams

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§6.2、§9、§10、§12、§14。
- 风险/验收组：中风险 production consumer seam；T09/A01覆盖最终集成。
- 当前成熟度：R03 已验收 runtime-link checkpoint；完成后关闭 Phase 03 downstream compile seam。
- 有效证据状态：本任务 clean commit叠加调度时 exact R03 integration checkpoint。上游 public API、受影响
  consumer call graph、依赖、fixture或测试变化会使证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent接收后合流。

## DAG 与执行约束

- 依赖：R03 PASS，可与 T07、T08B并行。
- 解锁：T09。
- branch：`codex/p3-t08a-runtime-seams`。
- worktree：`/Users/geek/workspace/skiff-p3-t08a-seams`。
- 五分钟内产生真实代码 edit；此前不跑测试、不重做设计；若 consumer需要 Phase 04语义才能运行，只做
  terminal compile seam或显式 fail-closed，不实现临时 adapter。

## 写入范围

- `runtime/{activation,eval,package-test,request,linked-type-plan}/**` 中受 T01–T06 terminal types影响的最小
  compile seam。
- 不修改 deployment/artifact、loader/linked-program/linker/host owner；不修改 router/test-runner/telemetry/live。

## 完成态

1. 受影响 runtime consumer要么消费新 immutable assembly image/ref，要么在 Phase 04入口前结构化 fail closed；
   不从 old DTO/raw JSON/display/source path推导新 target。
2. 删除/断开 production request-time service lazy-load与 service-specific program owner的残余调用边；legacy定义若
   仍为 Phase 05 consumer存在，不得从新 admission call graph可达。
3. 不实例化 ActivationContext，不实现 service dispatcher、boundary materialization、async/stream/callback/cancel
   传播；这些路径保持清晰 compile seam。
4. affected runtime crates至少可编译；已知 Phase 04未接线处由 typed error/feature boundary表达，不恢复旧 API。

## 最早风险探针

- 删除旧 symbol后 production反向搜索归零；仅隔离 legacy模块的残留逐项列出不可达证据。

## 唯一验证 ownership

```bash
cargo check -p skiff-runtime-activation -p skiff-runtime-eval -p skiff-runtime-package-test -p skiff-runtime-request -p skiff-runtime-linked-type-plan
git diff --check
```

只格式化本任务文件；不得运行完整 runtime selector或 Phase 03 gate。

## 回报

提交一个 commit，回报 commit、consumer seam表、残余 legacy定义及不可达证据、命令。
附自验收矩阵：`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`。
