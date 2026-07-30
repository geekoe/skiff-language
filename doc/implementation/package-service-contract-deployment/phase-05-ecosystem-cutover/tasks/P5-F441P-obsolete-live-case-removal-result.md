# P5-F441P Obsolete live case removal result

状态：`IMPLEMENTATION_PASS / PREEXISTING_UNRELATED_FULL_GATE_BLOCKED`。

本 leaf 已按用户决定精确删除两条无价值 live case 及其专用 normal-source helper；所有剩余
runtime-live case 都由 canonical source-root integration 真正 pure compile。聚焦合同已转绿，
但任务规定的完整 integration 文件仍被一个可在未含本 leaf 改动的 integration tree 上独立复现的
websocket fixture build-id golden mismatch 阻塞。

## 1. 输入、提交与写集

- 任务声明 implementation baseline：`c5777f16`；
- leaf dispatch HEAD：
  `334673715c8fd406ff40337375ff990de6ef58f6`；
- implementation：
  `5fcb53e81e4666ad25698e99cdd6ae698377316e`
  （tree `8836e2890b353dee778f41bb273e9c9267a74268`）。

Implementation 只修改任务允许的五个文件：

- `runtime/live-tests/internal/operation.live.test.skiff`；
- `runtime/live-tests/internal/file_live.live.test.skiff`；
- `runtime/live-tests/internal/operation.skiff`；
- `runtime/live-tests/internal/file_live.skiff`；
- `test-runner/tests/package_service_contract_deployment.rs`中的canonical live source-root test。

没有修改syntax、testing reference、test-runner production、Router/Runtime production、HTTP
manifest、script、其它fixture/task/result。本文由独立result-only commit交付。

## 2. Test-first RED

先只修改canonical live source-root test：

- 为DB、file、HTTP、operation四个tracked source冻结精确case数；
- 把operation加入与其它source相同的`compile_package_test_overlay`路径；
- 增加obsolete case、magic identifier与专用helper反向断言；
- 将runtime-live pure compile总数冻结为12。

随后运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --test package_service_contract_deployment \
  canonical_live_source_roots
```

旧source按预期得到`0 passed / 1 failed / 28 filtered`，精确首错为：

```text
canonical runtime-live source internal/operation.live.test.skiff
must not retain obsolete __skiffPayload
```

确认RED后才删除source。

## 3. 精确删除与终态

全仓consumer搜索确认没有其它真实consumer后，删除：

- `live operation dispatch crosses runtime binary payload boundary`；
- `payloadRoundTrip`；
- `live file runtime rejects stream above file guard limit`；
- `liveFileOverLimitChunks`及只被它调用的`liveFileSixtyFourMiBChunk`。

终态pure compile矩阵为：

| source | case数 |
| --- | ---: |
| default encrypted test-only source | 1 |
| runtime DB | 4 |
| runtime file | 3 |
| runtime HTTP | 4 |
| runtime operation | 1 |
| runtime-live合计 | 12 |
| default encrypted + runtime-live总计 | 13 |

operation只保留runtime-owned fixture case；file只保留原有三个lifecycle case；DB与HTTP各四条
完全未改。既有canonical receipt断言未修改，仍精确验证三个PackageArtifact、三个
ServiceContract、三个ServiceDeployment、`21/13/6` gateway/ingress、39 unary与1
server-stream。

## 4. 验证

所有Cargo命令均使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| focused `canonical_live_source_roots` | PASS，1 passed / 28 filtered |
| 完整`package_service_contract_deployment` | BLOCKED，27 passed / 1 failed / 1 ignored；本leaf canonical test在同一运行中PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

完整文件的唯一失败为既有
`ecosystem_http_private_wrappers_compile_for_all_owned_source_fixtures`：

```text
actual   skiff-package-build-v10:sha256:87120182c6a652e8d52fa530f0a93d86490bb02a0141d6c3924bf82bddfd50ad
expected skiff-package-build-v10:sha256:5ce089038445f6ea1bf05a5d8876ebb784c9193f4509ee993f0eb6b415c25880
```

同一filtered test已在干净integration worktree
`/Users/geek/workspace/skiff-phase-05-integration`的`e732b7d2`独立复现相同失败；该tree相对本
leaf dispatch只增加Router websocket broker代码与result，没有修改test-runner、compiler、std、
prelude或上述fixture。因此本 leaf 没有越过唯一写集刷新无关golden。

规定反向搜索：

```bash
rg -n '__skiffPayload|liveFileOverLimitChunks|payloadRoundTrip|later execution|discovered only|rejects stream above file guard limit' \
  runtime/live-tests test-runner/tests/package_service_contract_deployment.rs
```

为0命中（`rg` status 1）。补充搜索同时确认已删除的operation case title与
`liveFileSixtyFourMiBChunk`为0命中。

## 5. 停止条件与隔离

剩余operation、file、DB与HTTP case全部compile，没有暴露新的test-runner production blocker，
因此未触发任务定义的`TASK_SCOPE_EXPANDED`，也没有修改runner。完整文件的预存无关golden failure
保留为上层gate blocker，本文不把它伪报为PASS。

本 leaf 未运行任何live selector/workload，未启动或访问stable、instance、Mongo、Router、
Runtime、telemetry、watch或网络；未派sub-agent，未merge、rebase或push。
