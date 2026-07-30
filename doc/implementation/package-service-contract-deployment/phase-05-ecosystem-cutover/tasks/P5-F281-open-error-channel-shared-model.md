# P5-F281 Open error channel shared model checkpoint

状态：Ready for contract review。

## 直接父节点与权威链

- 直接父结果：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`
- 父结果继续引用：
  `P5-F279-open-service-error-channel-design-result.md`
- F279引用唯一架构与用户可见语言事实源。

启动时只读本任务；需要依据时沿父链向上读取。任务中的Rust命名与文件范围只是实现合同，不覆盖父文档语义。

## DAG位置、基线与共享状态

- 节点：F280 `W1-S` shared strict model/schema checkpoint。
- production base：Skiff integration commit `a07652da`；已包含F278 same-heap修正与F280审计结果。
- 当前成熟度：允许下游暂时不能编译的严格schema切换点，不是稳定候选。
- 直接前置：F279设计冻结、F280 owner审计、F278已合入。
- 完成后解除：language/std/lowering、artifact/contract、open-channel effects与runtime channel consumer扇出。
- 后续consumer开始前必须先由独立A1验收冻结本节点DTO；consumer不得自行改变本节点shape。

合同可执行性调整：F280曾把`artifact-identity/src/constants.rs`列入W1。删除model字段后该crate的其余
preimage/validation consumer必然同时需要迁移，单独修改常量无法在本节点验证。identity marker/prefix及其
mutation tests改由后续唯一artifact/contract consumer在同一提交中拥有。本调整只改变调度owner，不改变
任何schema/identity必须整体bump的要求。

## 唯一production写入范围

- `artifact-model/src/types.rs`
- `artifact-model/src/executable.rs`
- `artifact-model/src/package_artifact.rs`
- `artifact-model/src/boundary/operation.rs`
- `artifact-model/src/websocket_ingress.rs`，仅删除`BoundaryErrorContract` import与已失效的
  “WebSocket operation must not declare throws”检查
- `artifact-model/src/schema.rs`
- `artifact-model/src/lib.rs`
- `runtime/model/src/error.rs`
- `runtime/model/src/type_plan.rs`
- `runtime/model/src/value.rs`
- `runtime/model/src/lib.rs`
- `runtime/model/src/`下至多一个新的canonical service-error/exception identity module

允许机械更新`artifact-model/src/**`和`runtime/model/src/**`中的co-located test/fixture constructor，使这
两个crate独立编译并验证新strict shape；不得借机械修复改变其它production模块职责。

禁止修改syntax/compiler consumer、artifact-identity、deployment、runtime其它crate、router、telemetry、
std/prelude、scripts、VSC、skiff-packages、internals及F278语义。

## 必须冻结的canonical shape

具体Rust enum/field名称由本节点选择，但只能有一个owner，并满足以下可观察要求。

### 1. File IR declaration与actual catch identity输入

- File IR必须显式区分：
  - nominal record；
  - nominal representation；
  - named union；
  - transparent alias；
  - interface。
- representation不再与transparent alias共用无法区分的`Alias`形状；interface不再冒充空record。
- named union必须能表达并严格保存：
  - enclosing named-union identity/context；
  - concrete nominal branch；
  - anonymous discriminator synthetic branch；
  - literal branch；
  - branch在完全实例化type arguments下的稳定identity输入。
- 两个结构、tag或literal相同但owner named union不同的branch，模型上必须可表达为不同identity。
- anonymous union不创建新的名义identity，但执行时必须能保留实际branch identity。

### 2. Instruction source site

- source-authored statement/expression `throw`与所有source-authored `CallIr`必须携带required、可进入File IR
  identity的source site。
- runtime/compiler生成的synthetic call/throw site使用显式synthetic variant及有限stable reason，不伪造
  source path，也不使用optional缺失代表synthetic。
- `rethrow`不创建新throw site；其IR仍引用既有exception。
- `ExprIr::Catch.catch_type`改为required；删除`None` wire形状及其隐式catch-all可能性。

### 3. 删除closed throw-set DTO

- 删除`PackageCallableSignature.throw_types`。
- 删除`BoundaryErrorContract`。
- 删除`BoundaryOperationContract.errors`。
- 新model拒绝旧`throwTypes`/`errors`与optional/missing catch type；不加default、alias、legacy variant、
  dual field或兼容reader。
- 保留`StmtIr::Throw`、`ExprIr::Throw`与`TestEffectOutcomeIr::Throw`的`payload_type`；它是actual identity
  输入，不是operation throw set。
- 保留throw provenance、`throws_caller_alias`、`detached_error`等不属于本节点DTO删除面的事实。

### 4. Runtime value、catch identity与opaque cause

- runtime model必须能让名义identity随实际值存活，而不是只存在于static `TypeRefIr`或codec plan：
  record、representation、primitive-backed representation、named/anonymous union中的实际branch以及普通
  assignment/container/call传递都必须有一个可由后续runtime实现消费的canonical carrier。
- identity carrier必须区分local execution identity、Package schema identity、platform builtin identity与
  named-union context/branch；不能用display string或shape作为identity。
- request-local exception model必须能表达：
  - 已materialize local value及actual catch identity；
  - 未链接、不可被local catch匹配但可原样转发的opaque service error envelope；
  - 当前request自己的source/stack/correlation。
- 本节点只冻结model，不在`runtime/model`实现heap、catch、codec、telemetry或service dispatch。

### 5. 唯一fixed service error DTO

在`runtime/model`定义唯一strict serde owner：

```text
ServiceErrorEnvelope
  = PublicTypedError {
      packageId,
      stableSchemaKey,
      packageSchemaTypeId,
      encodedPayload,
      traceId,
      errorId
    }
  | InternalError {
      payload: {
        message,
        traceId,
        errorId
      }
    }
  | PlatformError {
      builtinErrorIdentity,
      encodedPayload,
      traceId,
      errorId
    }
```

- `InternalError`的message字段存在但具体固定文案由runtime consumer产生；DTO不能携带原私有type/display或
  arbitrary details。
- opaque forwarding可保存同一envelope和encoded bytes，不要求中间service decode/re-encode。
- unknown variant、extra/missing field、无效payload owner或相关identity字段缺失必须strict reject。
- generic runtime诊断`RuntimeErrorPayload { code, message, status, details }`暂时保留给control/internal路径；
  不把所有字符串code `"InternalError"`全局改成用户可catch的`std.service.InternalError`。

### 6. Schema版本

在`artifact-model/src/schema.rs`一次bump以下strict schema版本：

- File IR schema与format；
- PackageArtifact；
- ServiceContractDefinition；
- ServiceContract。

本节点不修改artifact identity marker/prefix，也不宣称新Package Local ABI、build或protocol identity已经可
生成；后续artifact/contract consumer必须在适配删除字段的同时同步bump并验证那些identity domain。

## 完成标准与风险探针

1. 新model能round-trip上述declaration/branch/source/catch/error-envelope正例。
2. strict serde负例至少拒绝：
   - 旧`throwTypes`与operation `errors`；
   - missing/`null` catch type；
   - source-authored throw/call缺site；
   - synthetic site伪造source字段或未知reason；
   - unknown envelope variant、extra field、identity/payload缺失。
3. runtime value/catch identity单测证明：
   - 同shape不同nominal identity不相等；
   - representation保留外层identity；
   - 同branch shape但不同named-union context不相等；
   - opaque envelope没有local catch identity但可bit-equivalent round-trip。
4. schema constant测试断言只接受新version；不得为旧version保留reader。
5. 反向搜索确认model定义中不存在`throw_types`、`BoundaryErrorContract`、
   `BoundaryOperationContract.errors`或optional catch type。
6. 结果必须列出因strict切换而仍不能编译的下游consumer类别，作为扇出输入；不得越界修复它们。

本任务唯一拥有：

```bash
cargo test -p skiff-artifact-model --lib --no-fail-fast
cargo test -p skiff-runtime-model --lib --no-fail-fast
cargo fmt --check
git diff --check
```

若crate名称或test selector不真实，先用`cargo metadata`/`--list`确认等价的最小真实命令并记录。不得运行
workspace check、compiler/runtime完整测试、`pnpm verify`、instance、live或生态发布。

## 风险、非目标与提交边界

- 风险：最高；验收组`A1 strict-model`。
- 不实现source `CatchLeaves`、lowering、linker index、boundary codec、`InternalError` std source、
  exception stack、telemetry或transport。
- 不保留旧artifact/wire兼容；不为暂时通过下游编译增加default或dual shape。
- 不重新设计F278 same-heap或删除throw provenance。
- worktree：`/Users/geek/workspace/skiff-p5-f281-error-model`
- branch：`codex/p5-f281-error-model`
- 不push，不操作stable，不回滚其它worktree修改。
- 从启动到第一次production修改不超过5分钟；合同不可执行时立即返回`TASK_NOT_EXECUTABLE`、精确缺口与最小
  前置，不继续研究。
- 完成后提交一次或少量有序commits，返回commit、下游break清单、自验收矩阵与是否出现设计缺口；不得自行
  承接下游consumer。
