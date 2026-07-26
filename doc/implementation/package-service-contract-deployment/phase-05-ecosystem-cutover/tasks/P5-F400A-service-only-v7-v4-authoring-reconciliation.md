# P5-F400A Service-only v7/v4 authoring reconciliation

状态：Superseded；不得执行。

本任务被`P5-F402-service-calls-manifest-selection-design-result.md`取代。其service-only source owner、
markerless service自动选择全部public callable、retired `serviceCall` marker等前提均不再成立。
后续实现必须从F402重新审计并拆分；不得复用本文production write set或fresh acceptance。

## 直接父节点

- `P5-F400-integrated-service-only-source-gate-result.md`

父节点已完成production lineage、真实失败、写入owner与验收矩阵审计，并继续引用F395的挂起语义
实现DAG。引用链可追溯到Phase 05唯一权威设计。实现时默认只读本任务和直接父节点；只有需要核对
依据时才沿链向上读取。

## DAG位置与目的

- DAG节点：F395 `G0`之前唯一的service-only source reconciliation。
- 前置：F400判定当前Phase 05候选不能读取未改动的current Relay service-only root。
- 完成后解除：重新执行F400 gate；通过后才允许启动挂起语义N0。
- 当前成熟度：实现检查点；不是稳定候选。
- 风险：高；canonical source owner、PackageArtifact/ServiceContract authoring与identity输入。

实现目标是把current service-only source语义移植进现有v7/v4 pipeline。不得merge/cherry-pick旧
实现，不恢复retired terminal pipeline、`package.yml`或`serviceCall`，不修改v7/v4 wire schema
和identity owner。

## 精确production范围

仅允许修改父结果已确认的owner：

- `compiler/input/src/service_config.rs`
- `compiler/input/src/package_config/mod.rs`
- `compiler/input/src/package_config/manifest_validation.rs`
- `compiler/driver/authoring.rs`
- `compiler/driver/pipeline/mod.rs`
- `compiler/projection/src/package_artifact/model.rs`
- `compiler/projection/src/package_artifact/projection.rs`
- `scripts/skiff.mjs`

职责要求：

1. `service.yml + api.yml`形成唯一typed `ServiceSourceRoot`。id/version/packages只来自
   `service.yml`；current access、route-list HTTP与timeout必须验证，不能静默丢弃。
2. package/service两种source owner复用同一publication id/version/dependency validator，输出
   同一种经过alias、exact-version、reserved-name与dependency-access校验的compile input；不得
   建第二份校验表。
3. 在读取manifest和创建artifact store之前按唯一root kind分派：
   package-only是ordinary package，service-only走同一
   `compile_service_package -> publish_package_artifact_records -> write_service_contract`
   v7/v4链；双manifest是ambiguous。
4. `package build`对service-only root只写package+contract records，并在deployment/profile
   lookup前成功返回。若`package publish`尚不能完整表达current deployment input，必须在任何
   record/pointer写入前fail closed，不能留下partial publication。
5. pipeline使用typed service role，而不是
   `validated_service_root: bool`。service role选择全部public callable roots；ordinary package
   不生成service roots。retired marker只能fail closed，不能参与选择。
6. projection使用明确的service-root policy，从canonical public function/public-instance ABI生成
   `PackageServiceCallRoot`。未改动Relay的markerless public instance必须精确生成两个method
   roots；PackageArtifact v7 wire和identity owner保持不变。
7. Node与Rust root classification一致：
   package-only=`package`、service-only=`service`、双manifest=`ambiguous`；`test/check/dev`
   和authoring不得产生相反判断。

开始修改前必须反查上述文件的当前实际符号；若父结果给出的owner已经移动，仅允许机械跟随同一职责。
若需要新增其它production owner、改变公共schema/identity、恢复旧pipeline或决定deployment语义，
立即返回`TASK_SCOPE_EXPANDED`。

## 明确禁止

不得修改：

```text
artifact-model/** production
artifact-identity/** production
compiler/contract/** production
deployment/**
runtime/**
router/**
internals/**
```

`compiler/contract/src/projection.rs`只允许新增测试，不得改production。不得加compatibility、
dual-read、validator waiver、synthetic `package.yml`、synthetic `serviceCall`或修改Relay source。
不得操作stable/live/Mongo共享实例、不得push、不得派子Agent。

## 测试范围与快速验证

允许修改：

- `compiler/input/src/service_config.rs`内联测试
- `compiler/input/src/package_config/tests.rs`
- `compiler/driver/authoring/tests.rs`
- `compiler/projection/src/package_artifact/tests/projection.rs`
- `compiler/contract/src/projection.rs`的test-only区域
- `scripts/tests/skiff-test-cli.test.mjs`
- `scripts/tests/package-service-authoring.test.mjs`

必须删除或反转“service-only必须拒绝”的过时断言，不能留下两套root契约。

由本节点唯一执行：

```bash
cargo test --manifest-path compiler/input/Cargo.toml service_config
cargo test --manifest-path compiler/projection/Cargo.toml package_artifact
cargo test --manifest-path compiler/contract/Cargo.toml projection
cargo test --manifest-path compiler/Cargo.toml authoring
node --test scripts/tests/skiff-test-cli.test.mjs
node --test scripts/tests/package-service-authoring.test.mjs
cargo fmt --all -- --check
git diff --check
```

迭代时先跑最小selector；实现冻结后各完整命令只跑一次。不得运行整个workspace gate。

## Fresh acceptance

在单一fresh临时root中：

1. 用`skiff-package-service-smoke-fixture --bootstrap-only` bootstrap canonical dependencies；
2. 发布未改动的Internals `packages/llm-api`；
3. 发布未改动的Internals `packages/llm-providers`；
4. 对未改动的`internals/codex-relay/service`执行`package build`。

命令与参数使用父结果第7.4节的精确矩阵。最后一步必须同时证明：

- Relay输入仍没有`package.yml`，`api.yml`仍没有`serviceCall`；
- stdout同时有PackageArtifact v7与ServiceContract v4 receipt；
- id/version/two exact dependencies来自`service.yml`；
- operation keys精确为
  `relayProxy.responsesCompleted`、`relayProxy.responsesCompletedResult`；
- 不生成deployment receipt、pointer或assembly；
- 所有records只在fresh artifact root；
- missing `service.yml`、双manifest、retired marker、缺任一dependency pointer均在store write前
  fail closed。

fresh probe是最早风险探针和完成证据，成功或首个失败后不得反复重跑完整链。失败时先用只读证据归类；
若属于本合同owner可用最小探针修复，若暴露新owner则停止。

## 交付

写`P5-F400A-service-only-v7-v4-authoring-reconciliation-result.md`，记录：

- exact start/end commit与tree；
- production/test变更与旧路径反向搜索；
-每条验证命令的真实计数；
- fresh Relay receipts、两operation、负例和临时状态清理；
- 自验收矩阵。

提交所有改动并保持worktree clean；不merge/rebase/push。启动后5分钟内进行第一次实际代码修改；
若仍不能形成任务规定的单一路径，返回`TASK_NOT_EXECUTABLE`。
