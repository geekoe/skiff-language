# P3-T01：Canonical Deployment / Assembly Contract Checkpoint

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§5、§10、§11、§12、§14。
- 风险/验收组：高风险 canonical schema/identity；T01完成后另设一次只读边界验收，T09覆盖最终集成。
- 当前成熟度：planning document checkpoint；完成后只推进为 canonical implementation checkpoint，不是稳定候选。
- 有效证据状态：本任务返回的单一 clean commit及其精确 dependency/fixture状态。之后任何 artifact/deployment
  public surface、identity projection、workspace dependency、checker subject或本任务测试变化都会使证据失效。
- integration边界：开发 Agent只提交 task branch，不 merge integration/main、不 push；主 Agent接收后合流。

## DAG 与执行约束

- 依赖：P3-D01 文档评审 PASS。
- 解锁：R01；R01 PASS 后 T02、T03、T04、T05均只依赖本 checkpoint。
- branch：`codex/p3-t01-canonical-contract`。
- worktree：`/Users/geek/workspace/skiff-p3-t01-contract`。
- 接受任务后五分钟内必须产生第一个真实代码 edit；此前不跑测试、不重做设计；若 shared surface 无法按本文冻结，回报
  `TASK_NOT_EXECUTABLE`，不得自行发明兼容层。

## 目标

冻结 Phase 03 唯一 typed wire、semantic refs、canonical identity 与基础 validator，并建立
`skiff-deployment` crate shell。它是后续并行 consumer 的 shared checkpoint，不实现 projection、resolution、
runtime loading 或 execution。

## 写入范围

- `artifact-model/**`：deployment/assembly DTO 与 leaf key/ref/template/link-plan 类型。
- `artifact-identity/**`：DeploymentArtifactIdentity、AssemblyIdentity 的 assign/validate、canonical projection、
  mutation matrix。
- 新 `deployment/Cargo.toml`、`deployment/src/lib.rs`、公共 error/validation模块及测试 fixture builder；预建并
  导出互不引用的空 `projection` / `assembly` module shell，使 T02/T03 不争抢 crate root。
- root `Cargo.toml`、`Cargo.lock`、必要 verify subject registry、identity checker及其 self-test。

不得修改 compiler、`runtime/**`、T02 projection 或 T03 resolver。既有 legacy DTO 可暂留给 Phase 05，但新对象
不得嵌入或依赖 `PublicationAbiUnit`、`PackageUnit`、`ServiceUnit`、raw `serviceAssembly`。

## 冻结 surface

至少定义并 strict-wire（camelCase、deny unknown、必填语义字段）：

- `PackageArtifactRef(packageId, packageVersion, packageBuildId, packageLocalAbiIdentity)`；
- `ServiceContractRef(serviceId, contractVersion, serviceProtocolIdentity)`；
- `ServiceDeploymentRef(serviceId, contractVersion, deploymentRevision, deploymentArtifactIdentity)`；
- `ServiceDeploymentInput`、`ServiceDeployment`、operation/package/service/ingress/config/secret/state/resource/
  capability/policy binding；
- `(callerPackageBuildId, packageRequirementAlias)` 与
  `(callerPackageBuildId, serviceRequirementSlot)` 的 typed key；
- `RuntimeAssembly`、resolved deployment/contract/package refs、canonical package link plan、activation-relative
  service/config/state/resource templates、global ingress table；
- schema version constants、opaque `DeploymentRevision`、distinct deployment/assembly identity newtypes。

`ServiceContract` 仍独占 descriptor/schema；deployment/assembly 只保存 exact refs/operation IDs。
`ServiceDeployment` 不保存 provider revision 于 consumer service selector；semantic refs 不保存 filesystem path。
empty assembly 是合法 canonical value。

## 完成态与最早探针

1. validator 拒绝 duplicate key、dangling ref、identity/coordinate mismatch、裸 slot 冲突、global ingress
   collision、unknown field和 declared identity tamper。
2. canonical normalization 与 insertion order 无关；display/diagnostic/path、resolved secret bytes、replica state
   不进入 identity。
3. implementation build、operation/dependency/ingress/config literal/secret ref/state/resource/policy、resolved graph、
   link plan或template变化会改变对应 identity。
4. canonical empty assembly assign/validate/serde round-trip稳定。
5. checker能识别新对象嵌入 legacy aggregate、第二 identity owner、字段改名/移动/重复 owner；self-test通过。

## 唯一验证 ownership

```bash
cargo test -p skiff-artifact-model -p skiff-artifact-identity -p skiff-deployment
node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-crate-public-api.mjs --crate skiff-deployment
git diff --check
```

只格式化本任务 Rust 文件。不得运行 Phase 03 完整 gate。

## 回报

提交一个 commit，回报 commit、public API 索引、identity inclusion/exclusion matrix、empty assembly证据、已运行
命令和 downstream 必须使用的 fixture/API。不得只报告 serde round-trip。

回报附自验收矩阵：`设计/任务条款 | 代码证据 | 反向搜索证据 | 测试`。
