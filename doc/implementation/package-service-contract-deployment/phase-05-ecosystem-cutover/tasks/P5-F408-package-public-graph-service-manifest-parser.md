# P5-F408 Package public graph and service manifest parser

状态：Ready。

## 直接父节点

- `P5-F407-service-calls-shared-schema-model-checkpoint-result.md`

F407已删除PackageArtifact selection、发布v8/v9模型并增加
`ServiceManifestAuthoring.service_calls`。本节点只迁移compiler的Package public graph producer和
service manifest shape validation；不实现typed contract selection。

## DAG位置与候选

- DAG节点：F407后的compiler producer/parser consumer。
- start commit：`288a105fc87399c5e93228ee9f2ba2e58c4cd2b6`。
- 完成后解除：F409 typed selection/contract/driver。
- 与F410、F411并行，写入不得重叠。
- 风险：高；public API source grammar与PackageArtifact producer。

## 独占写入范围

```text
compiler/core/**
compiler/input/**
compiler/input-model/**
compiler/source/**
compiler/compiled/**
compiler/projection-input/**
compiler/projection/**
compiler/emission/**
上述crate拥有的聚焦tests
本任务result
```

禁止修改`compiler/contract/**`、`compiler/driver/**`、deployment、runtime、router、test-runner、
artifact-model/identity、ecosystem source和权威设计。

## 必须实现

1. 删除`PublicationApiEntry`、public instance、resolved/compiled/projection DTO上的
   `service_call`字段与builder/copy。
2. `api.yml` function只接受scalar source selector；删除`source + serviceCall:true` object leaf。
   public instance object只允许`const/interfaces`。旧marker不兼容，必须fail closed。
3. Package public graph继续完整包含functions、constants、types、public instances及其全部listed
   interface methods；不得因selection移除boundary projection、links或Local ABI facts。
4. 删除`PackageExports.service_call_functions`、instance selection bool、
   `project_service_call_roots`及PackageArtifact field assignment。
5. 删除ordinary/service role在Package projection阶段的marker gate。Package producer完全不读取
   `service.yml.serviceCalls`。
6. `compiler/input::service_config`验证`service_calls`：
   - missing/empty合法；
   - 每项是非空canonical dotted public path；
   - duplicate在sort/dedup前报错；
   - 只验证字符串shape，unknown/non-callable/boundary availability留给F409 typed owner；
   - service root仍必须是`package.yml + api.yml + service.yml`，不得恢复service-only input。
7. 保留`service_call_refs`、FileIR call-site `ServiceCall`与dependency requirement语义。

## 测试与风险探针

必须新增/迁移：

- scalar function与`const/interfaces` public instance正例；
- `serviceCall`/`source` object旧写法负例；
- missing、empty、dotted、duplicate、wrong-shape `serviceCalls`；
- v8 PackageArtifact无selection但完整function/public-instance/method surface；
- ordinary Package与Service生成相同PackageArtifact/Local ABI（service manifest selection不参与）。

先用`-- --list`记录实际选择，再运行至少：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-input api_yml
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-input service_config
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-source api
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-projection-input
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-projection package_artifact
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-emission package_artifact
cargo fmt --all -- --check
git diff --check
```

可按真实crate/test名调整等价selector，但不得用零测试。下游contract/driver尚未迁移导致的workspace
compile failure不属于本checkpoint；不得越界修。不得运行完整workspace/isolated/stable/live，不得派
子Agent。

## 交付

写`P5-F408-package-public-graph-service-manifest-parser-result.md`，记录exact commit/tree、producer数据流、
parser正负例、Package identity不变量、反向搜索与测试计数。提交并保持clean，不merge/rebase/push。
