# P5-F400 Integrated service-only source gate result

状态：Superseded by `P5-F402-service-calls-manifest-selection-design-result.md`。

> 本文保留当时代码与probe事实，但其把current service-only Relay source当成canonical输入、要求compiler
> 恢复service-only owner，并让service role自动选择全部public callable的后续结论违反现行权威设计。
> F400A不得执行。service source继续是Package root加`service.yml`；service-call roots由
> `service.yml.serviceCalls`显式选择。

## 1. 判定

当前候选不满足 G0：

```text
TASK_SCOPE_EXPANDED
唯一 reconciliation：
P5-F400A-service-only-v7-v4-authoring-reconciliation
```

候选虽然精确包含 phase-05 的 PackageArtifact v7 / ServiceContract v4 链，但不能读取 current
Internals Relay 的 service-only source。它在创建 artifact store 前直接要求 retired
`package.yml`，因此没有 current Relay 的 v7/v4 positive receipt。不得返回 `G0_PASS`。

本节点没有修改 Skiff、Internals production，没有 merge/cherry-pick，没有访问 stable/live/外部服务，
也没有恢复 `package.yml` 或 `serviceCall`。

## 2. 输入锚点

| 输入 | commit | tree / subtree |
| --- | --- | --- |
| F400 candidate | `bb0f29cfbf9e1e97a767e009a3d6dc7e975e2008` | `8b725336012d6ae349f83b2700cf7d5eb7faaaf3` |
| phase-05 integration | `bb0f29cfbf9e1e97a767e009a3d6dc7e975e2008` | `8b725336012d6ae349f83b2700cf7d5eb7faaaf3` |
| Skiff main | `305882351b1e3ea644f1aef3bbc5a1477ab15858` | `7122a1c89a21de83e3b861a3148fcee6ff8317bc` |
| Internals main | `5861c13f3a92b7fb56a5cfa689e46f5d0462a02d` | `867c99c155386299e7dbb8b4fed95cee2427ba84` |
| Internals `codex-relay` subtree | 同上 | `d33f81b9806237d9cab190a6f160c2cfa794bb46` |

current Relay source 锚点：

- `codex-relay/service/service.yml` blob：
  `e9e062851e3fc8474a34b89342a1cd6d7c9ddda9`
- `codex-relay/service/api.yml` blob：
  `d2283338ded1f3edaa82ae3fd49d28f36f63b8d6`
- `codex-relay/service/package.yml`：不存在。
- `service.yml` 是 `id = agine.ai/codex-relay`、`version = 0.1.0` 与两个 package dependency
  的 current canonical owner。
- `api.yml` 没有 `serviceCall`；`relayProxy` 是 public instance。其 current contract operation
  集应精确为：
  `relayProxy.responsesCompleted`、`relayProxy.responsesCompletedResult`。

## 3. Ancestry 与 production lineage

精确 ancestry：

```text
merge-base:
5c3322ac3116ac98c4407de4396562ff632ed7b5

merge-base tree:
60321ab8c4fa11e6877a94d44b3e2d078fd428ac

git rev-list --left-right --count <main>...<phase>:
2  1156
```

main-only 两笔 commit 都是文档：

```text
305882351b1e3ea644f1aef3bbc5a1477ab15858 docs: define actor suspension points
8d2fa212a821191d38fe20749fc616e0db79c2a7 docs: refine actor runtime semantics
```

所以不存在可从 current main merge/cherry-pick 的 main-only service authoring production commit。
service-only lineage 的实际边界是：

- `c63b26fa543dde4a8e829ff133061fb1f9b25d23` 的初始 source 曾由
  `compiler/input/src/service_config/{mod,io,validation,overlay}.rs` 读取
  `service.yml` 自有的 id/version/packages/access/http/timeout；
- shared ancestor
  `9fb262197f6a0ca01500c3d98531f047e57c16e7`
  `refactor(compiler): remove terminal service input owners`
  已删除上述 owner、service job/dependency/package input 与对应 1,522 行 service config tests；
- current main 只剩 `scripts/skiff.mjs::detectRootKind` 能把只有 `service.yml` 的目录分类为
  `service`；对应 compiler bin 与 terminal input owner 已不存在，不能生成 canonical artifact。

phase-only 相关提交及其当前语义：

| commit | 当前行为 |
| --- | --- |
| `c69e673ca147f54cc5125ba48d0a09228685474f` | canonical package publication coordinator |
| `eb206aaf381ca1dcd5e4a12e93c94f5fc430a4f5` | service root 必须同时有 `package.yml`；version/dependencies 移到 package owner |
| `fe1ed1453b7c748d4ca3a18569b435d2ee5984e1` | 从 PackageArtifact 投影 service API |
| `dd13c0aaa5565d4ecaae55dfe3e759186f568a7b` | 只选择显式 `serviceCall` roots |
| `86187c0511cb78597f1907d57e9cabf826087826` | CLI 切到 package-owned service authoring |
| `384af34d93238f9f02ce53245f0adeb9df4c5601` | CLI 明确拒绝 service-only root |
| `d4922c50c900ba81e5612e0d535ec282337d7007` | 只接受 phase named HTTP map，拒绝 current Relay route-list/access shape |

候选因此不是 service-only 行为的等价 descendant：commit ancestry 包含 shared deletion，但 phase-only
提交在语义上又把 current source owner替换成了 retired inputs。

## 4. Fresh-store positive probe

命令：

```bash
probe_root="$(mktemp -d /tmp/p5-f400-gate.XXXXXX)"
CARGO_TARGET_DIR="$PWD/build/cargo-target" \
  node scripts/skiff.mjs package publish \
  /Users/geek/workspace/internals/codex-relay/service \
  --artifact-root "$probe_root/artifacts" \
  --environment dev \
  --json
```

本次 probe root：

```text
/tmp/p5-f400-gate.8fbWz7
```

结果：exit `1`，终端错误为：

```text
error: failed to read package manifest /Users/geek/workspace/internals/codex-relay/service/package.yml: No such file or directory (os error 2)
```

失败前后该 probe root 的 entry count 都为 `0`。`compiler/driver/authoring.rs` 当前顺序也是先
`read_user_package_manifest(root/package.yml)`，成功后才
`CanonicalArtifactStore::create(artifact_root)`，所以没有 fresh-store partial write，更没有
stable-store read/write。

这证明：

- root classification / canonical owner：失败；
- service-owned id/version/dependencies：未消费；
- PackageArtifact v7：未生成；
- ServiceContract v4：未生成；
- positive G0 receipt：不存在。

候选常量本身是：

```text
PACKAGE_ARTIFACT_SCHEMA_VERSION = skiff-package-artifact-v7
SERVICE_CONTRACT_SCHEMA_VERSION = skiff-service-contract-v4
```

有 schema 常量不等于能从 current source 生成对应对象。

## 5. 双向 fail-closed 证据

### 5.1 缺 phase-05 一侧：Skiff main

命令把所有显式 state 放在 fresh temp 下：

```bash
probe_root="$(mktemp -d /tmp/p5-f400-main-negative.XXXXXX)"
SKIFF_DEV_HOME="$probe_root/dev-home" \
CARGO_TARGET_DIR="$probe_root/cargo-target" \
  node scripts/skiff.mjs check \
  /Users/geek/workspace/internals/codex-relay/service \
  --artifact-root "$probe_root/artifacts"
```

本次 probe root：

```text
/tmp/p5-f400-main-negative.ZbAKIo
```

main 先把该 root 正确识别成 service-only，随后 exit `1`：

```text
error: a bin target must be available for cargo run
```

probe entry count 为 `0`；main schema 只有 PackageArtifact v2 / ServiceContract v2。因此缺
phase-05 一侧不能冒充 v7/v4 receipt。

### 5.2 缺 service-only 一侧：phase candidate

候选有三层显式 fail-closed：

- `scripts/skiff.mjs::detectRootKind`：
  `service.yml` without `package.yml` 返回 `missing`；
- `compiler/input/src/service_config.rs::read_service_package_root`：
  无条件 `require_control_file(package.yml)`；
- `compiler/driver/authoring.rs::build_authoring_object`：
  在 store creation 前直接读 `package.yml`。

上述 fresh Relay probe 正是该负例。不得通过加回 package file 或 API marker把它改写成正例。

## 6. Focused tests

| 命令 | 结果 | 锁定内容 |
| --- | --- | --- |
| `CARGO_TARGET_DIR="$PWD/build/cargo-target" cargo test --quiet --manifest-path compiler/input/Cargo.toml service_config::tests::` | `11 passed` | 当前 phase parser 要求 package owner，并拒绝 service-owned version/packages 与 current access/HTTP shape |
| `node --test scripts/tests/skiff-test-cli.test.mjs` | `8/8 PASS` | 含显式 “rejects service-only and manifest-less dirs before Cargo” |
| `CARGO_TARGET_DIR="$PWD/build/cargo-target" node --test scripts/tests/package-service-authoring.test.mjs` | `9/9 PASS` | phase synthetic fixture 能生成 v7/v4，但 fixture 有 `package.yml`、`serviceCall: true`，不是 current Relay proof |

这些测试说明 phase 链内部自洽，也精确说明它与 current canonical owner 不等价。

## 7. 唯一 reconciliation：P5-F400A

### 7.1 类型与移植方式

只允许 semantic port，不 merge/cherry-pick：

- 不 cherry-pick main-only commit：它们只有文档。
- 不 revert/cherry-pick `9fb262...`：这会恢复已删除的 terminal files、jobs 与旧 artifact path。
- 只参考 `9fb262^` 中 service-owned id/version/packages 与 current access/HTTP/timeout 的验证语义，
  把所需行为移植到 current v7/v4 pipeline。
- 不恢复 `package.yml`、不合成/要求 `serviceCall`、不加 validator waiver。

### 7.2 精确 production write set

仅以下文件：

1. `compiler/input/src/service_config.rs`
   - 把 `service.yml + api.yml` 读成唯一 `ServiceSourceRoot`；
   - id/version/packages 只来自 `service.yml`；
   - current `access`、route-list `http`、`timeout` 使用 typed validation，不能作为 unknown value
     静默丢弃；
   - service-only root 不查找 `package.yml`；
   - 同时存在 `package.yml` 与 `service.yml` 必须报 ambiguous owner；
   - service-only `api.yml` 出现 `serviceCall` 必须报 retired marker，而不是借 marker选择 operation。

2. `compiler/input/src/package_config/mod.rs`
   与 `compiler/input/src/package_config/manifest_validation.rs`
   - 提取 package/service 两种 source owner 共用的 publication id/version/dependency validator；
   - 生成一个经过同一 alias、exact-version、reserved-name、dependency-access 校验的
     `PackageManifest` compile input；
   - 不建立第二份 identity/dependency validation table。

3. `compiler/driver/authoring.rs`
   - 在读任一 manifest 与创建 store 前按唯一 root kind dispatch；
   - ordinary package 仍由 `package.yml` author；
   - service-only build 从 `ServiceSourceRoot` 进入同一
     `compile_service_package -> publish_package_artifact_records -> write_service_contract`
     v7/v4 path；
   - `package build` 是本 gate 的 package+contract record-only transaction，在 profile/deployment
     lookup 前成功返回；
   - `package publish` 若 current deployment input 尚不能完整表达，必须在任何 record/pointer write
     前 fail closed；不得留下 package/contract partial publication。

4. `compiler/driver/pipeline/mod.rs`
   - 把 service role 作为 typed compile input，而不是 `validated_service_root: bool`；
   - service role 选择全部 public callable roots；ordinary package 仍不生成 service roots；
   - marker presence fail closed，不能作为 current role-selection input。

5. `compiler/projection/src/package_artifact/model.rs`
   与 `compiler/projection/src/package_artifact/projection.rs`
   - 增加明确的 service-root projection policy；
   - service-only role 从 canonical public function/public-instance ABI生成
     `PackageServiceCallRoot`；
   - Relay markerless public instance 必须只生成两个 method roots；
   - 保持 PackageArtifact v7 wire与 identity owner不变。

6. `scripts/skiff.mjs`
   - 恢复 current main 的 terminal root classification：
     package-only -> `package`、service-only -> `service`、两者并存 -> `ambiguous`；
   - `test/check/dev` 与 authoring不得在 Node/Rust boundary 对同一 root 给出相反分类。

明确不在 write set：

```text
artifact-model/**
artifact-identity/**
compiler/contract/** production
deployment/**
runtime/**
router/**
Internals/**
```

因此该 reconciliation 不修改 suspension schema、PackageArtifact v7、ServiceContract v4、identity
preimage、deployment 或 runtime。

### 7.3 精确 test write set

- `compiler/input/src/service_config.rs` inline tests；
- `compiler/input/src/package_config/tests.rs`；
- `compiler/driver/authoring/tests.rs`；
- `compiler/projection/src/package_artifact/tests/projection.rs`；
- `compiler/contract/src/projection.rs` 只增 markerless two-operation projection tests，不改 production；
- `scripts/tests/skiff-test-cli.test.mjs`；
- `scripts/tests/package-service-authoring.test.mjs`。

必须删除/反转当前“service-only 必须拒绝”的 assertions；不能保留两套互相矛盾的 root contract。

### 7.4 必须通过的 acceptance

先跑 owner tests：

```bash
cargo test --manifest-path compiler/input/Cargo.toml service_config
cargo test --manifest-path compiler/projection/Cargo.toml package_artifact
cargo test --manifest-path compiler/contract/Cargo.toml projection
cargo test --manifest-path compiler/Cargo.toml authoring
node --test scripts/tests/skiff-test-cli.test.mjs
node --test scripts/tests/package-service-authoring.test.mjs
```

然后使用一个 fresh root，先在同一 root bootstrap canonical dependencies，再 build 未改动的 current
Relay：

```bash
probe_root="$(mktemp -d /tmp/p5-f400a-acceptance.XXXXXX)"
export CARGO_TARGET_DIR="$probe_root/cargo-target"

cargo run --quiet --manifest-path test-runner/Cargo.toml \
  --bin skiff-package-service-smoke-fixture -- \
  --bootstrap-only \
  --artifact-root "$probe_root/artifacts" \
  --environment dev \
  --platform-source-root "$PWD"

node scripts/skiff.mjs package publish \
  /Users/geek/workspace/internals/packages/llm-api \
  --artifact-root "$probe_root/artifacts" --environment dev --json

node scripts/skiff.mjs package publish \
  /Users/geek/workspace/internals/packages/llm-providers \
  --artifact-root "$probe_root/artifacts" --environment dev --json

node scripts/skiff.mjs package build \
  /Users/geek/workspace/internals/codex-relay/service \
  --artifact-root "$probe_root/artifacts" --environment dev --json
```

最后一条必须满足全部条件：

- 输入仍是上述 exact Internals commit/tree/blobs；
- root 内仍没有 `package.yml`，`api.yml` 仍没有 `serviceCall`；
- stdout 同时有 `packageArtifactReceipt` 与 `serviceContractReceipt`；
- record 分别是 PackageArtifact v7、ServiceContract v4；
- id/version/two dependency exact 来自 `service.yml`；
- operation keys 精确为
  `relayProxy.responsesCompleted`、`relayProxy.responsesCompletedResult`；
- 不生成 `serviceDeploymentReceipt`、pointer 或 assembly；
- 所有 record path 都在本次 `$probe_root/artifacts`；
- missing `service.yml`、双 manifest、retired marker、缺任一 dependency pointer 都在 store write 前失败。

P5-F400A 完成后必须重新执行本 gate；只有 fresh current Relay receipt 成立时才能返回新的
`G0_PASS` candidate commit/tree。当前 `bb0f29c...` 不能进入 F395 N0。
