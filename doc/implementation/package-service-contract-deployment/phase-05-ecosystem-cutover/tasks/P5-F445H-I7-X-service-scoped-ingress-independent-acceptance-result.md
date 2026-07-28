# P5-F445H-I7-X Service-scoped ingress independent acceptance result

状态：

```text
PASS
X_COMPLETE = YES
SERVICE_SCOPED_INGRESS_ACCEPTED = YES
PRODUCTION_BUGS = 0
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```

X在冻结candidate上独立复验了D0/K/C/L/R/F的service-scoped ingress纵向合同。最终结果是：
HTTP Host不参与Skiff service route；Router严格使用`x-skiff-service`与`x-skiff-version`
选择精确deployment，再在该deployment内按method/path选择入口；Router到Runtime的frame v2携带
同一个精确deployment。HTTP与WebSocket正负例全部通过。

候选初始存在有界的test-only代际漏迁移。主Agent授权后，X只刷新测试fixture、shared golden和断言，
没有修改production decoder、mapper、schema、identity或路由实现。修复后全部本轮owner gate通过。

## 1. Exact identities

| 项 | 值 |
| --- | --- |
| frozen candidate commit/tree | `57c93c3026a17f8d0c134b80197c294b3a325f52` / `225cd7e9001fac9a55ce9c5ef89db842402f1f98` |
| fixture closure commit/tree | `16ce2275aac42c4a7472ec20d89cd2b7c718e518` / `b03cc70e5e153740b6eb2f0778050868954bd138` |
| independent positive commit/tree | `bd908c1d859c9e3ea9f77216b61d93cb2dc6a616` / `0fbe32815baf55493c79f396fc7bfb8245d985fc` |
| branch | `codex/p5-f445h-i7-x-ingress-acceptance` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-x-ingress-acceptance` |
| integration owner | `/root/phase05_integration_steward` |

最终result commit/tree由Git handoff报告；result不能自引用自己的commit identity。

## 2. Independent positive receipt

新增的Router测试在一个active assembly中放入：

```text
skiff.run/codex-relay  GET /v1/models
skiff.run/aihub        GET /v1/models
```

两次请求复用同一个Router HTTP监听端口和同一个`Host: api.localhost`，只改变
`x-skiff-service`/`x-skiff-version`。测试逐个解码实际发送给Runtime的binary frame并断言：

- Relay header选择Relay deployment与Relay gateway identity；
- AIHub header选择AIHub deployment与AIHub gateway identity；
- 两个frame都使用相同service-local selector；
- `httpRequest.url`保留同一个Host作为业务metadata；
- 两个请求都得到各自Runtime响应。

这条receipt与assembly resolver、Host request和WebSocket完整owner suite共同证明，不同service的同形
入口合法，但精确deployment不会在Router、wire或Host处退化。

## 3. Negative matrix

| 要求 | 独立证据 |
| --- | --- |
| 旧Host selector/wire拒绝 | artifact-model严格serde、Router unary mutation与compiler旧Host authoring负例 |
| 旧generation拒绝 | deployment/assembly identity、Runtime frame v2、activation/shared-wire corpus |
| 同service重复method/path拒绝 | compiler service_config、deployment resolver、Router ingress index |
| 缺失/非法service/version header拒绝 | Router service selection与WebSocket admission |
| 未知service/version拒绝 | Router HTTP/WebSocket exact deployment lookup |
| 同service/version多revision拒绝 | artifact identity、assembly resolver、Router ingress index |
| cross-deployment frame substitution拒绝 | Runtime Host HTTP与WebSocket JSON-RPC exact routing tests |
| HTTP Host不参与选择 | 独立双service正例、Router Host metadata测试、Host URL metadata测试 |
| WebSocket跨service同path合法并固定精确deployment | Router/Host WebSocket gateway、generation lifecycle与JSON-RPC pin suites |

## 4. Real RED and bounded repair

冻结candidate的真实RED：

- `skiff-runtime-transport`：unit `89/95`，6个失败；replica integration `1/2`，1个失败；
- Router：`835/837`，2个失败，继续展开旧receipt后为`836/837`；
- compiler timeout generation断言：`3/4`；
- 所有失败均为测试仍期待RuntimeAssembly v2、frame v1、旧artifact identity或缺少frame
  `deployment`字段；current production严格拒绝旧代际本身是正确行为。

有界修复只涉及：

```text
compiler/tests/timeout_artifact_lowering.rs
cross-system-fixtures/package-service-ecosystem/runtime-wire.json
router/tests/compilerGeneratedManifestCompatibility.test.ts
router/tests/runtime-assembly-unary-dispatch.test.ts
runtime/transport/src/control_mapper.rs                     # cfg(test) only
runtime/transport/src/control_response_mapper.rs            # cfg(test) only
runtime/transport/src/request_mapper.rs                     # cfg(test) only
runtime/transport/tests/assembly_replica_registration.rs
```

另新增本task/result。没有production行为改动、兼容读取、dual path或fallback。

## 5. Final GREEN

### Complete Rust owner combination

```text
cargo test --locked --no-fail-fast \
  -p skiff-artifact-model \
  -p skiff-artifact-identity \
  -p skiff-deployment \
  -p skiff-runtime-loader \
  -p skiff-runtime-linker \
  -p skiff-runtime-transport \
  -p skiff-runtime-request \
  -p skiff-runtime-package-test \
  -p skiff-runtime-host \
  -p skiff-test-runner
```

结果：PASS，`1017`个可执行测试通过，`0`失败，`4`个明确ignored；另`2`个doc test通过。

| owner | 计数 |
| --- | --- |
| artifact-model | 180/180 |
| artifact-identity | 144/144 |
| deployment resolver/model | 61/61 |
| runtime loader | 19/19 |
| runtime linker | 58/58 |
| runtime transport | 97/97 |
| runtime request | 41/41 |
| runtime package-test | 8/8 |
| runtime Host | 339/339，另1个doc test |
| test-runner | 70/70，3 ignored |

### Compiler authoring/projection

```text
cargo test --locked -p skiff-compiler-input service_config
cargo test --locked -p skiff-compiler --test http_gateway_projection
cargo test --locked -p skiff-compiler --test websocket_ingress
cargo test --locked -p skiff-compiler --test timeout_artifact_lowering
```

结果：PASS，`20 + 11 + 10 + 4 = 45/45`。

### Router

```text
pnpm --dir router type-check
pnpm --dir router test
```

结果：typecheck PASS；完整可执行suite PASS，`59/59` test files、`838/838` tests。不是零匹配。

聚焦`host-ingress`、service selection、HTTP unary、WebSocket与WebSocket JSON-RPC五个文件的
首次组合为`52/52`；新增纵向正例后HTTP unary单文件为`22/22`并已进入上述`838/838`完整结果。

## 6. Non-ingress compiler diagnostics

一次扩大到整个`skiff-compiler` test target的诊断还观察到7个与本任务无关的既有失败target：

- actor dispatch fixture缺少hydrated schema index；
- stream boundary availability旧期望；
- root-path/runtime-slots/shared-lane数据库fixture未声明database state requirement；
- std import/native调用旧期望；
- stream SSE native调用旧期望。

合计16个失败测试，均不涉及service-scoped ingress、Host removal、Runtime frame v2或本任务写集。
X没有越界修改，也没有把它们计入本轮GREEN；需要由各自既有owner另行处理。

## 7. Scope and safety

- `git diff --check`：PASS；
- 改动文件反搜RuntimeAssembly v2/frame v1：0；
- production write：0；
- stable/live/network/Mongo/OAuth/browser：未访问；
- push：未执行。

```text
X_COMPLETE = YES
SERVICE_SCOPED_INGRESS_ACCEPTED = YES
PRODUCTION_BUGS = 0
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```
