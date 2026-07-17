# P2-T01：Canonical Compile-Contract Checkpoint

## 目标

冻结 Phase 02 所有并行任务共同消费的 typed wire、identity 与无代码 contract producer。不得把旧
`PublicationAbiUnit`、`PackageUnit` 或 `ServiceUnit` 改名成新对象。

## 依赖与 worktree

- 从 Phase 02 文档 checkpoint 创建独立 worktree。
- 建议 branch：`codex/package-service-p2-t01-contract-checkpoint`。
- 高风险 canonical schema/identity；完成并合入 integration 后才启动 T02–T04。

## 完成态

1. artifact-model 按职责模块化定义 `ContractTypeId`、contract type ref、boundary value plan、
   `BoundaryOperationDescriptor`、`BoundaryImplementationRequirements`、
   `BoundaryCallableProjection`、稳定 Unavailable reasons、`PackageRequirement`、`ContractRequirement`、
   `ServiceRequirement`、`ServiceCallRef`、`PackageLocalAbi`、`PackageArtifact`、`ServiceContract`。
2. 所有 wire 使用 tagged union、deny-unknown 和必填语义字段；unsupported/unknown 不能靠缺字段表达。
3. artifact-identity 是 contract type、operation、ServiceProtocol、PackageLocalAbi/Build identity 的唯一
   assign/validate owner；mutation golden覆盖 inclusion/exclusion和map插入顺序。
4. ServiceProtocolIdentity 包含完整 operation descriptor与closed schema；排除provider/build/deployment/
   route、implementation requirements和诊断文本。
5. 新增独立 typed `ServiceContractDefinition -> ServiceContract` leaf pipeline；不读取 provider code、
   PackageArtifact、service config或runtime state。Phase 02 不新增用户可见文件语法。
6. ContractTypeId 不复用 AbiTypeId；package signature中 contract ref显式携带稳定ID，descriptor从contract
   closure解析。
7. PackageArtifact identity projection纳入requirements、implementation links、callable facts、boundary
   projection/provenance/value plan；不把旧 serde aggregate当preimage。
8. 把目前位于 `service_unit.rs` 但属于 package executable leaf 的 target/callable类型移到中性模块；新对象
   不依赖 ServiceUnit module。
9. 新增结构 checker/自测，禁止两个最终对象嵌入 PublicationAbiUnit/ServiceUnit或复制 identity owner。

## 写入范围

- `artifact-model/**`、`artifact-identity/**`。
- 新的 contract definition/compiler leaf crate及其 tests。
- root workspace、Cargo lock、verify subject、crate DAG/public API policy的必要接线。
- 不修改 compiler source/lowering/projection/emission/driver production path。

## 验证

```bash
cargo test -p skiff-artifact-model -p skiff-artifact-identity
cargo test -p <new-contract-leaf-crate>
node scripts/check-artifact-identity-single-source.mjs --self-test
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-compiler-crate-dag.mjs
git diff --check
```

若实际 crate 名不同，回报中给出精确命令。测试必须覆盖严格 wire、closed schema、identity mutation、
provider-independent contract和禁止字段负例。

## 回报

提交 commit、自验收矩阵、公共 API 索引、mutation matrix和仍需 downstream producer填写的字段；不得只
报告 serde round-trip。
