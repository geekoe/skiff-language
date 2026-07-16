# Package Code / Service Deployment 总体实现计划

状态：active，Phase 01 ready，其余阶段 outline-only
日期：2026-07-16

本文是 `doc/architecture/package-code-and-service-deployment.md` 的分阶段实施计划。它不尝试
一次写完全部任务；只冻结总体方向、阶段边界和当前可执行阶段。每个阶段验收后，下一阶段
才展开成任务 DAG，并允许依据实现事实调整后续阶段。

## 1. 最终结果

本轮实施完成后，Skiff 采用以下模型：

1. Package 是唯一用户源码编译单元，具体 artifact 继续使用 `PackageUnit`；不新增一套
   `CodeUnit` DTO 与它竞争。
2. Service 是无用户源码的部署单元，具体 artifact 继续使用 `ServiceUnit`，但其职责改为
   Service Deployment Unit：引用 root `PackageUnit`、选择 boundary operation，并拥有 ingress、
   config/state owner、timeout、routing 和 activation metadata。
3. `ServiceUnit` 不再拥有 File IR；`LinkedProgramImage` 不再区分 `service_files` 与
   `package_files`。所有可执行代码来自有序的 PackageUnit graph，root package 是显式 slot。
4. package dependency 使用 Local Code ABI；service dependency 使用 Boundary ABI。即使两个
   service 在同一 runtime 内，service call 也不能退化成共享 heap 的普通 package call。
5. 本轮所有 service edge 都组装为 `InProcessBoundary`。不存在运行时 remote fallback，也不
   允许缺少本地 provider 时把请求交给 router。
6. 一个 runtime replica 加载完整 Runtime Assembly；扩容复制整个 assembly。每个 replica 有
   独立 heap、CPU 调度和 activation，MongoDB、Redis 等外部存储按 deployment 配置共享。
7. 保留 transport-neutral boundary contract 与 dispatcher seam，未来可以增加
   `RemoteBoundary`，但本轮不实现远程 service-to-service transport。

## 2. 已冻结的实现选择

### 2.1 名称与 artifact owner

- 架构文档中的 Code Unit 落地为现有 `PackageUnit`，不做纯命名迁移。
- 架构文档中的 Service Deployment Unit 落地为职责收窄后的 `ServiceUnit`。
- `PublicationAbiUnit` 只描述 package publication 的 public code API；service protocol 从所选
  boundary operation contract 派生，不再把 service 当成第二种 source publication。
- PackageUnit 拥有 File IR refs、代码资源、package dependencies、service contract
  requirements、local callable contract 和可选 boundary projection。
- ServiceUnit 拥有 service identity/protocol、root package ref、operation selection/binding、
  dependency binding、ingress、activation requirements 和 deployment metadata，不复制用户
  executable body。

### 2.2 Service requirement

package source 使用与当前 service dependency 相同的 canonical declaration shape，在
`package.yml` 顶层声明 `services`：service id、精确 version 和 alias。编译时从可信 service
artifact root读取 `ServiceProtocolContract` view。

一个service contract由 `(serviceId, exact serviceVersion)` 标识，并包含具名operation map；每个
operation对应完整BoundaryOperationContract。service deployment从root PackageUnit的可用boundary
callable中显式选择并映射这组surface。`serviceProtocolIdentity`只hash该canonical surface，不包含
root package/build、deployment revision、route或config/state值。

service version是contract release version；deployment revision是实现/配置/路由revision。同一
service id/version的revision必须保持相同protocol identity。PackageUnit保存typed
`ServiceContractRequirement`：service id/version/protocol identity以及实际引用operation的typed
expectation；不保存provider package id，也不把某次provider build id当作调用寻址键。

后续 assembly 必须用同 assembly 内的 ServiceUnit 满足这些 requirement；id/version/protocol
不匹配、缺provider或revision选择后出现多个provider都fail closed。Phase 01只支持依赖已经发布
到可信artifact root的contract；初次发布的循环service contract不引入placeholder绕过。

### 2.3 Boundary 与 recoverable

即时 service call 的值计划与 recoverable value 是两层契约。这里采用用户提出的
“linkable 后再叠 future validity”分层：

```text
LinkableValuePlan<Lane>
  = 当前值可在指定 lane 中立即 materialize 的 carrier/encoding/lifetime 计划

RecoverableValuePlan<Lane>
  = LinkableValuePlan<Lane> + 值离开当前 request 后仍有效的 FutureValidityPlan
```

普通 service 参数不因为使用 Boundary ABI 就必须 recoverable。显式 recoverable slot、DB、
spawn、queue 等跨 request/persistent lane 才启用 recoverable 规则。request-scope `any I` 或
native handle 若能跨 service，只能投影成 callback capability，不能传 method table、native
对象或要求它可持久恢复。`Linkable` 在这里是 value boundary 术语，不等同于 code linker 能否
解析 executable symbol；它也不是脱离 lane 的全局布尔属性。同一值可以对即时 service-call
lane linkable，而对 DB/recoverable lane 不成立。

Phase 01的callback/stream ABI只冻结value/capability owner、operation surface、lifetime、失效、
item/error/cancel channel等可链接事实；callback重入调度、stream buffer/backpressure、强制取消
等级属于Phase 03的Execution Contract，不进入Phase 01 boundary identity，也不阻塞projector。

这一分层会在 Phase 01 的架构前置任务中同步到 canonical 文档；在文档完成前，不实现新的
boundary DTO。

### 2.4 Effect 与可部署性

- 每个 package public callable 始终有 Local Code ABI。
- 每个 public callable 还必须得到明确的 boundary projection 状态：`Available(contract)` 或
  `Unavailable(reasons)`，不能用字段缺失让 linker 猜原因。
- effect/link requirement 采用 sound may-analysis，允许保守拒绝，不允许漏掉共享 heap、参数
  写入、返回 alias、escape、未知 native/external effect。
- effect 分析消费 typed source/lowered facts，不解析 artifact JSON，也不在 linker 重读 AST。
- mutable helper 可以继续作为 package local API；部署选择它时以结构化原因失败。
- 当前阶段不新增local/remote语法标记；eligibility由effect + boundary projection推导，deployment
  选择/link时检查。未来若加入annotation，也只能作为compiler assertion，不能成为第二套规则。
- 当前阶段不新增local/remote语法标记；eligibility由effect + boundary projection推导，deployment
  选择/link时检查。未来若加入annotation，也只能作为compiler assertion，不能成为第二套规则。

### 2.5 迁移方式

- 在当前仓库原地替换受影响子系统，不新建平行仓库。
- 阶段内可以有尚未合并的临时桥接提交；每个阶段验收时不得留下**该阶段已经负责切换**的旧
  path、双DTO reader或fallback。具体删除点固定为：Phase 01保留旧service-source与remote
  transport，但它们只能消费共同typed contract；Phase 02删除service-source production path和
  service-owned code；Phase 03关闭所有production remote selection/fallback；Phase 04物理删除
  remote relay/protocol残留。
- Skiff 尚未发布，不兼容旧 manifest、artifact 或 CLI 形态。回滚单位是整个阶段 merge，
  不是长期 dual-read/dual-write。

## 3. 当前可复用资产与替换面

保留并演进：

- syntax/parser、name/type resolution、File IR 与大部分 lowering；
- PackageUnit、PublicationAbiUnit、package implementation links；
- boundary codec/type plan、operation identity、timeout/cancel/error/stream 基础；
- eval/interpreter、request heap、DB/native/recoverable runtime；
- `LinkedProgramImage` 与 `RuntimeActivation` 分层；
- RuntimeHost 同时持有多个 `ServiceRuntimeContext` 的能力；
- router ingress、runtime control connection 和 artifact distribution。

直接替换：

- `PublicationInput::Service` / `PublicationCompilePolicy::Service` 及 service source compile；
- ServiceUnit-owned File IR、service resources 和 `UnitAddr::Service`；
- package 禁止 service requirement 的输入规则；
- 当前空的 effect metadata 与 service/package 分散的 boundary projection；
- service-to-service 经 router relay 的生产路径；
- 把 service source root 当作 CLI/dev/release 输入的流程与测试。

## 4. 营地原则检查

本功能会直接放大的现有问题：

- package/service source policy 让同一 public API、schema、config 和 operation 规则分叉；
- service dependency parser/resolver 只服务于 service manifest，package 若复制会产生第二 owner；
- `ConfigAndEffectMetadata` 用 opaque metadata 承载 config，effect 实际只有 `Empty`；
- compiler projection handoff、package projection 和部分 lowering 文件已超过千行；
- ServiceUnit、ArtifactGraph、LinkedProgramImage 和 runtime resolver 都把 `service_files` 当作
  隐式 root code slot；
- runtime boundary encode/decode 与 router transport 绑定过紧，容易把“逻辑 boundary”误当成
  “一定远程”。

处理规则：

- Phase 01 先抽出 service requirement 单一输入 owner、typed effect/link analysis、boundary
  projection 单一 owner，并拆分本阶段必须修改的超长 projection handoff/package projection
  文件。
- 后续阶段在细化前重新做一次路径审计；发现重复或隐式契约时新增前置任务，而不是写进某个
  feature task 的附带工作。
- 不清理与当前数据流无关的 router、runtime 或 compiler 历史问题。

## 5. 阶段图

```text
Phase 01  Package code contract foundation
    │
    ▼
Phase 02  Config-only ServiceUnit cutover
    │
    ▼
Phase 03  Runtime Assembly + InProcessBoundary
    │
    ▼
Phase 04  Remove remote path + runtime/router hardening
    │
    ▼
Phase 05  Ecosystem migration + final acceptance
```

后续阶段不是不可调整的瀑布计划。每次阶段验收可以修改尚未细化的阶段数量、顺序和范围，
但不得改变 §1–§2 已冻结的最终模型，除非先更新架构文档并取得用户确认。

## 6. 阶段摘要

### Phase 01：Package code contract foundation

状态：`ready`，任务见 `phase-01-code-contract-foundation/phase-plan.md`。

产出：package 可以独立解析 typed service requirements；PackageUnit 带完整 local callable
contract、sound effect/link requirements 和显式 boundary projection 状态；package test/build 能
消费新 artifact。现有 service source production path 暂不切换，但本阶段新增规则只有一个 owner。

阶段验收不要求 runtime 本地执行 service dependency；只要求调用已经成为可链接的结构化 code
fact，供 Phase 02/03 消费。

### Phase 02：Config-only ServiceUnit cutover

状态：`outline-only`，见 `phase-02-service-deployment-cutover/phase-overview.md`。

产出：service.yml 只做 deployment projection；ServiceUnit 引用 root PackageUnit，不拥有 source
files；compiler、artifact writer、loader、linker、CLI/dev sync 同步切换；旧 service source compile
在阶段合并前删除。阶段结束时跨 service 调用仍可暂用现有 transport，以降低一次切换的维度。

### Phase 03：Runtime Assembly 与本地 Boundary

状态：`outline-only`，见 `phase-03-local-runtime-assembly/phase-overview.md`。

产出：assembly 闭合全部 service/package 依赖；每个 runtime replica 原子加载完整 assembly；
service call 通过 provider activation 执行 `InProcessBoundary`，覆盖 detached value、callback、
stream、timeout/cancel、error 和 principal/context owner。

### Phase 04：删除远程路径与系统加固

状态：`outline-only`，见
`phase-04-remote-path-removal-and-hardening/phase-overview.md`。

产出：删除 router service relay、runtime remote fallback 和废弃 protocol；保留 transport-neutral
dispatcher seam；补齐 assembly readiness、原子 reload、drain、内存 admission、health/telemetry
和多 replica 失败语义。

### Phase 05：生态迁移与最终验收

状态：`outline-only`，见
`phase-05-ecosystem-migration-and-final-acceptance/phase-overview.md`。

产出：迁移仓库 fixture、官方 packages 和实际 service；同步 canonical 文档；运行完整非 live
验证、必要 live/smoke，以及多个 runtime replica 共享外部存储的系统验收。跨仓库改动分别提交，
未经用户要求不 push。

## 7. 阶段执行协议

1. 细化当前阶段的 DAG、任务文件、测试 disposition 和阶段 gate。
2. 文档做一次独立完整评审；只修 blocking issue。
3. 为阶段建立 integration worktree/branch，并启动贯穿该阶段的单一集成协调 Agent。
4. 每个 ready DAG 节点由一个 Agent 在独立 worktree 实现并提交。
5. 集成协调 Agent在每一批后合并、运行最小check并发布下批唯一base commit；最终再运行阶段
   gate。下游Agent不得自行拼接多个前置分支。
6. 独立验收 Agent 只读当前生产路径、任务文档和测试结果；不复用开发 Agent。
7. 验收通过后合并 `main`、清理 worktree/branch，再细化下一阶段。

实现期间发现方向性问题时：

- 可由当前架构唯一推导：升级为独立前置任务并更新 DAG；
- 影响后续阶段但不影响当前阶段：记录到对应 phase overview，当前阶段继续；
- 无法唯一裁决或会改变 §1–§2：停止并询问用户。

## 8. 测试与验证策略

### 8.1 三层测试

```text
任务级：只跑直接受影响 crate/test/filter
阶段级：跑当前阶段涉及的 subject selector + 架构 gate
最终级：Phase 05 才跑 pnpm test / pnpm verify 和必要 live smoke
```

任务文件必须列出聚焦命令。Agent 可以增加一个直接相关测试命令，但不能把“跑全量”当作
不理解影响面的替代品。

### 8.2 旧测试处理

每个阶段维护 test disposition：

- `keep`：语义不受 ownership 变化影响；
- `rewrite`：不变量保留，但 fixture/artifact shape 改变；
- `delete`：只验证已删除的 service-source、service_files 或 remote relay 模型；
- `add`：新模型以前没有覆盖。

删除测试时必须在提交说明中指出其旧语义，并给出 replacement test 或说明该行为已被整体
删除。无需维持测试数量，也不允许机械替换字段名让旧结构测试“继续通过”。

### 8.3 评审与验收的节奏

- 每个任务不单独配一轮文档评审和一轮独立验收；由开发 Agent 自测，阶段统一验收。
- 高风险节点可以在任务文件中要求 paired read-only review，但必须有明确原因。
- 文档 reviewer 只把架构矛盾、DAG 不可执行、阶段无法形成 coherent state、缺少关键验收
  证据列为 blocking。
- 完美命名、未来 remote 细节、非关键错误文案和更多测试想法属于 non-blocking。
- 默认一轮；修过 blocking 后最多再做一轮。仍无法裁决则问用户。

## 9. 阶段提交与回滚

- 每个实现/集成任务一个提交，阶段集成保留任务提交边界；只读验收 Agent 不修改 worktree。
- 阶段验收通过后合并到 `main`，阶段 merge 是最小回滚单位。
- Phase 02 及以后发生 schema/CLI breaking change 时，不提供旧格式兼容；若阶段失败，回滚
  整个阶段 merge。
- 跨 `skiff`、`skiff-packages`、`internals` 的 Phase 05 改动分别提交；不记录跨仓库 commit
  pointer，不自动 push。

## 10. 总体验收不变量

完成 Phase 05 时必须同时满足：

1. 用户源码只由 package compiler 编译；仓库中没有 service-source compile production path。
2. ServiceUnit 不含 File IR refs 或用户代码 resources，只引用 root PackageUnit 与代码闭包。
3. PackageUnit 对每个 public callable 给出 local contract 和显式 boundary projection 状态。
4. mutable helper 本地可调用，但不能被 service deployment 选择。
5. package service requirement 在独立编译时完成 type/operation resolution，并在 assembly 结构化
   绑定。
6. runtime image 没有 `service_files`/`UnitAddr::Service`；root code 是显式 package slot。
7. 每个 service deployment 有独立 activation/config/state/principal owner，即使共享 PackageUnit。
8. 当前生产路径的 service call 全部走 `InProcessBoundary`，缺 provider fail closed，不经 router。
9. 多 runtime replica 分别加载完整 assembly；任一 replica 退出不破坏其它 replica，持久存储按
   配置共享。
10. router 只负责 ingress/control/distribution，不包含当前生产可达的 service relay。
11. Linkable Value 是即时 boundary 契约；Recoverable Value 在其上增加 Future Validity，普通
    即时调用不被强制 recoverable。
12. 直接触碰的重复规则已经收敛，新增核心逻辑不继续堆进已知超长文件。
