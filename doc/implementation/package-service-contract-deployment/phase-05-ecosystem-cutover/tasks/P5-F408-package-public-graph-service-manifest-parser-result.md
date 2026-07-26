# P5-F408 Package public graph and service manifest parser result

状态：Complete（F408 scope）。

## 1. 提交锚点与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| 任务规定 start | `288a105fc87399c5e93228ee9f2ba2e58c4cd2b6` | `4688200acf69afe8778b06189c545e06d49d7212` |
| task definition checkpoint | `97f3f831b02507a8caa1f831c590ea044655f895` | `9a208b775f009a9795aaaf7714dae16e1f2d3d25` |
| implementation end | `70f1c407174d8ca7d4cfb45a3c4812fee73a2abc` | `158ae28852aaa4e544897ddee3dedf67ca317a12` |

实现提交：

```text
70f1c407174d8ca7d4cfb45a3c4812fee73a2abc
feat(compiler): detach package service call selection
```

production/test改动只落在任务授权的：

```text
compiler/core/**
compiler/input/**
compiler/source/**
compiler/compiled/**
compiler/projection-input/**
compiler/projection/**
compiler/emission/**
```

本文是唯一额外result文件。没有修改`compiler/contract/**`、`compiler/driver/**`、
deployment、runtime、router、test-runner、artifact-model/identity、ecosystem source或权威设计；
没有承接F409 typed selection或其它DAG节点。

## 2. Producer数据流

Package public graph现在只有一条不带selection bit的数据流：

```text
api.yml scalar/public-instance shape
  -> PublicationApiEntry / PublicationApiPublicInstanceEntry
  -> PublicCallable / PublicInstance
  -> ExportCallableBinding / ExportPublicInstanceBinding
  -> compiled projection input
  -> ExportCallableProjection / ExportPublicInstanceProjection
  -> PackageExports
  -> PackageLocalAbi + implementation/callable links + boundary projections
  -> PackageArtifact v8
```

具体收敛：

- 从`PublicationApiEntry`和`PublicationApiPublicInstanceEntry`删除`service_call`字段及两个
  `with_service_call` builder；
- 从source resolved public callable/public instance、compile-model binding、compiled handoff以及
  projection-input public/export DTO删除`service_call`字段和全部copy；
- 从`PackageExports`删除`service_call_functions`，从`PackageExportPublicInstance`删除selection
  bool；
- 删除`project_service_call_roots`及`PackageArtifact.service_call_roots` assignment；
- Package producer的输入没有service manifest或source role，因此ordinary Package与service root
  使用同一个projection路径；
- producer不读取`ServiceManifestAuthoring.service_calls`。该字段只由compiler input parser校验并
  保留给F409 typed owner消费。

`service_call_refs`、File IR `CallTargetIr::ServiceCall`和service dependency requirement链保持原样。

## 3. Parser正负例

### 3.1 `api.yml`

canonical function/public symbol leaf只接受scalar source selector：

```yaml
echo: api.echo
```

public instance object只接受`const`和`interfaces`：

```yaml
managedLlm:
  const: root.llm.managedLlm
  interfaces:
    - root.llm.ManagedLlm
```

以下旧形态全部fail closed：

- `source` object；
- `source + serviceCall: true/false` object；
- marker-only object；
- public instance额外`serviceCall`字段，包括重复marker。

parser不把`source` object悄悄解释为嵌套`<path>.source` export，而是直接报告function必须使用scalar
string selector。

### 3.2 `service.yml serviceCalls`

compiler input在共享authoring DTO之外增加了两层边界：

1. 先检查原始YAML：`serviceCalls`必须是sequence，每项必须真实为YAML string。该步骤阻止
   `serde_yaml`把number/bool scalar转换为`String`后伪装成路径。
2. 再检查每个字符串：非空、无trim漂移、由canonical identifier segment组成，segment之间以`.`连接。

验证顺序和结果：

- missing与`[]`均合法并得到空selection；
- `send`和`worker.run`均合法，成功结果canonical sort为`send, worker.run`；
- duplicate在任何sort/dedup之前报告错误，不静默合并；
- scalar container、number/bool item、空字符串、前导空白、空segment和非identifier segment均拒绝；
- unknown path、non-callable path和boundary availability没有在本层解析，明确留给F409；
- 带非空selection与missing selection读出的Package manifest事实相同；
- service root仍必须同时有`package.yml + api.yml + service.yml`。

## 4. PackageArtifact与identity不变量

projection正例固定断言：

| surface | 结果 |
| --- | --- |
| PackageArtifact wire | `skiff-package-artifact-v8` |
| Package build prefix | `skiff-package-build-v9:sha256` |
| Package build preimage marker | `skiff-package-artifact-build-identity-v7` |
| PackageLocalAbi prefix | `skiff-package-local-abi-v6:sha256`，不变 |
| PackageLocalAbi preimage marker | `skiff-package-artifact-local-abi-identity-v4`，不变 |
| legacy selection wire | 不含`serviceCallRoots` |

同一v8 artifact fixture显式保留完整public surface：

- type `Worker`；
- constant `VERSION`；
- functions `mutate`、`run`；
- public instance `worker`；
- listed interface method `worker.handle`。

探针同时确认：

- Local ABI包含type、constant、两个function、public instance和method；
- implementation links包含type、constant、functions以及public-instance receiver；
- callable links、semantic facts和boundary projections均完整包含三个callable；
- public instance的method map与`worker.handle` callable link保持；
- package/contract/service requirements仍在；
- `service_call_refs`仍有精确slot/operation/protocol事实；
- ordinary与service角色两次调用相同Package producer，完整PackageArtifact相等且
  `local_abi_identity`相等；
- stale v7 artifact schema、stale v8 build prefix和stale Local ABI prefix均fail closed。

因此service manifest selection不会改变Package build或Local ABI；F409只应在typed
contract/deployment selection阶段消费它。

## 5. Test discovery与执行

所有可执行selector都先运行`-- --list`，再运行同一selector。

| selector | `-- --list`实际选择 | 执行 |
| --- | ---: | --- |
| `skiff-compiler-input api_yml` | 12 | 12 passed |
| `skiff-compiler-input service_config` | 14 | 14 passed |
| `skiff-compiler-source api` | 18 | 18 passed |
| `skiff-compiler-projection-input` | 9 | 9 passed |
| `skiff-compiler-projection package_artifact` | 62 | 62 passed |
| `skiff-compiler-emission package_artifact` | 10 | 10 passed |

有效Rust测试合计`125 passed / 0 failed`。projection-input的doc-test target为0，没有计入通过数。

执行命令：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-input api_yml -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-input service_config -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-source api -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-projection-input -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-projection package_artifact -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test --locked -p skiff-compiler-emission package_artifact -- --list

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
```

当前F408 worktree没有合入并行F410 deployment consumer。`skiff-compiler-input`的一个范围外
store-backed test声明`skiff-deployment` dev-dependency，因此最终树上两个input `-- --list`会在进入
selector前被以下F410-owned compile break阻断：

```text
deployment/src/fixtures.rs: ServiceDeploymentOperationInput.package_public_path
deployment/src/projection/operations.rs: binding.package_public_path
```

为实际执行F408 input单测，验证时短暂把该dev-dependency改为未启用的optional dependency，并只
cfg-gate唯一使用它的范围外store-backed test；两个临时变化在测试后完整恢复，`git diff`确认
`compiler/input/Cargo.toml`和`compiler/input/src/contract_dependencies/tests.rs`均为零差异。
这没有更改F408 production或上述26个被选测试。最终树上的阻断证据保留，未越界修改deployment。

额外聚焦编译：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo check --locked \
  -p skiff-compiler-input \
  -p skiff-compiler-source \
  -p skiff-compiler-compiled \
  -p skiff-compiler-projection-input \
  -p skiff-compiler-projection \
  -p skiff-compiler-emission \
  --lib
```

六个F408 crate lib均通过；source显示既有unused/dead-code warnings，没有新增selection warning。
emission聚焦fixture补上当前`CallIr`要求的canonical synthetic source site后，10个selector测试全部
可执行。

最终质量检查：

```bash
cargo fmt --all -- --check
git diff --check
```

均PASS。没有运行workspace/full isolated/stable/live、instance或生态publish。

## 6. 反向搜索

授权compiler owners：

```text
rg '\bservice_call\b|\bservice_call_functions\b|\bservice_call_roots\b|\
\bproject_service_call_roots\b|\bPackageServiceCallRoot\b|with_service_call' <F408 owners>
=> 0 files

rg 'serviceCallRoots' <F408 owners>
=> 1 file
```

唯一`serviceCallRoots`字符串是v8 wire absence断言，不是reader、writer、field或compat adapter。

```text
rg '\bservice_calls\b' <F408 owners>
=> 1 file: compiler/input/src/service_config.rs

rg '\bservice_call_refs\b|serviceCallRefs|CallTargetIr::ServiceCall|\
validate_file_ir_service_calls' <F408 owners>
=> 9 files
```

这证明service manifest selection只停留在parser owner，而call-site/service requirement事实没有被
selection删除波及。

全compiler范围仍有6个明确的F409/driver-owned旧selection文件：

```text
compiler/Cargo.toml
compiler/contract/src/projection.rs
compiler/driver/pipeline/mod.rs
compiler/driver/pipeline/tests.rs
compiler/driver/source_compile/canonical_dependencies.rs
compiler/tests/service_call_roots.rs
```

它们仍引用`PackageServiceCallRoot`、`service_call_roots`、`.service_call`或旧builder，是任务明确
禁止修改并留给F409的consumer break。input自己的`serviceCall`字符串只存在于strict negative
tests/error text。lowering中的`service_call` helper和`lower_with_service_calls`属于必须保留的File IR
call-site lowering，不是Package selection marker。

## 7. 自验收矩阵

| 任务条款 | 代码/测试证据 | 结论 |
| --- | --- | --- |
| 删除entry/resolved/compiled/projection selection DTO | 授权owners exact identifier反搜为0 | PASS |
| function只接受scalar selector | scalar正例；`source/serviceCall` object负例 | PASS |
| public instance只接受`const/interfaces` | root selector正例；marker/extra字段拒绝 | PASS |
| v8 Package public graph完整 | type/constant/functions/instance/method + Local ABI/links/boundary断言 | PASS |
| 删除Package roots producer | `PackageExports`无selection；无root projector/assignment；wire absence | PASS |
| Package producer不读manifest selection | `service_calls`仅input owner；ordinary/service artifact全相等 | PASS |
| `serviceCalls` shape与duplicate | raw YAML type gate、canonical path、pre-sort duplicate、canonical sort tests | PASS |
| unknown/callable/boundary不越权 | parser只检查字符串shape | PASS |
| service root仍有三个control files | missing package/api/service矩阵 | PASS |
| 保留service call-site语义 | 9-file反搜；artifact ref slot断言；core ServiceCall target fixture | PASS |
| 不迁移F409/F410范围 | forbidden paths零diff；下游break如实保留 | PASS |

结论：F408 compiler producer/parser checkpoint完成；解除F409的typed selection/contract/driver前置，
但不宣称未集成F409/F410的独立worktree可以完成workspace编译。
