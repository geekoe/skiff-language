# P3-T05：Shared PackageArtifact Linked Image

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§3、§6、§9、§10、§12、§14。
- 风险/验收组：高风险 shared-code/activation-owner boundary；与 T04、T06合成一次 runtime-link batch只读验收。
- 当前成熟度：R01已验收 canonical implementation checkpoint；完成后推进 shared linked-image checkpoint。
- 有效证据状态：本任务 clean commit叠加调度时 exact R01 integration checkpoint。link-plan/model surface、
  linked-program production API、依赖、fixture或测试变化会使证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent接收后合流。

## DAG 与执行约束

- 依赖：R01 PASS；以 canonical link-plan fixtures开发。
- 解锁：T06。
- branch：`codex/p3-t05-shared-image`。
- worktree：`/Users/geek/workspace/skiff-p3-t05-image`。
- 五分钟内产生真实代码 edit；此前不跑测试、不重做设计；若 T01 link plan不足，回报 amendment，不自行扩
  artifact schema。

## 写入范围

独占 `runtime/linked-program/**` 及其 `Cargo.toml`。不得修改 loader、linker、host、deployment/artifact或其它
runtime crate。

## 完成态

1. 定义 assembly-level hydrated/shared program image：按 PackageBuildId/code slot持有只读 package code、File IR、
   static resource view、callable link lookup与 local ABI facts。
2. 同一 build在 replica内只有一个 code owner；image不拥有 activation config/state/service binding/callback table、
   runtime replica identity或其它 mutable activation state。
3. package direct-call解析链严格为 caller build + requirement alias → exact dependency ref/expected local ABI →
   dependency code slot → PackageCallableId → OperationTargetRef；缺失/错 ABI/错 callable fail closed。
4. service call在 linked instruction中保留 `(callerPackageBuildId, serviceRequirementSlot, operationId,
   expectedProtocolIdentity)`，不得 patch为 provider executable或复用 package direct-call target。
5. 新 image是内存 hydration类型，不作为 serde artifact wire；canonical identity来自 RuntimeAssembly link plan，
   不是 `Arc`/地址/host handle。
6. empty link plan构造空 image成功；所有 lookup fail closed。
7. production新 API不依赖 legacy `ServiceUnit`/`PackageUnit` aggregate。旧 program类型若暂留，必须隔离且不能成为
   T06新 linker返回值。

## 最早风险探针

- 两个 activation共享 package image但不能观察共享 mutable owner。
- 两个 caller package都用 alias/slot 0时不冲突。
- same build不同 content、missing callable、wrong expected local ABI和 service-call provider patch均失败。

## 唯一验证 ownership

```bash
cargo test -p skiff-runtime-linked-program
rg -n 'ServiceUnit|PackageUnit|serviceAssembly' runtime/linked-program/src
git diff --check
```

只格式化本任务 Rust 文件；不得运行完整 runtime/phase gate。

## 回报

提交一个 commit，回报 commit、new image/API索引、package/service call解析证据、shared-vs-activation owner表和命令。
附自验收矩阵：`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`。
