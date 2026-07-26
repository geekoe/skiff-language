# P5-F351 HTTP gateway artifact model / identity foundation

状态：Ready（C0 shared prerequisite；2026-07-26按WebSocket业务消息抽象修正scope）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`

只沿父文档引用读取权威设计和F347–F350事实。若审计建议与H36冻结语义不同，以H36及其最高权威设计为准；
不得重新打开authoring、compiler、deployment或runtime架构决策。

## 目标

在Rust artifact层建立HTTP后续consumer唯一复用的gateway external-protocol model与canonical identity
owner，并提供未来其它external protocol可以复用的protocol-neutral schema/key/identity叶子类型。完成后，
HTTP compiler/deployment/runtime/Router可以引用同一类型和golden，但本任务不接线任何consumer。

本任务不设计WebSocket业务消息入口。原计划中的raw `websocketReceive`与message source已被权威父文档
撤回；已有WIP若包含这些类型必须删除。WebSocket connect、message selector/envelope、message handler
surface与两层identity等到单独设计冻结后再扩展shared model。

必须形成：

1. 强类型`GatewayEntryKey`与`GatewayEntryIdentity`。Key是validated service-owner-local opaque key，
   identity是带generation prefix的content identity；两者不得互换或接受空白/非法值。
2. 一个新shared gateway artifact module，拥有：
   - closed `GatewayAdapterKind`：HTTP typed JSON、HTTP raw；
   - closed `GatewayAdapterSource`，精确覆盖权威gateway文档的三种HTTP source；
   - `GatewayAdapterArg`的`param + source`严格DTO，供后续execution-plan consumer复用；
   - normalized HTTP `GatewayEntryProtocolSurface`；
   - HTTP raw/typed、unary/server-stream mode；
   - 只表示真正external source/sink的entry-local schema view；
   - fixed closed external error projection/version。
3. Entry-local external schema是artifact/docs/diagnostics/identity表示，不是runtime codec：
   - 使用strict、closed Rust enum/struct表达当前JSON wire vocabulary；
   - 支持null/string/number/integer/boolean/bytes、array、record、closed union、nullable与string literal；
   - record field使用canonical ordered map；union/required等有确定顺序和重复校验；
   - 不提供`TypeRefIr`、`PackageSchemaTypeId`、package/public/source path、nominal/display symbol、
     handler target、codec plan或任意`serde_json::Value`逃生字段；
   - private named type只能留下结构，不可能通过DTO序列化其Skiff identity。
4. Protocol surface只保留wire兼容性事实：
   - entry/protocol/adapter kind、dispatch mode；
   - normalized external HTTP input/output schema；
   - 会改变外部wire的标准source选择；
   - fixed external error projection及显式external documentation metadata中确实影响wire的closed字段。
5. `GatewayAdapterArg.param`、完整adapter execution plan与protocol surface分离。目标参数重命名、handler替换、
   build变化、内部context/codec变化不能改变`GatewayEntryIdentity`；后续compiler负责从execution plan
   确定性投影normalized protocol surface并逐项校验。
6. `artifact-identity`是唯一identity owner：
   - 新schema marker与prefix；
   - canonical normalization/preimage；
   - SHA-256 framed identity；
   - validated identity parser；
   - golden和mutation matrix。

Rust具体文件拆分可遵循现有crate惯例，但不得把canonical hash实现复制到`artifact-model`、compiler、
deployment、runtime或Router。

## Identity inclusion / exclusion matrix

测试必须直接证明：

| 变化 | `GatewayEntryIdentity` |
| --- | --- |
| HTTP raw ↔ typed、unary ↔ server stream | 必须变化 |
| HTTP external request/response schema变化 | 必须变化 |
| HTTP external source语义变化，如raw request ↔ typed body | 必须变化 |
| fixed external error projection/version变化 | 必须变化 |
| map/list输入顺序的非语义变化 | 不得变化或必须被validation拒绝；不能静默产生两个identity |
| selector host/method/path变化 | 不得变化，且selector不能出现在preimage类型 |
| `GatewayEntryKey`变化 | 不得变化，且key不能出现在preimage类型 |
| handler/pre/guard selector或`PackageCallableId`变化 | 不得变化，且target不能出现在preimage类型 |
| PackageArtifact/build/deployment policy/revision变化 | 不得变化，且这些字段不能出现在preimage类型 |
| handler目标参数名或`GatewayAdapterArg.param`变化 | 不得变化 |
| private nominal name/id、context type/id或内部codec/execution plan变化 | 不得变化，且这些字段不能进入surface DTO |

如果某个excluded事实只能通过“类型中根本没有该字段”证明，测试应同时加compile-time construction fixture或
serialization golden，不能只在注释中声称。

## Validation

必须fail closed：

- 空白key/identity、错误prefix、非小写64位hex digest、unknown enum kind/unknown field；
- 非HTTP adapter/source，包括任何WebSocket connect/receive/message kind；
- typed HTTP没有external body/response schema，raw HTTP伪造typed body schema；
- server stream缺item schema或unary携带stream item；
- external schema包含重复union branch、重复/非法required field、open/unknown record field；
- external schema试图携带Skiff nominal/public/source identity；
- 非canonical collection order在loaded canonical artifact边界被拒绝，而builder/normalizer只产生canonical
  order。

不要为旧identity、旧gateway manifest或`ContractOperationId` ingress保留reader/fallback。
不要为了“以后也许能用”预留`serde(other)`、unknown protocol variant、WebSocket placeholder或
`serde_json::Value`扩展槽；WebSocket设计冻结后可以在Skiff尚未发布的前提下显式扩展并更新generation。

## 写入范围

允许修改：

- `artifact-model/src/compile_identity.rs`
- `artifact-model/src/gateway.rs`及其同目录tests
- `artifact-model/src/lib.rs`
- `artifact-model/src/tests.rs`（只用于module registration）
- `artifact-identity/src/gateway.rs`及其tests/golden
- `artifact-identity/src/constants.rs`
- `artifact-identity/src/lib.rs`
- `artifact-identity/src/tests/**`
- 为上述两个crate直接验证所必需的最小fixture/corpus

禁止修改：

- `compiler/**`
- `deployment/**`
- `runtime/**`
- `router/**`
- `test-runner/**`
- `service.yml`、`api.yml`、三仓库service或任何authoring parser
- 现有ServiceContract/PackageArtifact/Deployment/RuntimeAssembly schema generation与identity
- lockfile、稳定instance、live fixture

若现有共享type必须移动才能避免循环，只报告blocker，不扩大写入范围。

## 最小验证

先枚举selector并确认非零：

```bash
cargo test -p skiff-artifact-model gateway -- --list
cargo test -p skiff-artifact-identity gateway -- --list
```

再运行：

```bash
cargo test -p skiff-artifact-model gateway
cargo test -p skiff-artifact-identity gateway
cargo test -p skiff-artifact-model
cargo test -p skiff-artifact-identity
cargo fmt --all -- --check
```

还必须运行`git diff --check`并人工反搜本任务新增文件，确认没有
`ContractOperationId`、`ServiceProtocolIdentity`、selector、handler target、Package build或
`TypeRefIr`进入identity preimage，也没有`websocket`、`receive`、`ConnectionMessage`或context
expectation进入新增shared surface。

不运行workspace/root、stable/live，不安装或更新依赖，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f351-gateway-model`
- branch：`codex/p5-f351-gateway-model`
- 从包含本task的integration checkpoint创建。

提交production/tests，新增
`P5-F351-gateway-artifact-model-identity-result.md`记录exact base/commit/tree、最终DTO/identity
preimage、validation、selector数量、命令结果、合法残余与限制。不得修改本task状态、父文档或其它任务；
返回commit，不push。
