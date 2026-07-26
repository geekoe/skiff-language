# P5-F371 Bootstrap RuntimeAssembly gateway field correction result

状态：Completed（fresh artifact bootstrap机械前置；不表示S7 test-runner ingress、
dispatch wire、WebSocket fixture或ecosystem smoke consumer已经迁移）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base / task checkpoint | `8b7f3f20b91bbd280da3ec053024bf736c8252b2` | `e571c0a797f450289f61dce3d60ffe49c74ed8c6` |
| production/tests | `69fbdbcc4774cb0b94089869bb3813a7e7ab5a1e` | `803f721a63f272728ad7385875f1e78563c9415c` |

工作分支为`codex/p5-f371-bootstrap-gateway-field`，worktree为
`/Users/geek/workspace/skiff-p5-f371-bootstrap-gateway-field`。本leaf没有merge/rebase
integration，没有push，没有启动或修改stable/live，也没有修改lockfile、其它test-runner
synthesis、artifact/deployment/runtime/Router、service或三仓库源码。

## 2. Mechanical correction

`test-runner/src/bin/package_service_smoke_fixture.rs::initialize_empty_environment`构造
`RuntimeAssembly`时只把已经退出schema的

```text
global_ingress: Vec::new()
```

替换为当前required字段

```text
gateway_ingress: Vec::new()
```

其它字段和控制流没有变化。bootstrap assembly继续保持empty roots、deployments、contracts、
packages、package link plan、service binding templates和activation templates；没有gateway entry、
selector、service operation、兼容转换或伪造identity。assembly identity仍由
`assign_runtime_assembly_identity`从当前canonical empty assembly计算。

## 3. Real fresh bootstrap evidence

在production commit对应binary上，以`mktemp -d`创建全新artifact root并实际执行：

```text
build/cargo-target/debug/skiff-package-service-smoke-fixture \
  --bootstrap-only \
  --artifact-root /tmp/skiff-p5-f371-bootstrap-final.drufIC \
  --environment p5-f371-final \
  --platform-source-root /Users/geek/workspace/skiff-p5-f371-bootstrap-gateway-field
```

命令退出码为0。receipt与落盘record逐项验证结果：

| 证据 | 实际值 |
| --- | --- |
| receipt schema / generation | `skiff-package-service-bootstrap-v1` / `0` |
| assembly identity | `skiff-runtime-assembly-v2:sha256:247fc2b3714bf715dc7918a10618be49493645efbbc0f293fc7b3d2e4d32b50f` |
| assembly ingress | required `gatewayIngress: []`；`globalIngress`不存在 |
| environment activation | `p5-f371-final` generation `0`指向上述exact assembly；`pending: null` |
| std coordinate | `skiff.run/std@1.0.0` |
| std build | `skiff-package-build-v8:sha256:eb7a294930e76caabe86d73600f84da63cdb4e88ac8c763bbb64377ffe7ea69f` |
| std package record | `records/package-artifacts/skiff~drun~sstd/1.0.0/eb7a294930e76caabe86d73600f84da63cdb4e88ac8c763bbb64377ffe7ea69f/package.json` |
| std pointer | `pointers/package-artifacts/skiff~drun~sstd/1.0.0.json` |
| receipt-owned records | package record、pointer和16个File IR records均存在；0 resource records |

落盘assembly的键集合精确为
`activationTemplates`、`assemblyIdentity`、`gatewayIngress`、`packageLinkPlan`、
`resolvedContracts`、`resolvedDeployments`、`resolvedPackages`、`roots`、`schemaVersion`和
`serviceBindingTemplates`；所有collection均为空。receipt pointer与落盘pointer逐值相等。

已有direct integration probe也实际启动该binary到自己的fresh test root，并用typed
`PackageArtifactRef`、`PackageArtifactPointerPath`和`CanonicalArtifactStore`复核official std
package/pointer：

```text
cargo test -p skiff-test-runner --test canonical_std_seed_bootstrap \
  bootstrap_only_seeds_the_exact_std_records_and_pointer_receipt
```

结果为`1 passed; 0 failed`。

## 4. Verification

| 命令 | 结果 |
| --- | --- |
| `cargo check -p skiff-test-runner --bin skiff-package-service-smoke-fixture` | PASS；仅既有`skiff-compiler-source` warnings |
| `cargo test -p skiff-test-runner --bin skiff-package-service-smoke-fixture -- --list` | PASS；如实枚举`0 tests, 0 benchmarks` |
| 上述real fresh `--bootstrap-only` binary执行与receipt/落盘record验证 | PASS |
| targeted `canonical_std_seed_bootstrap` direct integration probe | PASS；1/1 |
| `rg -n 'global_ingress' test-runner/src/bin/package_service_smoke_fixture.rs` | PASS；零匹配 |
| `git diff --check` | PASS |

## 5. 自验收矩阵

| 任务条款 | 代码/动态证据 | 结果 |
| --- | --- | --- |
| current empty assembly字段 | 唯一production diff为`gateway_ingress: Vec::new()` | PASS |
| 不制造gateway/operation/identity | fresh落盘assembly所有collection为空；identity由canonical helper计算 | PASS |
| 不迁移S7 consumers | diff不含其它test-runner、script、wire、WebSocket或ecosystem fixture | PASS |
| canonical std package/pointer receipt | real binary receipt、落盘records与typed direct integration probe | PASS |
| fresh root而非既有artifact | `/tmp/skiff-p5-f371-bootstrap-final.drufIC`由本次`mktemp -d`创建 | PASS |
| ownership与运行边界 | 未修改或运行stable/live及禁止生产域 | PASS |

## 6. Out-of-scope observation

额外只读调用现有
`scripts/lib/package-service-ecosystem-smoke-oracle.mjs::validatePackageServiceBootstrapReceipt`
会因其仍把assembly identity写死为
`^skiff-runtime-assembly-v1:sha256:...$`而拒绝本次正确的v2 identity。该oracle是F358已经明确列出的
后续cross-system consumer，不影响binary fresh bootstrap、typed std probe或本任务验收；本leaf没有
越权修改它，也没有因此返回`TASK_SCOPE_EXPANDED`。
