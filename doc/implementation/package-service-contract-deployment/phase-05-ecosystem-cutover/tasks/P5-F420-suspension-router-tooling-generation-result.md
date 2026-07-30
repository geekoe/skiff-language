# P5-F420 Suspension Router, tooling and current-generation oracles result

状态：`TASK_SCOPE_EXPANDED`。本节点只形成了既定 write set 内的 mechanical implementation
checkpoint；F421 **未解除**。

## 1. Exact candidate 与 checkpoint

- integrated start：
  `b58dbde08a0de76b9c5cf94398df76f5f5717f11`；
- integrated start tree：
  `c7d9fd6d10578f483558358519c8b7734e9c064b`；
- task commit：
  `4c719b33131fff39a2f8f2e692b88b4710aae892`；
- task tree：
  `2cdde01a073205c004329fc2c4fbe93943a9b98b`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`；
- implementation checkpoint：
  `b66bee3cacc645601ca38cea182a54e7dcab2060`；
- implementation tree：
  `10a6d0ed698c4cc83d36e2618fffe3107942745e`。

`git merge-base --is-ancestor` 对 integrated start 与 F415 都返回 `0`。implementation checkpoint
共修改既定 write set 内 39 个文件；没有修改前三层 production、`test-runner` production 或
ecosystem 仓库。

## 2. 修改前 listing / probe

按任务要求先 listing/probe，再修改。

### Node 五文件组

```text
tests 36
pass 33
fail 3
```

真实首错依次为：

1. `package-service-authoring.test.mjs` 仍断言
   `skiff-service-protocol-v4`，真实 compiler receipt 已是 v5；
2. I02 oracle 仍要求 Package build v4，fixture 当时给 build v9；
3. `validI02SpawnSubmitFixtureReceipt` 仍写已不存在的
   `entrypoints[0].contract.serviceId`。

审计记录的 34/2 已被当前 exact candidate 上新增的 protocol v4 断言首错取代；没有删除或过滤
测试。

### Router 五文件组

`vitest list` 实际列出 164 个测试。修改前 execution：

```text
Test Files 3 failed / 2 passed
Tests      11 failed / 153 passed / 164 total
```

真实首错包括：

- compiler fresh record 已输出 PackageArtifact v9，但断言仍是 v8；
- filesystem loader 仍只接受 PackageArtifact v8 / build v9；
- RuntimeAssembly reader 仍只接受 ServiceProtocol v4；
- Router PackageUnit positive fixture 仍写 v1，当前 identity CLI 只接受 v2。

### test-runner

规定的 `--list` 在列出测试前编译失败，首轮精确错误为：

```text
unresolved import skiff_artifact_model::BoundaryCancellationContract
no field may_suspend on BoundaryOperationContract
no field cancellation on BoundaryOperationContract
```

因此修改前没有伪报“24 listed”。

### identity single-source checker

修改前 checker 返回 15 个 stale requirement；首错为
`artifact-identity/src/constants.rs is missing owned ASSEMBLY_IDENTITY_PREFIX`。其余集中在已删除
owner、F415 collection mapping、gateway ingress/current assembly shape，以及已退出当前执行路径
的 adapter/delegation要求。

## 3. 已完成的 mechanical checkpoint

implementation checkpoint 已完成以下 write-set 内收敛：

- Router runtime protocol与 direct RuntimeAssembly join 改读 ServiceProtocol v5；
- filesystem loader 改读 PackageArtifact v9 / Package build v10；
- Router PackageUnit pointer/type 改为 v2；
- compiler-generated manifest positive oracle 改为 PackageArtifact v9、build v10、
  ServiceContract v5、protocol v5；
- authoring/source-suite/I02 validators 改为 RuntimeAssembly v2；
- ecosystem fixture/oracle 改为 Package build v10 / local ABI v7；
- runtime-boundary checker 删除旧 `BoundaryCancellationContract` requirement，改锁统一
  `async_stream_cancel::execute_service_call` lane；
- identity single-source checker改锁当前 implementation-links v2、PackageArtifact build/local ABI、
  package schema与service protocol marker/prefix owner，并接受F415
  `collection_name_mapping`、gateway entry与RuntimeAssembly `gateway_ingress` shape；
- test-runner test 删除 contract-level `may_suspend` / `cancellation` 字段断言，保留 concrete
  callable `CallableMayEffects.may_suspend == true`；
- test-runner 的service selector、exact contract/provider binding断言仍在
  `package_service_contract_deployment.rs:609-623`；没有修改
  `canonical_package_bindings` production owner。

current positive generation清单：

| 对象 | checkpoint |
| --- | --- |
| PackageArtifact | `skiff-package-artifact-v9` |
| Package Local ABI | `skiff-package-local-abi-v7:sha256` |
| Package build | `skiff-package-build-v10:sha256` |
| PackageUnit | `skiff-package-unit-v2` |
| ServiceContract | `skiff-service-contract-v5` |
| Service protocol | `skiff-service-protocol-v5:sha256` |
| RuntimeAssembly | `skiff-runtime-assembly-v2` |

## 4. Dynamic fixture regeneration receipt

实际运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
cargo test --locked -p skiff-artifact-identity \
  --test regenerate_dynamic_build_id_fixture -- --ignored --test-threads=1
```

producer source checkout为 task commit/tree
`4c719b33131fff39a2f8f2e692b88b4710aae892` /
`2cdde01a073205c004329fc2c4fbe93943a9b98b`；最终生成记录由 implementation
checkpoint/tree `b66bee3c...` / `10a6d0ed...` 捕获。没有手工猜 identity hash。

首次运行真实停在 stale FileIR 缺少 `serviceCallRefs`；将 tracked case 输入机械迁移到 FileIR v8、
current named-union branch shape与PackageUnit v2后，official generator：

```text
1 passed / 0 failed
```

生成记录：

- case：
  `cross-system-fixtures/dynamic-build-id-parity/case.json`；
- service unit：
  `units/services/example~com~~dynamic-golden/2026.06.04.json`；
- dynamic build：
  `skiff-service-build-v1:sha256:ed32b93ba8d48f7cb93cb4ef13720e943eec758b3e87a757eaec32b0a290ed26`；
- service unit identity：
  `skiff-service-unit-v1:sha256:1ed23e89365f01cde88881a41d5f13a14895fc899d39be255ef9f3a9e98c81c7`。

该共享 fixture 的 PackageUnit lane按当前模型使用PackageUnit v2、legacy-unit build v3/local ABI v2；
它不伪装成 PackageArtifact v9 lane，且已不含legacy build v2。

## 5. 修改后证据与 legacy negative inventory

已运行：

| 命令 | 结果 |
| --- | --- |
| Node 五文件组 | 36 total / 35 pass / 1 fail |
| official dynamic fixture generator | 1/1 PASS |
| `node scripts/check-artifact-identity-single-source.mjs --self-test` | PASS |
| `node scripts/check-artifact-identity-single-source.mjs` | PASS |
| `git diff --check` | PASS |

current positive反搜中不再有PackageUnit v1。旧 token只剩明确负例：

- filesystem loader拒绝PackageArtifact v8、canonical build v9和RuntimeAssembly v1；
- runtime protocol拒绝ServiceProtocol v4；
- assembly request mutation使用RuntimeAssembly v1验证失败关闭。

concrete `maySuspend` 仍保留在 FileIR/callable semantic facts；删除的只是
contract/interface suspension fields。

## 6. 唯一范围扩张 blocker

修改后 Node 五文件组唯一失败：

```text
I02 combined owner performs valid commit, two zero-I/O requests, and real rollback
actual URL: http://127.0.0.1:46000undefined
expected: /\/probe$/
```

current v2 receipt entrypoint的exact shape为：

```text
{
  deployment,
  gatewayEntryKey,
  gatewayEntryIdentity,
  mode,
  selector: { protocol, host, method, path }
}
```

但范围外 production owner
`scripts/lib/package-service-i02-combined-real.mjs::requestTypedUnary` 仍读取旧扁平
`unary.method`、`unary.path`、`unary.host`。只改test fixture会掩盖real I02/F421同一生产故障；
让oracle输出双形态则会新增明确禁止的compatibility adapter。

最小新增 owner只有：

```text
scripts/lib/package-service-i02-combined-real.mjs
```

建议后继 `P5-F420A` 直接读取并校验
`unary.selector.{method,path,host}`，随后从 implementation checkpoint 重跑F420完整矩阵。

## 7. 未绿 / 未跑 gates

遵守 `TASK_SCOPE_EXPANDED` 后没有继续修改或扩大验证：

- Node 五文件组仍为 35/36；
- Router 164 execution尚未在checkpoint上重跑；
- Router `tsc`未跑；
- test-runner listing/execution尚未在checkpoint上重跑；
- `node scripts/run-skiff-tests.mjs`未跑；
- `verify --only router`与`verify --only tooling`未跑；
- `cargo fmt --all -- --check`未跑。

因此本result不宣称N4通过，F421仍被阻断。
