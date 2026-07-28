# P5-F445H-I7-S0 real-source / artifact checkpoint result

状态：`PASS / S0 COMPLETE`。

```text
S0_COMPLETE = YES
S1_UNBLOCKED = YES
A_UNBLOCKED_BY_S0 = YES
C_UNBLOCKED_BY_S0 = YES
```

这里的 A/C 结论只表示它们对 S0 的依赖已经解除；它们仍须满足各自其它直接父节点与候选输入，
本结果不把 A、C 或整个 I7 提升为完成。

## 1. 执行身份

| 项 | 值 |
| --- | --- |
| baseline commit | `54fb087f122c53aed5c017260c7bca43e2b54404` |
| baseline tree | `008d3a05927cdf845004db980d1b46de263612be` |
| task commit | `65aee539257ccc16f924f0cddc479895b392ac08` |
| task tree | `65d0a403a8239ba72a6bee05cefb15a25d4e9990` |
| implementation commit | `163466b48e70a3443e6dc5af59bc8dc3222f287b` |
| implementation tree | `c1643b8c0f42d488327583f4de5deebad1fd4ac7` |
| branch | `codex/p5-f445h-i7-s0-source-artifact` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-s0-source-artifact` |
| official-package candidate | `b06d7aaf16b6914837de1f74920fd3f626040472` |
| official-package tree | `fb9db28a7d1bd3babafd1dfa7a23687e393ff856` |
| official-package worktree | `/Users/geek/workspace/skiff-packages-phase-05-integration` |

task中记录的初始 official-package provenance
`19cfab5dfc827450d37e1a103d21f31f8effa4f0`不能解析当时的 official source；P0 随后提供上表
candidate。F76最终证据严格取自该更新后的 clean candidate。

## 2. 实际写入

新增：

```text
test-runner/fixtures/package-service-current-scope/consumer/**
test-runner/fixtures/package-service-current-scope/helper/**
test-runner/fixtures/package-service-current-scope/provider/**
P5-F445H-I7-S0-real-source-artifact-checkpoint.md
P5-F445H-I7-S0-real-source-artifact-checkpoint-result.md
```

修改：

```text
test-runner/tests/package_service_contract_deployment.rs
test-runner/src/canonical_package/tests/combined.rs
```

没有修改 compiler、runtime、Host、Router、artifact model/identity、公开 schema、Internals 或
`skiff-packages` production。`combined.rs`只把 F76 test provenance机械刷新为 current
`tests/<package>` service root、精确 subject package build与非零 case/binding receipt。

## 3. Source 到 artifact 的纵向 receipt

checked-in multi-root fixture包含 helper、provider与consumer。consumer的真实 `.skiff` source
为以下六类 carrier各写入两层嵌套 `timeout(...)`，共十二个 timeout File IR节点：

- HTTP unary与server stream；
- WebSocket `requestJsonToConnection<string, string>`，保持三个业务参数；
- file create/read；
- Actor `getOrCreate`与method call；
- canonical `payments/echo` service call。

同一 source经 canonical authoring producer形成 provider/consumer package、contract、
deployment与runtime assembly；canonical store round-trip后，typed records与reference均保持
一致。HTTP split manifest产生 unary与server-stream gateway，WebSocket split manifest只产生
connect gateway。fixture不含 cancellation、requestId、legacy `ServiceTimeoutConfig`或
deployment timeout继承。

冻结的 exact identity tuple为：

| receipt | identity |
| --- | --- |
| File IR | `skiff-file-ir-v9:sha256:9e0b0915efe308c05081320012f282ef81e37e9536c02f16af0a770a021f60f6` |
| Package build | `skiff-package-build-v10:sha256:9b03476e93f5ccb66dc69ff899f4a8fb9c68593e70c5aeda94d4e865aab688ad` |
| Package local ABI | `skiff-package-local-abi-v7:sha256:605b18a2b130957f4b1feec499583334601b3788514ea851530b6623a017aed4` |
| Service protocol | `skiff-service-protocol-v5:sha256:9ea7ac440bd594ef31632c1c1914b40f2e92957e7fb0f73f587f4cb4d8563fa5` |
| Deployment artifact | `skiff-deployment-artifact-v3:sha256:aa74be018958d2e2375b91e500e4f73b6fea8fb97c4d694962d6745fe475791c` |
| HTTP unary gateway | `skiff-gateway-entry-v2:sha256:0fd289d7eec4e03b01e9e8f5633aedd7e1cc64158fa7932f99a9686e559c02f2` |
| HTTP stream gateway | `skiff-gateway-entry-v2:sha256:1aef41f397b7c817110cb0cc74a7b472ba9732c5ac6bcfe6e219e3ac51ab6bd0` |
| WebSocket gateway | `skiff-gateway-entry-v2:sha256:f385624021966bab998385e1fd2c88804b51992f15f9c9d76c05d3e17a75018d` |
| Runtime assembly | `skiff-runtime-assembly-v2:sha256:ec66d8a209e65198ee5b82086a365a4b3a98021ef8117e2572c66fee8eac5f6e` |

## 4. Negative matrix

| mutation | 结果 |
| --- | --- |
| 在 `service.yml`内联 `http` | fail closed：unknown field |
| 把 `payments/echo`改回 `payments.echo` | compile fail closed |
| `timeout(250ms)`改为`timeout(251ms)` | File IR与package build identity改变；public local ABI identity不变 |
| 输入 `skiff-runtime-assembly-v1` identity | typed decode拒绝，并要求v2 |

## 5. 验证证据

聚焦 owner证据：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-compiler --test timeout_artifact_lowering --locked` | `PASS`，4/4 |
| `cargo test -p skiff-test-runner --test package_service_contract_deployment canonical_live_source_roots_compile_to_current_receipts --locked -- --nocapture` | `PASS`，1/1 |
| `P5_F76_PACKAGES_ROOT=... cargo test -p skiff-test-runner p5_f76_contextual_callable_provenance_combined --locked -- --ignored --nocapture` | `PASS`，1/1；四个current official test-service roots均有非零cases、精确subject build与一一对应bindings |
| `cargo fmt --all -- --check` | `PASS` |
| `git diff --check` | `PASS` |

完整组件探测 `node scripts/verify.mjs --only compiler,test-runner`没有获得全绿结果，且不得登记为
S0 scoped PASS：

- compiler阶段命中九个候选既有失败：`actor_dispatch_linking`、`prelude_std_schema`、
  `root_path_references`、`runtime_slots`、`shared_fixture_lane_probes`、`std_package_imports`、
  `streams_emit`、compiler-lowering lib与compiler-source lib；症状属于数据库fixture缺口、
  stale prelude pin及旧IR shape断言等既有债务；
- 因compiler阶段提前停止，另行运行 `node scripts/verify.mjs --only test-runner`；它只命中既有
  `ecosystem_http_private_wrappers_compile_for_all_owned_source_fixtures` stale
  websocket-smoke build/deployment/assembly pin。相同test binary中的S0聚焦selector通过。

这些失败不触及本节点零production diff的fixture/test owner，也不是S0 scoped gate。按集成owner
裁定记录为candidate-existing broad-gate debt，不在本节点扩写production或另开任务；最终全仓
与债务裁决仍属于后续集成/J owner。

未运行或改动 stable/live instance、network、MongoDB、OAuth或browser状态。

## 6. 自验收

| 条件 | 结论 |
| --- | --- |
| checked-in real source覆盖全部current-scope carriers | `PASS` |
| File IR/package/contract/deployment/gateway/assembly exact receipt | `PASS` |
| canonical store typed round-trip与service binding closure | `PASS` |
| inline ingress、旧call spelling、timeout mutation、旧assembly identity negatives | `PASS` |
| F76 current official test-root provenance | `PASS` |
| production/schema/identity语义零改动 | `PASS` |
| S0写集与I7R冻结边界一致 | `PASS` |

因此本节点的 source/artifact checkpoint完成，`S0_COMPLETE = YES`；S1以及A/C的S0依赖解除。
