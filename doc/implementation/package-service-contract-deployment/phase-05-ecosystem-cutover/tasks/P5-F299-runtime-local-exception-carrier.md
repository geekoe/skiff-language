# P5-F299 Runtime local value and exception carrier

状态：Implemented checkpoint with upstream gaps。结果见
`P5-F299-runtime-local-exception-carrier-implementation-result.md`。

第一次执行发现required instruction site与required catch type在linked IR转换时被丢弃；精确事实记录在
`P5-F299-runtime-local-exception-carrier-result.md`。前置
`P5-F300-linked-exception-sites-result.md`已经完成。必须从包含F300集成提交
`91e0e48f3d6458b9e2967f12d9bf82a83f01a81b`的新HEAD创建全新开发Agent恢复本任务，不复用第一次会话。

## 直接父节点与权威链

- linked/type-plan结果：
  `P5-F297-applied-nominal-linked-consumer-result.md`
- shared runtime error/value model：
  `P5-F284-open-error-model-acceptance-result.md`
- runtime owner审计：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`

父链继续引用唯一权威runtime/error设计。启动时只读本任务；需要依据时沿父链向上读取。

## DAG位置与并行边界

- 节点：F293 S4与F280 W2-R中的request-local value/exception carrier。
- 与F296 compiler、F298 loader/index并行。
- 本任务只完成local runtime semantics；不实现service envelope export/import、InternalError转换、
  router/transport或telemetry sink。
- 完成后与F298共同解除canonical service error channel orchestrator。
- 当前是实现检查点，不是稳定候选。

## Production范围

- `runtime/model/**`
- `runtime/eval/**`

仅为model carrier在eval边界正确转换确有必要时，允许：

- `runtime/boundary/**`中纯RuntimeValue/plan adapter，不得实现service error envelope分类

允许co-located tests/fixtures。禁止修改loader/linked-program/linker/linked-type-plan、request/host/
transport/capability-context、artifact/compiler/router/std/生态仓库或权威文档。

## 完成标准

### 1. RuntimeValueCarrier成为identity handoff单位

- slot、heap node/object field、array/map element、assignment、construct、local call argument/return、
  stream item及throw payload保存/传递`RuntimeValueCarrier`或等价不丢identity的typed carrier；
- ordinary move/clone保持catch identity；field/container projection按设计显式选择内层carrier；
- 不允许helper调用`into_value`后静默丢identity，再从static type/shape重建；
- native/boundary materialization返回值以exact linked type plan创建新carrier；
- `Box<string>`与`Box<number>`同declaration不同arguments获得不同
  `LocalExecutionTypeIdentity`，nested argument使用typed canonical identity，不用display string；
- record、representation、named-union三类branch、transparent alias与anonymous unionactual branch
  正确产生/传播identity。

### 2. Request-local exception

- initial throw使用actual carrier identity/value，不先`runtime_to_wire`；
- 每次throw创建request-local`RequestException`，包含：
  - local cause carrier；
  - required throw instruction site；
  - 当前local call stack；
  - request trace/error correlation；
- catch leaves来自fully-instantiated linked type plan，按exact `CatchIdentity`匹配；
- catch成功返回同一local value/Exception控制流，不做JSON encode/decode；
- catch不匹配继续传播同一exception；
- rethrow要求现有Exception并原样保留cause、source、stack、trace/error id，不创建site或新id；
-同shape不同nominal、同address不同args、同branch不同enclosing union不得匹配。

### 3. Control-flow一致性

- ordinary、async、stream、concurrent lane、timeout/cancel、actor/local package call及test-effect local
  throw均保存同一exception carrier；
-已有concurrent确定错误选择规则不变；
- source/call stack使用F286/F297 required sites；不能以null/empty或diagnostic string代替；
- platform catchable errors进入用户request后获得typed platform catch identity与local stack；
- ingress decode在进入operation前失败仍不是业务catchable；
- private/nonclosed/capability字段nominal本地throw/catch/rethrow不要求任何serialization。

### 4. 旧路径删除

- 删除initial throw/local rethrow的generic wire encode/decode；
- 删除old `TypeIdentity`/address-only/shape/display catch reconstruction与optional catch-all；
- generic runtime diagnostic `RuntimeErrorPayload`保持独立，不冒充用户error value；
- 不实现service response envelope；remote/opaque cause只保留A1 model形状供下游。

## 最小测试与验证owner

至少覆盖：

- private non-SchemaClosed nominal local throw/catch/rethrow且boundary encoder未调用；
- record、primitive-backed representation、alias、anonymous union；
- generic record/representation/named union三类branch；
- same-shape different nominal、same address different args、different enclosing union catch miss；
- construct→slot→field→array/map→local call return→throw identity保持；
- initial source/stack非空、nested call frames、rethrow完全相同；
- catch miss、platform error、async/stream/concurrent获胜错误；
- malformed/missing required runtime identity fail closed。

唯一owner：

```bash
cargo test -p skiff-runtime-model --lib -- --list
cargo test -p skiff-runtime-model --lib --no-fail-fast
cargo test -p skiff-runtime-eval --lib -- --list
cargo test -p skiff-runtime-eval --lib --no-fail-fast
cargo test -p skiff-runtime-boundary --lib -- --list
cargo test -p skiff-runtime-boundary --lib --no-fail-fast
git diff --check
```

先确认selector非零。若loader/host等旧consumer遮挡，运行最窄owner target并记录精确首错；
不得越界修复。不得运行workspace、生态、stable、live或chat smoke。

## 风险与交付

- 风险：最高；后续必须进入`A5-runtime-channel`独立验收。
- worktree：`/Users/geek/workspace/skiff-p5-f299-runtime-local-carrier`
- branch：`codex/p5-f299-runtime-local-carrier`
- 不push、不操作stable。
- 启动到第一次production修改不超过5分钟；不可执行时立即返回
  `TASK_NOT_EXECUTABLE`、精确缺口与最小前置。
- 提交后返回commit、carrier/exception路径、自验收、旧路径反搜与所有遮挡；
  不自行承接service channel/wire。
