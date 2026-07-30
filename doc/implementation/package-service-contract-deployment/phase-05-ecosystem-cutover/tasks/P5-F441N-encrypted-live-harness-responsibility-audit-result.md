# P5-F441N Encrypted live harness responsibility audit result

状态：`PASS / THREE_STABLE_RESPONSIBILITY_BOUNDARIES_PROVEN`。

本节点只读检查F441K后的`encrypted-storage-live-harness.mjs`及direct test。没有修改文件、运行测试、
启动进程或访问live状态。结论不是按行数机械拆分：当前1076行文件已形成三个相互独立、从facade单向依赖的
子图，可以在不改变export、class API、公开字段或行为的前提下拆出。

## 1. 输入

- 检查状态包含：
  - `P5-F441K-encrypted-storage-live-harness-canonicalization-result.md`
  - implementation `478c22de`对应代码；
- 直接架构事实仍来自F441J/F441K；本节点只拥有模块职责事实，不改变live语义。

## 2. Pure contract owner

建议新建：

```text
scripts/lib/encrypted-storage-live-contract.mjs
```

它拥有无I/O的live接口与activation生命周期契约：

- `repoRoot`与相关contract constants；
- `encryptedStorageTestRunnerArgs`；
- `encryptedStorageBuildArgs`；
- `encryptedStorageProductionAssembly`；
- `encryptedStorageIngressRequest`；
- `runEncryptedStorageTestLifecycle`；
- 这些函数专用的纯校验/helper。

这些symbol不依赖`EncryptedStorageLiveHarness`、Mongo、端口、文件或进程；class只调用它们。因此依赖方向
固定为：

```text
encrypted-storage-live-harness.mjs
  -> encrypted-storage-live-contract.mjs
```

原harness路径必须re-export现有public helper与`repoRoot`，避免调用方迁移。

## 3. Mongo probe owner

建议新建：

```text
scripts/lib/encrypted-storage-live-mongo-probe.mjs
```

它拥有：

- mongosh/EJSON adapter；
- database/collection/document读写；
- transient encrypted storage discovery；
- replica-set initialization；
- encrypted envelope/database response decode。

当前九个Mongo方法、storage observation与replica initialization只围绕同一`mongoJson`adapter形成聚类。
依赖方向固定为：

```text
harness -> mongo-probe -> mongosh-json-command
```

probe只接收`mongoPort`、`cwd`与可注入command seam，不读取harness对象。harness保留所有现有同名方法作为
delegation，使checker调用方式不变。

## 4. Instance resource owner

建议新建：

```text
scripts/lib/encrypted-storage-live-instance-resources.mjs
```

它拥有：

- isolated port范围、禁用端口与lease；
- temp instance paths/config生成；
- ownership-checked process-group fallback termination；
- 对应纯校验/helper。

当前资源职责散落在static create、port helper、config serializer、cleanup fallback与process-group
primitive中，并包含实际`process.kill`安全边界，属于独立owner。依赖方向固定为：

```text
harness -> instance-resources -> local-port-lease
```

factory返回`{ paths, portLease }`，不得import或构造harness class。process-group逻辑必须保持
“验证owned PGID -> 记录公开cleanup状态 -> TERM -> wait -> survivor KILL”的可观察顺序。

## 5. Facade保留职责

原`encrypted-storage-live-harness.mjs`继续拥有：

- public facade/class；
- initialize/build/test/activation高层编排；
- keyring/runtime restart；
- HTTP retry；
- cleanup高层顺序与公开状态；
- command/log orchestration；
- 对Mongo/resource模块的兼容delegation。

keyring helper、HTTP retry与log reader体量小且紧密依赖facade状态，本次不再拆。现有另一套isolated test
instance config/replica-set逻辑虽外形相似，但supervisor/timeout契约不同，不顺手合并。

## 6. 验证边界

最小回归要求：

- 现有direct test继续从旧harness路径import，证明re-export兼容；
- 精确export-surface断言；
- pure contract现有command/receipt/lifecycle断言不变；
- Mongo probe以fake mongosh证明URL/expression/cwd和canned response decode；
- instance resource以fake lease/fs/kill/delay证明port、config、ownership、TERM/KILL顺序；
- retired-surface扫描扩展到三个新模块，防止只把禁用字符串搬走。

该拆分不需要修改source roots、test-runner、Router/Runtime、live config或调用者。若实现中出现循环依赖或
必须改变public API，说明审计前提被证伪，应停止而不是继续拆。
