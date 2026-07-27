# P5-F441I Canonical live source-root authoring result

状态：`PASS / CANONICAL_LIVE_SOURCE_ROOTS_AUTHORED`。

## 1. 输入、提交与写集

- 任务声明 implementation baseline：
  `c3878e3df9e010381bc6bf0dcfb60379e5f6dcf7`
  （tree `5256045bdc82c89eac7c878b3cbb901cf8130fb1`）。
- leaf dispatch HEAD：
  `84f598674393173014aae9f7274fffbd4b7684aa`
  （tree `b2d3b3818ff83434a117d3537dc0d014c536a35b`）。
- implementation：
  `182ca65576a02cd19e83079102d4d6ed86ce7496`
  （tree `a03a5bd4e6f6b346d486a77fb79c4bc4c8b726d4`）。

Implementation 共修改 32 个文件，全部位于任务唯一写集：

- 三个 tracked source root；
- encrypted store package 与 runtime kit canonical package；
- `test-runner/tests/package_service_contract_deployment.rs` 中本 leaf 的真实 root
  compile / receipt test及直接 helper。

没有修改 test-runner production、scripts/harness/plan、Compiler、Router/Runtime production、
其它 fixture、其它 task/result或 stable/live 状态。本文由独立 result-only commit交付；
其 commit/tree 由最终交付消息记录。

## 2. Test-first RED

先只增加真实 root probe，再运行：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --test package_service_contract_deployment \
  canonical_live_source_roots
```

legacy root按预期得到 `0 passed / 1 failed / 28 filtered`。精确首错为：

```text
runtime/encrypted-storage-live/default-service must own package.yml before compilation:
failed to read package manifest .../default-service/package.yml:
No such file or directory
```

随后才迁移 control files、source ownership与dependency package位置。

## 3. Canonical authoring终态

### 3.1 三个service root

三个root现在都拥有`package.yml`、`api.yml`、精简`service.yml`、`http.yml`与一个固定profile：

- default与mapped为ordinary service；`service.yml`只保留`id`，
  profile固定为`config.dev.yml`；
- runtime-live为`id: skiff.run/runtime-live`、`kind: test`，
  profile固定为`config.skiff-test.yml`，不存在`config.dev.yml`；
- 三个`api.yml`均为`{}`，真实producer生成三个zero-operation contract；
- 三个root均未创建`websocket.yml`，service文件不再拥有version、packages、HTTP、routes或timeout。

default与mapped profile均只拥有`timeout: 120000`及完整normal deployment policy/state binding。
default的`encryptedLive.testRunnerSecret`由normal private accessor声明并绑定固定测试值。
mapped的service-local与dependency database state key绑定到同一namespace。

runtime-live profile绑定`runtime-live-store`，并精确绑定normal private source声明的四个config
requirements：

```text
runtimeLive.db
runtimeLive.file
runtimeLive.httpAdapter
runtimeLive.operation
```

target environment、router/ingress URL、service selector与version均未进入profile。

### 3.2 HTTP与dependency package

- `http.yml`顶层精确为default 21、mapped 13、runtime 6个冻结key；
- 40条均为`rawHttp`，每条唯一adapter arg均为
  `request <- http.request`；
- encrypted的34条entry逐条拥有`internal.live.guard`；
- `typedJsonEcho`仍由handler内部手工decode，gateway仍为raw；
- `runtime.guarded`仍为GET；
- `runtime.stream`为唯一server-stream，其余39条为unary。

runtime package entry绑定当前实现package的
`internal.http_adapter.packageEcho` private wrapper；wrapper再使用current package dependency
resolver调用runtime kit。gateway没有直接绑定dependency alias。

runtime kit已从递归source位置移至：

```text
runtime/live-tests/.skiff-packages/example~com~~runtime-live-kit/1.0.0
```

其`package.yml`、`api.yml`与`http.skiff`形成独立canonical package；旧inline API owner与旧
`package-store`位置均已删除。encrypted store package声明自己的database state requirement；
mapped manifest使用唯一accepted字段`collection_name_mapping`。

### 3.3 Test-service production ownership

- `RuntimeLiveDoc`、DB object、rollback/claim probe helpers移入normal
  `internal/db_live.skiff`；
- file stream/size helpers移入normal`internal/file_live.skiff`；
- operation、DB、file与HTTP marker均由normal accessor/handler读取；
- test-only文件消费这些normal owners并保留assertions；
- 已退出的`runtime-live.config.example.json`已删除；
- HTTP test不再从test config猜router URL、service id或version。

纯compile probe额外证明：

- default encrypted test-only source：`1` case可对normal owner编译；
- runtime DB/file/HTTP test-only sources：`12` cases可对normal owners编译；
- operation source的`2` cases保持可发现；任务明确留给后续execution owner的
  `__skiffPayload` lowering未在本leaf吞并；
- over-limit expected-platform-error执行语义同样保持后继ownership。

## 4. Fresh producer receipt

integration test每次创建fresh temporary artifact store，先seed compiler-owned canonical std，
再发布任务要求的两个business dependency package，随后编译三个真实service root。所有断言读取
canonical store中的真实producer records，不读取本机stable artifacts。

| receipt | 数量 / 结果 |
| --- | --- |
| business dependency PackageArtifact | 2：encrypted store、runtime kit |
| service implementation PackageArtifact | 3：default、mapped、runtime-live |
| ServiceContract | 3，全部0 operation / 0 package type requirement |
| ServiceDeployment | 3，operation binding均为0 |
| gateway / ingress | `21/21`、`13/13`、`6/6` |
| HTTP entry | 40：39 unary、1 server-stream |
| test-only pure compile | 13 compiled，2 operation cases discovered |

receipt逐条冻结40个key、host、method、path、current-package handler、guard、pre absence、
adapter kind/arg/source与dispatch。current v2真实producer identity为：

```text
raw unary:
skiff-gateway-entry-v2:sha256:0fd289d7eec4e03b01e9e8f5633aedd7e1cc64158fa7932f99a9686e559c02f2

raw server stream:
skiff-gateway-entry-v2:sha256:1aef41f397b7c817110cb0cc74a7b472ba9732c5ac6bcfe6e219e3ac51ab6bd0
```

没有复制F388的v1 golden。mapped package requirement与deployment binding均精确携带：

```text
package_secret -> mapped_package_secret
```

runtime deployment diagnostic精确为
`skiff.run/runtime-live@0.1.0 (skiff-test)`，证明实际选择固定test profile。

## 5. 验证

所有Cargo命令均使用共享：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| focused `canonical_live_source_roots` | PASS，1 passed / 28 filtered |
| 完整`package_service_contract_deployment` | PASS，28 passed / 1 ignored |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

完整integration中唯一ignored仍是既有
`I16/G16 shared-target identity probe only`，与本leaf无关。

## 6. 反向搜索与隔离

规定的第一条搜索：

```bash
rg -n '^[[:space:]]*(version|packages|http|routes|timeout):|root\.' \
  --glob service.yml --glob http.yml \
  runtime/encrypted-storage-live/default-service \
  runtime/encrypted-storage-live/mapped-service runtime/live-tests
```

为0命中（`rg` status 1）：没有legacy owner/key，也没有需要分类保留的
`http.request`误命中。

第二条搜索：

```bash
rg -n 'collectionNameMapping|runtime-live\.config\.example|package-store/.+runtime-live-kit' \
  runtime/encrypted-storage-live runtime/live-tests
```

同样为0命中（`rg` status 1）。

补充搜索确认test-only sources不再直接读取config，也不再含
`runtimeLive.routerHttpUrl`、`runtimeLive.serviceId`或`runtimeLive.version`。

本leaf未启动或访问Mongo、Router、Runtime、telemetry、instance、watch、stable、固定端口或任何
live workload；未派sub-agent，未merge、rebase或push。canonical compile没有暴露需修改
production/scripts的owner，因此未触发`TASK_SCOPE_EXPANDED`。
