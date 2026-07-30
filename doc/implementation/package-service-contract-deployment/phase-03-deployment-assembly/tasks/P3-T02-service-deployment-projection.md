# P3-T02：Source-free ServiceDeployment Projection

> 已完成的历史任务；2026-07-30后不得重发。当前projection不再消费config profile，也不绑定
> config/SecretRef/state/resource/policy；独立snapshot projection由Phase 05新任务拥有。

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2 列表 1–5、§5、§9、§10、§11、§14。
- 风险/验收组：高风险 deployment projection；R02在同一 checkpoint分别判定 deployment/assembly边界，
  T09覆盖最终集成。
- 当前成熟度：T01 canonical implementation checkpoint；完成后推进 deployment implementation checkpoint。
- 有效证据状态：本任务 clean commit叠加调度时的 exact T01 integration checkpoint。T01 public surface/
  identity、projection代码、依赖、fixture或测试变化会使相关证据失效。
- integration边界：只提交 task branch，不 merge integration/main、不 push；主 Agent接收后合流。

## DAG 与执行约束

- 依赖：R01 PASS；与 T03/T04/T05之间无 API依赖，按可用 worker调度。
- 解锁：R02 deployment verdict；为 T09提供真实 deployment producer。
- branch：`codex/p3-t02-deployment-projection`。
- worktree：`/Users/geek/workspace/skiff-p3-t02-deployment`。
- 接受任务后五分钟内产生真实代码 edit；此前不跑测试、不重做设计；若 T01 缺字段，立即回报 checkpoint
  amendment，不在本任务扩 schema。

## 写入范围

独占 `deployment/src/projection/**` 及相邻 module export/tests。不得修改 `artifact-model/**`、
`artifact-identity/**`、compiler、assembly resolver 或 runtime。

## 输入与完成态

实现唯一 pipeline：

```text
ServiceDeploymentInput + exact ServiceContract + implementation PackageArtifact closure
  -> validate/project -> ServiceDeployment
```

必须满足：

1. 人类 public path 只在输入 trust boundary 解析成 `PackageCallableId`；artifact 不保留 display/public path。
2. contract 每个 operation 恰好映射一次；missing/duplicate/extra、unknown operation/callable 全失败。
3. selected callable 必须 `BoundaryCallableProjection::Available`，其 operation contract、ContractTypeId/value
   plan、effect guarantee与 ServiceContract descriptor 精确相符。
4. implementation runtime callable/capability requirements唯一闭合；业务配置由独立snapshot projection验证，
   不进入本输入或输出。
5. implementation package及所有 package direct requirements 用 exact version/local ABI/build ref闭合；binding key
   是 `(callerBuildId, alias)`，同 key 不得产生两个 target。
6. service dependency只保存 contract selector与 caller-relative slot，不选 deployment revision/provider executable。
7. projection只读取 typed artifacts；production依赖图和反向搜索中不得出现 compiler AST/source/lowering/File IR
   signature inference、legacy runtime DTO、adapter或fallback。
8. 输出 canonical normalize 后 assign并立即 validate DeploymentArtifactIdentity。

## 最早风险探针

- 同一 callable 可显式映射两个 contract operations；同名函数不自动绑定。
- structurally equal package-local nominal type不能冒充 ContractTypeId。
- missing required capability、Unavailable callable、protocol mismatch都给稳定结构化错误。
- 改 public path拼写但解析到同一 callable不改变 artifact identity；改 exact target或 binding会改变 identity。

## 唯一验证 ownership

```bash
cargo test -p skiff-deployment projection
rg -n 'compiler|source|lowering|PackageUnit|ServiceUnit|serviceAssembly' deployment/src/projection
git diff --check
```

只格式化本任务 Rust 文件；不得运行完整 deployment/phase gate。

## 回报

提交一个 commit，回报 commit、projection API、正负例矩阵、source-free 反向搜索和精确命令。
附自验收矩阵：`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`。
