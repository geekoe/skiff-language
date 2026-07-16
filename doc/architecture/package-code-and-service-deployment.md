# Package Code 与 Service Deployment 分层架构（草案）

状态：方向草案。本文描述目标架构及其成立条件，用于后续修改
`publication`、compiler pipeline、artifact、runtime activation 与部署模型；在这些
canonical 文档完成同步前，本文不表示当前实现已经具备这些行为。

本文关注长期边界，不规定最终 YAML 字段名、源码关键字、CLI 命令或迁移批次。

## 1. 背景

当前 Skiff 把 package 和 service 都当成带源码的 publication root：

- package 编译为可在宿主 runtime program 中直接链接的代码；
- service 自己拥有源码，并额外生成远程 operation、ingress、路由、配置和运行时 metadata；
- 两者共享 `PublicationAbiUnit`，但在 manifest、编译输入、projection、artifact 和
  activation 上仍是两条 publication 形态。

这个模型带来一个不自然之处：同一份业务实现若既希望作为宿主内 helper 使用，又希望
独立部署，就需要先决定它“是 package 还是 service”。代码身份、调用 ABI、部署身份和
运行时配置被过早绑在一起。

目标模型把当前 service 概念拆成两个正交部分：

1. **代码部分**：统一为 package，是唯一的用户代码编译单元；
2. **部署部分**：service deployment，只包含部署配置以及由 compiler 派生的边界适配，
   不包含独立的用户源码集合。

同一个 package 可以：

- 被其他代码作为 package 直接链接；
- 作为一个或多个 service deployment 的实现；
- 同时在不同部署中以不同 service 身份、配置和状态 owner 被实例化。

这不是要求任意 package 函数都能远程调用。package 的完整本地 API 与可部署的 service
operation surface 是两个不同层次。

## 2. 决策摘要

目标架构采用以下决策：

1. 用户源码只编译成 **Code Unit**；当前 `PackageUnit` 是其自然演进基础。
2. service deployment 引用一个 root Code Unit，并从其 public API 中选择 operation，生成
   **Service Deployment Unit**。它不再拥有用户 source files。
3. package 直接调用使用 **Local Code ABI**，可以依赖同一 heap、引用 identity 和原地
   mutation。
4. service operation 使用 **Boundary ABI**。它必须具有与物理位置无关的调用语义，只有
   满足 boundary projection 的 callable 才能成为 service operation。
5. service operation 的物理绑定可以是进程内或远程；两者必须保持相同的**语言层
   boundary semantics**。进程内绑定是 Boundary ABI 的优化实现，不是普通 package call，
   也不承诺复制远程部署的网络故障概率和进程隔离属性。
6. compiler 生成 caller adapter、provider dispatcher、callback/stream plan；不发布一个
   用户可见的 stub package。
7. effect 推导只要求 sound 的 may-effect，不要求理论上不可实现的精确语义分析。
8. recoverable value 是 boundary-passable value 之上的持久恢复约束，不决定一次即时调用
   能否本地或远程绑定。

用一张图表示：

```text
Package sources + package manifest + api.yml
                  │
                  ▼
              Code Unit
        ┌─────────┴──────────┐
        │                    │
        ▼                    ▼
 direct package link   service deployment projection
 Local Code ABI               │
                              ▼
                    Service Deployment Unit
                              │
                    ┌─────────┴─────────┐
                    ▼                   ▼
            in-process boundary   remote boundary
               binding               binding
```

## 3. 目标与非目标

### 3.1 目标

- package 成为唯一、可独立缓存和复用的用户代码编译单元。
- 同一份 package 实现可以作为本地代码复用，也可以被 service deployment 发布。
- package 内部和本地 public helper 保留 Skiff 当前的可变引用语义。
- service operation 在进程内和跨进程执行时具有一致的语言层 boundary semantics，并明确
  列出不承诺等价的运行属性。
- 部署链接阶段能明确判断每条调用边允许哪种 binding，并给出可追踪的失败原因。
- compiler、runtime 和 artifact 中只有一个 remote/boundary projection 规则 owner。
- service 的配置、状态、路由、revision 和协议身份不污染代码 artifact 身份。

### 3.2 非目标

- 不提供任意普通函数的透明 RPC。
- 不承诺 local-only package API 可以在不修改依赖种类的情况下变成 service call。
- 不通过“序列化一遍看看是否成功”定义 boundary ABI。
- 不要求 compiler 精确预测所有运行时控制流和副作用。
- 不承诺进程内和远程 binding 具有相同延迟、可用性、资源竞争或故障爆炸半径。
- 不在本文确定 `local`、`remote`、`value`、`isolated` 等最终源码或 YAML 拼写。
- 不在本文给出历史 artifact 或旧 manifest 的兼容方案。Skiff 尚未发布，落地时应直接
  收敛到新模型。

## 4. 三种不同的调用

整个方案成立的关键，是不把“都在一台机器上执行”误认为“语义相同”。目标架构明确
区分三种调用。

### 4.1 Package-internal call

同一个 Code Unit 内部的普通函数调用。它使用语言内部调用约定，可以共享当前 request
heap、mutable root 和具体实现信息。

### 4.2 Direct package call

consumer 通过 package dependency 调用另一个 Code Unit 的 public API。它使用 Local Code
ABI，可以传递 heap handle、本地 `any I` carrier、native handle 或其他只在同一 linked
program 中成立的值。

这种调用要求代码被组装到同一个 runtime program，不具有远程 projection 也完全合法。

### 4.3 Service boundary call

consumer 通过 service dependency 调用 service operation。它使用 Boundary ABI：

- 参数、返回、错误、stream、callback、timeout 和 cancel 都必须有明确 boundary plan；
- provider 的 service identity、配置和状态 owner 不能因为进程内优化而变成 caller；
- 进程内与跨进程实现可以使用不同物理 carrier，但不能改变语言层 boundary semantics。

因此：

```text
direct package call != in-process service boundary call
```

前者是代码链接；后者仍然是 service 调用，只是 transport 没有离开当前进程。

### 4.4 Boundary equivalence 的范围

本文所说的“相同 boundary semantics”只覆盖语言和 service contract 能够规范的部分：

- ordinary data 的 detached value/alias 语义；
- provider service identity、activation context、config/state owner；
- caller principal、授权上下文和 capability owner；
- operation dispatch、单次调用内的 ordering/reentrancy 规则；
- return、throw/error envelope、callback 和 stream contract；
- deadline 的计算、timeout 对 caller 的结果，以及 cancel 信号的传播；
- trace、effect attribution 和审计归属。

以下是 deployment/transport 的运行属性，不承诺两种 placement 的事件分布完全相同：

- 网络延迟、分区和 provider unavailable 的发生概率；
- provider process crash、OOM、native fault 对 caller process 的爆炸半径；
- CPU、memory、connection pool 等资源竞争；
- 非协作代码收到 cancel 后是否能被强制终止。

remote binding 可以额外产生 transport/provider-unavailable 类错误；in-process binding 不需要
人为模拟网络故障，但两者必须把可比较的失败映射到同一公开错误分类。timeout/cancel contract
描述 caller 何时结束等待和 cancel 如何传播，不自动承诺 provider 已被强制终止。

因此 Boundary ABI 之外还必须有 **Execution Contract** 与 **Placement Requirements**。它们
描述 service 的调度、并发、重入、取消强制等级、principal/authorization、resource quota、
trust boundary 与故障隔离要求。assembly 只有在目标 runtime 能满足这些要求时才能选择
in-process binding；否则必须选择 remote binding，若 remote 也不可用则 fail closed。

## 5. 两层 ABI，而不是一个物理 ABI

本地链接和远程链接不应被描述成“完全相同的 ABI”。它们共享的是 operation contract，
物理调用约定必然不同。

```rust
struct CodeCallableContract {
    source_signature: CanonicalSourceSignature,
    local_abi: LocalCodeAbi,
    effects: FunctionEffectSummary,
    link_requirements: LinkRequirements,
    boundary_projection: Option<BoundaryOperationContract>,
}

struct BoundaryOperationContract {
    operation_signature: CanonicalOperationSignature,
    parameter_plans: Vec<BoundaryValuePlan>,
    return_plan: BoundaryReturnPlan,
    error_plan: BoundaryErrorPlan,
    stream_plan: Option<BoundaryStreamPlan>,
    callback_plan: BoundaryCallbackPlan,
}

struct ServiceExecutionContract {
    scheduling: SchedulingContract,
    concurrency: ConcurrencyContract,
    reentrancy: ReentrancyContract,
    cancellation: CancellationContract,
    security_context: SecurityContextContract,
    resource_policy: ResourcePolicyContract,
}

struct PlacementRequirements {
    isolation: IsolationRequirement,
    trust: TrustRequirement,
    cancellation_enforcement: CancellationEnforcement,
    native_fault_containment: FaultContainmentRequirement,
}
```

这里的类型名只是职责草图。核心事实是：

- `local_abi` 对可链接 package callable 总是存在；
- `boundary_projection` 可以不存在；
- service deployment 只能选择存在 `boundary_projection` 的 callable；
- assembly 再从同一个 boundary contract 派生进程内或远程物理 plan。

### 5.1 Local Code ABI 可以携带的事实

Local Code ABI 可以依赖当前 linked program，例如：

- executable address 或 link target；
- request heap handle；
- mutable root identity；
- 本地 `any I` 的 concrete type、method table 和 payload；
- runtime-local native handle。

这些事实不需要伪装成 wire schema。

### 5.2 Boundary ABI 必须描述的事实

Boundary ABI 至少描述：

- operation identity 与 canonical signature；
- ordinary data 的 boundary materialization；
- callback capability 的 owner 和调用路由；
- stream channel、backpressure 和 cancel；
- error/throw envelope；
- timeout、trace 和 service effect metadata；
- provider protocol compatibility expectation。

它不是具体 JSON、WebSocket 或某个进程内函数指针格式；这些属于 transport/link plan。

## 6. Boundary value projection

“可序列化”不是这里最有用的总分类。compiler 应直接为每个 boundary value 生成语义计划。

### 6.1 Ordinary data

primitive、record、array、map 和其他普通 schema data 在 service boundary 上采用 detached
value 语义：callee 不获得 caller 的 mutable root identity。

远程 binding 通过 encode/decode materialize；进程内 boundary binding 也必须表现为隔离值。
compiler 可以通过不可变性、唯一所有权、逃逸分析或 copy-on-write 消除物理复制，但不能
让优化改变别名和 mutation 的可观察结果。

materialization 以完整 argument tuple 为一个 boundary graph，但不会保留任何 caller mutable
root。跨参数 alias 是否需要保留必须由 boundary plan 明确表达；初始模型对依赖 caller alias
identity 的 callable 拒绝 projection。当前 request heap 不允许 cycle，因此带 cycle 的 boundary
graph 继续 fail closed，而不是让进程内 binding 获得额外能力。

### 6.2 `any I`

`any I` 的本地与远程 carrier 本来就不同：

- Local Code ABI 可以使用 method table 和本地 payload；
- Boundary ABI 使用指向 owner 的 callback capability 和 operation projection；
- 进程内 boundary binding 可以把 callback route 优化为直接 dispatch，但它仍受 capability
  owner、lifetime 和 boundary operation contract 约束。

method table 本身不是远程表示，也不需要被编码到对端。

### 6.3 Native handle

native handle 若跨 service boundary，不在 receiver 侧重建原生对象。它与 `any I` 一样，必须
投影为 owner capability，由 receiver 回调 owner 支持的 operation。没有 callback adapter 的
native handle 不具有 boundary projection。

### 6.4 Stream

`Stream<T>` 使用 stream channel plan，而不是 ordinary value serialization。无论物理 binding
是否远程，都必须保留一次消费、顺序、backpressure、cancel 和 chunk boundary semantics。

### 6.5 Recoverable value

一次 request 内可通过 boundary 传递，不等于能在未来恢复后继续使用：

```text
boundary-passable
  + durable identity
  + compatible restore plan
  + future-valid owner/capability
  = recoverable
```

request-scope callback capability 可以用于即时 service 调用，但通常不可持久化。本文不把
recoverability 作为所有本地或远程调用的前置条件。

## 7. Mutable helper 与 remote projectability

下面的函数是合法的 package helper，也可以进入 package 的本地 public API：

```skiff
function mutate(input: User) -> void {
  input.name = "new"
}
```

它的语义依赖 caller 与 callee 共享同一个 mutable root：

```text
Local Code ABI: available
Boundary projection: unavailable
Reason: writes through parameter root `input`
```

这不是 source compile error。只有以下行为会失败：

- service deployment 尝试把 `mutate` 选为 operation；
- service dependency call 尝试解析到这个 local-only callable；
- assembly 尝试把包含该调用的代码边拆成 service boundary。

如果未来需要表达“函数收到一个隔离副本，并允许修改该副本”，语言可以增加描述参数
value/ownership semantics 的修饰。该修饰描述的是值语义，不应叫作 `remote function`，因为
物理部署仍由 deployment 决定。

## 8. Effect 分析的边界

### 8.1 不要求理论上的精确推导

对一般程序，静态判断某个 mutation 是否在任意可能执行中实际发生，会遇到停机问题。
平台拥有全部源码并不能消除这个计算理论限制。

因此 linker 不能依赖“精确 effect inference”。compiler 应计算 sound may-effect：

- 不允许 false negative：不能漏掉可能依赖共享 heap 的行为；
- 允许 false positive：保守地把少数实际安全实现判为 local-only；
- 作者可以通过更明确的 value/ownership contract 或显式 boundary wrapper 消除歧义。

### 8.2 可组合的 effect summary

现有 `Function Effect Metadata` 已经要求 read/write path、return provenance、external effect、
callback 和 stream facts。目标模型在此基础上产生 linkage facts：

```rust
struct LinkRequirements {
    requires_same_heap: bool,
    parameter_writes: Set<ParameterPath>,
    returned_aliases: Set<ParameterRoot>,
    escaped_aliases: Set<ParameterRoot>,
    unsupported_boundary_values: Vec<BoundaryFailure>,
}
```

推导必须是传递和保守的：

- 直接 assignment 产生 parameter/root write；
- 调用 helper 时按实参 provenance 代入 callee summary；
- 分支合并取 may-effect union；
- recursion/mutual recursion 使用有限 lattice 固定点；
- native 和无法查看实现的 callable 使用声明过的 contract，缺失时 fail closed。

linker 对已经生成的 `LinkRequirements` 做确定性判断。它不读取 AST，也不在部署阶段重新
解释函数体。

### 8.3 哪些变化属于 contract 变化

具体实现 effect 不必全部进入 service protocol identity，但以下变化必须进入可链接 contract：

- boundary projection 从 available 变为 unavailable；
- parameter/return 的 boundary value plan 改变；
- callback、stream、error 或 cancel contract 改变。

已经发布为 service operation 的 callable 若更新后不再满足原 boundary contract，新的 service
deployment projection 必须失败，不能静默降为 local-only。

## 9. Service deployment projection

service deployment 是配置驱动的 projection，而不是第二种源码 publication。

概念输入如下：

```rust
struct ServiceDeploymentInput {
    service_identity: ServiceIdentity,
    root_code: ResolvedCodeUnit,
    exported_operations: Vec<CodeOperationSelector>,
    service_dependencies: Vec<ServiceDependencyBinding>,
    ingress: IngressSpec,
    runtime: RuntimePolicy,
    config: ConfigBinding,
    state: StateOwnershipSpec,
    execution: ServiceExecutionContract,
    placement: PlacementRequirements,
}
```

deployment compiler 只消费 Code Unit 中已发布的 typed facts：public API graph、callable
contract、effect/link requirements、File IR refs 和 schema closure。它不能穿透 Code Unit 去
引用 private source declaration，也不能重新分析 AST。

输出的 Service Deployment Unit 包含：

- service operation table 与 protocol identity；
- operation 到 Code Unit executable target 的结构化 binding；
- boundary schema/value plans；
- service dependency locks；
- ingress、routing、timeout、config 和 state ownership metadata；
- scheduling、reentrancy、cancel enforcement、security/resource policy 和 placement requirements；
- compiler 生成的 caller/provider/ingress adapter plan。

它不复制 Code Unit 的用户 executable bodies。runtime assembly 通过 typed refs 组合 Code Unit
与 Service Deployment Unit。

deployment projection 可以生成 synthetic adapter IR 或可解释 adapter plan；“deployment 没有
代码”指没有第二份用户源码和业务实现，不禁止 compiler 为边界生成机器可执行适配器。

## 10. Link 与 assembly

目标 assembly linker 处理两种依赖边。

### 10.1 Code edge

package dependency 产生 code edge：

- provider Code Unit 必须进入同一个 linked program；
- consumer 可以使用 provider 的 Local Code ABI；
- local-only helper 合法；
- provider 在执行时使用当前宿主 request frame 和宿主 service context，除非某项 package
  capability 自己显式定义其他 owner。

### 10.2 Service edge

service dependency 产生 service edge：

- consumer 只看到 provider 的 Boundary ABI；
- deployment topology 决定使用 in-process boundary binding 或 remote boundary binding；
- 两种 binding 都保留 provider service identity、request frame、配置/state owner、timeout、
  cancel、principal/authorization、trace 和 error boundary；
- 不能因为两个 service 恰好在同一 runtime process 就改用 Local Code ABI。
- assembly 必须用目标 runtime capability 校验 provider 的 Execution Contract 与 Placement
  Requirements；要求 process/security/fault isolation 而 runtime 无法进程内提供时，只能 remote。

概念上的最终绑定为：

```rust
enum AssemblyCallBinding {
    DirectCode(DirectExecutablePlan),
    InProcessBoundary(InProcessBoundaryPlan),
    RemoteBoundary(RemoteBoundaryPlan),
}
```

### 10.3 选择发生在哪里

源码调用不通过字符串或运行时猜测选择 binding：

- package resolver root 产生 code edge；
- service resolver root 产生 service edge；
- service edge 的物理 placement 由 deployment assembly 决定；
- callsite 和 artifact 保留 structured operation/link requirement；
- topology 与 requirement 不兼容时，assembly fail closed。

这避免了一个容易混淆的全局 `package.localLink: true/false`。binding 是每条依赖边、每个
operation 与当前 deployment topology 的结果，不是 package artifact 的固有模式。

## 11. State、Config 与运行时上下文

代码/部署分离后，最容易被低估的不是 stub，而是 owner。

Code Unit 只能声明 config、DB、queue、actor、file/resource 或其他运行时能力需求；实际 owner
由它所在的执行语义决定：

- direct package call 在宿主 service context 中执行；
- service boundary call 在 provider service context 中执行，即使 provider 与 caller 进程内
  co-locate；
- 同一个 package 被部署成两个 service 时，两者拥有独立配置、状态 namespace、revision 和
  lifecycle。

因此 in-process boundary binding 不能只是跳到 provider function address。它至少需要建立
provider request frame/context view，并执行与远程 dispatch 相同的 boundary validation 和
lifecycle hooks。

Code Unit 与 File IR 可以作为 immutable artifact/cache 跨 deployment 共享，但每个 service
deployment revision 必须拥有独立的 activation overlay。service requirement binding、resolved
config、DB/state namespace、public instance/singleton、runtime-local mutable cache、quota、principal
policy、初始化和关闭 lifecycle 都属于 activation，不能因 Code Unit 相同而跨 service 共享。
in-process boundary dispatch 切换的是 provider activation，不只是 provider executable address。

远程路径上的 caller identity、principal 和 capability 权限校验不能在进程内路径省略。若某个
security policy 依赖 process/trust-domain 隔离，而 runtime 没有等价 sandbox，placement validation
必须禁止 in-process binding。

这条规则也是独立部署可行性的基础：代码身份可以复用，状态和运行身份不能被代码 artifact
隐式占有。

Code Unit 若要调用 service，也必须把所需 service contract/alias 作为自己的 typed compile
requirement 发布。宿主 deployment 可以提供具体 binding，但不能把一组未进入 Code Unit contract
的 service alias 隐式塞给 package。否则同一个 package 会随宿主而获得不同的名字解析和 call
graph，不再是独立编译单元。

这条 requirement 可以直接锁定一个 service，也可以在未来表达为由 deployment 满足的 contract
slot；两者都必须让 package compiler 在不读取宿主源码的情况下完成类型检查，并让 assembly
得到结构化 service edge。

## 12. Identity 与依赖的最小边界

本文不展开完整 ID schema，只规定两个必要事实：

- package/code identity 标识可复用代码与 Local Code ABI；
- service identity 标识部署、寻址、配置、状态和协议线。

二者必须是 tagged identity，不能因为 display string 相同而在 registry、artifact path 或 resolver
中混淆。

service deployment 显式引用实现它的 root package。consumer 若依赖 service，只锁 service
contract；它不需要同时声明实现 package。in-process assembly 可以在最终 deployment lock 中记录
解析到的 Code Unit/build，作为复现和审计事实。

## 13. Compiler 与 artifact 目标边界

长期 pipeline 应从“package/service 两种 source publication”收敛为两条职责不同的流水线：

```text
Code compilation
  PackageInput + package dependencies + service contract requirements
    -> SourceCompileModel
    -> LoweredCode
    -> CodeProjection
    -> Code Unit + File IR

Deployment projection
  ServiceDeploymentInput + Code Unit contracts
    -> Boundary/Ingress Projection
    -> Service Deployment Unit

Assembly
  Service Deployment Units + Code Units + topology
    -> linked program images + route/remote bindings
```

关键约束：

- source parsing、name/type resolution、effect/provenance inference 和 lowering 只发生在 Code
  compilation；
- package 的 service call 在 Code compilation 时解析到 typed service contract，并作为结构化
  service edge 写入 Code Unit；deployment 只负责满足/bind 该 requirement；
- deployment projection 不读取 AST 或 source text；
- boundary projection builder 只有一个 owner，不能分别为“service source”和“deployed package”
  实现两套；
- caller proxy、provider dispatcher、ingress adapter 都从同一 typed boundary contract 派生；
- canonical artifact DTO 与 runtime linked overlay 继续分离。

平台侧拥有全部源码使完整构建和诊断更容易，但不是跨阶段读取源码的理由。Code Unit 必须携带
足够的 ABI、effect、provenance 和 projection facts，才能保持独立编译单元成立。

## 14. 营地原则与现有架构收敛

这个方向会直接触碰当前 package/service publication 分叉。实现时不能新增第三条
“package-deployed-as-service”旁路，否则相同规则会在以下位置重复：

- package/service manifest validation；
- source compile policy；
- public operation/schema projection；
- `PackageUnit`/`ServiceUnit` artifact emission；
- operation identity 与 effect/config metadata；
- runtime artifact graph assembly 和 activation；
- compiler-generated ingress/provider adapters。

本次架构落地的前置清理应是把共同代码事实收敛到 Code Unit，把 deployment-only 事实收敛到
Service Deployment Unit。现有共享 `PublicationAbiUnit` 是可复用基础，但它的职责需要重新命名
和切分：source public graph/Code ABI 属于 Code Unit，service operation/protocol projection 属于
deployment。

与该方向直接冲突的 canonical 文档必须在实现前或同阶段更新，至少包括：

- `reference/publication.md`；
- `architecture/compiler-publication-pipeline.md`；
- `architecture/compiler-entity-and-identity.md`；
- `architecture/runtime-compiler-shared-artifact-types.md`；
- `architecture/release-registry.md`；
- config、DB 和 recoverable value 中依赖 service/package owner 的部分。

不应保留旧 service-source compile 作为兼容路径；那会永久保留两套事实来源，并使 service
deployment 能否从 package 派生无法成为结构性不变量。

## 15. 可行性与主要风险

### 15.1 为什么方向可行

Skiff 当前已经具备几块关键基础：

- package/service 已共享 public API graph 和 `PublicationAbiUnit` 的大部分 shape；
- compiler 已有 read/write path、return provenance、callback、stream 和 external effect
  metadata 的目标契约；
- artifact 已区分 canonical unit 与 runtime linked overlay；
- service call 已有 operation identity、protocol identity、dependency lock、router 和 runtime
  dispatch；
- `any I` 已明确区分本地 method-table carrier 与远程 operation carrier。

因此这不是要求发明一种全新执行模型，而是重新划分已有事实的 owner，并让 service
projection 消费 Code Unit contract。

### 15.2 真正困难的部分

主要风险按重要性排序：

1. **运行时 activation/owner 隔离**：进程内 service binding 必须保留 provider 的配置、DB、
   actor、queue、resource、principal、quota 和 lifecycle context，不能共享 mutable activation。
2. **Execution/placement contract**：必须区分语言 boundary equivalence 与 transport failure，
   并让 assembly 对调度、重入、取消、security 和 fault isolation fail closed。
3. **Boundary semantics**：ordinary mutable data、alias provenance、callback capability 和 stream
   必须在进程内/远程保持一致。
4. **Sound link requirements**：effect/provenance 分析必须保守，native/unknown contract 必须
   fail closed，同时为误判提供显式 value wrapper 出口。
5. **独立演进**：service protocol 与实现 Code Unit revision 必须解耦，部署/回滚不能把 code
   build identity 当作 service selector。
6. **性能预期**：进程内 boundary 不能默认退化成完整 wire encode/decode，但所有优化都必须受
   boundary semantics 和测试约束。

这些都是明确可建模的工程问题，没有发现理论上的阻断点。若 owner context 或 boundary
semantics 不先确定，单独实现 stub/link 开关会得到一个表面可用、长期不可维护的系统。

## 16. 这种模型是否少见

组成它的单个思想并不少见：

- 代码 artifact 与 deployment/release 配置分离是普遍做法；
- [WebAssembly Component Model](https://component-model.bytecodealliance.org/design/components.html)
  把代码组件、machine-readable interface、composition 和 Canonical ABI 分开；
- [Akka location transparency](https://doc.akka.io/libraries/akka-core/2.10.7/general/remoting.html)
  明确采用“先定义可分布语义，再把本地执行作为优化”，而不是把任意本地对象调用推广成
  远程调用；
- [Microsoft Orleans](https://learn.microsoft.com/en-us/dotnet/orleans/benefits) 使用逻辑
  identity 隐藏物理 placement，并且即使目标位于同一 silo，也为 grain call 保留消息边界；其
  [serialization contract](https://learn.microsoft.com/en-us/dotnet/orleans/host/configuration-guide/serialization-immutability)
  明确通过参数复制保护调用边界。

相对少见的是这些能力在一门语言中同时出现：

- 同一个用户代码 package 既提供共享 heap 的 Local Code ABI；
- 又能由纯配置投影成 service；
- compiler 为可投影的 operation 同时生成进程内与远程 boundary plan；
- linker 根据部署拓扑选择物理 binding。

这是一种不常见但有清晰先例支撑的组合。风险不来自“少见”本身，而来自是否错误地宣称
所有调用都 location-transparent。本文通过两类 surface 和三种 call binding 明确拒绝这种
宣称，因此方向上比传统透明分布式对象模型更稳健。

## 17. 被拒绝的替代方案

### 17.1 继续让 service 拥有独立源码

会继续把相同业务实现分裂成 package/service 两个 publication，并迫使 compiler 保留两条 source
projection。不能解决“一份代码，两种部署”的根问题。

### 17.2 所有 package public function 都强制 boundary-safe

会删除有价值的本地 helper、共享 mutation 和高效数据结构能力，并把普通 code composition
错误地降成 RPC 子集。

### 17.3 根据当前函数体精确猜 remote eligibility

理论上无法精确，且实现变化会让 public capability 隐式漂移。应使用 sound may-effect、显式
link contract 和 boundary projection validation。

### 17.4 发布独立 stub package

会引入额外版本、名字、缓存、依赖和一致性问题。stub 是某次 service contract 与 transport 的
生成适配器，应属于 assembly/artifact projection，不是用户代码 package。

### 17.5 进程内 service call 退化成 direct package call

会改变 mutable alias、service state owner、config、timeout、cancel、错误和观测语义，使 placement
成为业务可见行为。该优化不允许存在。

## 18. 架构验收不变量

后续实现与 canonical 文档至少应证明以下不变量：

1. 不存在带用户 source files 的 Service Deployment Unit。
2. 同一个 Code Unit 可以被多个 service deployment 引用，无需重复 source compile。
3. local-only mutable helper 可以正常进入 package local API，但不能被选为 service operation。
4. linker 对每个 unavailable boundary projection 提供结构化原因和完整 call/deployment path。
5. 同一个 service operation 在 in-process 与 remote binding 下通过同一组语言 boundary 语义
   测试，覆盖 alias、error taxonomy、deadline/cancel propagation、callback、stream、principal、
   config 和 activation/state owner；测试不把延迟和故障发生概率误列为等价项。
6. deployment projection 不访问 AST/source text，不重新推导 effect 或 schema。
7. caller adapter 与 provider dispatcher 来自同一 Boundary Operation Contract。
8. service protocol identity 不以某次 Code Unit build identity 作为路由 selector。
9. `any I`/native handle 的远程形态是 callback capability，不尝试传输 method table 或重建 native
   object。
10. recoverable validation 只在跨 request/持久边界启用，不成为普通 service call 的隐式条件。
11. 要求 process/security/native-fault isolation 的 service 在不具备等价 sandbox 的 runtime 上
    永远不会被 assembly 绑定为 in-process；不满足任何 placement 时 fail closed。
12. provider trap、非协作 cancel、callback reentrancy、resource exhaustion 和 process loss 都有
    明确的 contract 分类：要么由 runtime capability 满足，要么成为 placement constraint，要么
    明确属于不保证等价的运行属性。

满足这些不变量后，package 是独立代码单元、service 是配置驱动部署单元、同一边界 operation
可以按 topology 选择进程内或远程实现，三者才真正形成一个闭合模型。
