# Runtime Lazy-Load Deployment Model

## 本文负责 / 不负责

本文定义部署与版本解析的长期目标模型：**不可变产物 + 小指针表 + 注册目录 + runtime 懒加载**。
它取代当前 activation 协调层（Mongo `activation_state` 仓库、coordinator 的 prepare/commit CAS、
epoch store、generation lease、durable pending 与 reconcile）作为部署语义的权威。

本文不定义语言语法、YAML 字段、artifact DTO 细节或 CLI 拼写。Package / ServiceContract /
ServiceDeployment / RuntimeAssembly 的对象语义仍以
[`package-service-contract-deployment.md`](package-service-contract-deployment.md) 为最高权威；
registry 生命周期与 typed pointer 的机械细节以
[`release-registry.md`](release-registry.md) 为权威。本文只负责"版本如何解析到 buildId、
runtime 如何获得可执行内容、部署切换为什么不需要协调"。

当前实现仍运行 activation 协调层；本文是迁移目标，迁移步骤见"与既有文档的关系"与"迁移路径"。

## 四条不变式

1. **产物只增不换**。内容寻址的 immutable record（file-ir、package artifact、service contract、
   service deployment、runtime assembly）一旦写入永不变更；相同内容的重构建产出相同 identity。
2. **唯一可变状态是一张小指针表**。`(profile, serviceId, version) → buildId`，单键原子更新；
   没有"当前 assembly"概念，多个版本可以同时存在。
3. **runtime 无协调状态**。runtime 以 buildId 为唯一加载单位：内存中有就直接执行，没有就从
   配置的 artifact 目录懒加载；加载不到就是错误。runtime 不感知版本、不感知"当前"、不参与任何
   跨 runtime 协议。
4. **路由 fail closed**。请求携带人类坐标 `(serviceId, version)`；router 解析指针得到 buildId，
   只派发给"已加载该 buildId 或具备懒加载能力"的 runtime；解析不到、无候选或加载失败一律快速失败。

## 概念与身份

- **人类坐标**：`serviceId + version`。version 来自 `package.yml` 的 version 字段。前端/调用方
  携带 version，不携带 buildId。
- **buildId**：内容哈希（`skiff-package-build-*` / deployment artifact identity）。是 runtime
  唯一的加载单位，也是指针表的取值。
- **指针表**：`(profile, serviceId, version) → buildId`。复用 typed pointer store 的原子写机制
  （rename + lock），单键更新天然原子。rollback = 把键指回旧 buildId。
- **deployment 记录**：服务的一次发布产物，烘焙该次发布的服务配置（config 属于产物，不属于
  独立提交的 snapshot），指向其 package 闭包与 file-ir。

## Runtime 懒加载

runtime 收到一个待执行的 buildId：

1. 内存中已有该 buildId 的已加载 image → 直接执行。
2. 没有 → 进入该 buildId 的临界区（per-buildId 锁）：同 buildId 的并发请求在锁外等待；
   持锁者按内容寻址从配置的 artifact 目录读取 deployment 记录、package 闭包与 file-ir 并
   构建可执行 image。
3. 加载成功 → 注册进"已加载集合"，放行等待的请求。
4. 加载失败或超时（记录缺失、目录不可达、超时阈值）→ 等待的请求快速失败。

已加载集合**只增不删**，与内容寻址同一哲学。长期运行的 runtime 内存会累积 buildId；逐出策略
（如 LRU）是 runtime 本地策略，逐出后回到懒加载路径，语义不变，不引入任何协调。

runtime 的注册与能力通告：

- 注册内容 = 已加载 buildId 集合；
- 能力通告（复用 `runtime.capabilities` 帧）携带 artifact root 与 lazy-load 能力标记，
  供 router 判定"可加载但尚未加载"的候选。

## Router 派发

请求解析：`(serviceId, version)` → 指针表 → buildId。

候选集：已注册该 buildId 的 runtime ∪ 具备懒加载能力且共享同一 artifact store 的 runtime。
无任何候选 → fail closed（`no eligible runtime`）。派发后 runtime 按懒加载语义执行，
加载不到即错误。

新版本上线路径：发布新 buildId → 更新指针（单键原子）→ 后续请求自然解析到新 buildId。
窗口期语义：新 buildId 尚未被任何 runtime 加载时，请求快速失败；可选地，router 可向能力者
发送 fire-and-forget 的预载提示（无 ack、无 pending、失败就重试）来收敛窗口，但不构成协议。

## 发布、版本与回滚

- **新增版本**（version 变化）：新 buildId + 新指针键，旧键不动，新旧版本并行存在。
- **同版本覆盖**（version 相同，唯一替换路径）：新 buildId + 单键指针更新。旧 buildId 的
  image 在已加载集合中保留；在途请求不受影响；新请求走新 buildId。
- **回滚**：指针指回旧 buildId。旧 buildId 大概率仍在 runtime 内存中，瞬时可用；不在则懒加载。
- **assembly 的重新定位**：assembly 不再是运行时切换单位，而是**发布/验证的快照 bundle**
  （build-once-deploy-many 的引用单位，供 verify 与 release promotion 使用）。

## 多 runtime

多个 runtime 实例（共享或各自持有 artifact store）是可互换的工人：各自独立懒加载、互不等待、
互不通知，零协调成本。新版本通过"收敛"而非"切换"扩散到全部实例。单 runtime 与多 runtime
没有任何语义差异——多 runtime 只是同一模型的水平复制。

## 配置归属

服务配置（API key、provider 端点等）在发布时烘焙进 deployment 记录，随 buildId 懒加载。
不存在独立于产物的 config snapshot 提交；没有第二个可变状态。

## 被移除的机制

| 机制 | 原职责 | 为什么不再需要 |
| --- | --- | --- |
| `skiff-router.activation_state` 仓库 | committed 世代持久化 + pending | 权威改为指针表；无 pending 概念 |
| coordinator prepare/commit/abort CAS | 两阶段切换 + 乐观锁 | 无"切换"；指针更新单键原子 |
| durable pending / reconcile / 启动恢复 | 崩溃一致性 | 无跨步骤事务 |
| epoch store + generation lease | 一致视图 + 请求钉住 | 请求在 admission 时绑定 runtime 会话；版本并行存在 |
| config snapshot 提交 | 随切换提交配置 | config 烘焙进 deployment 记录 |
| runtime 全量 assembly admission | 提交前预载候选 | 懒加载按 buildId 按需加载 |
| activation 504 语义 | 超时可能异步提交 | 无跨参与者等待，不存在歧义终态 |

## 流水线接口

与部署流水线（publish / activate / verify / rollback）对齐：

- **publish**：产出 buildId + deployment 记录（+ assembly 快照 bundle），写入不可变 store；
  幂等（同内容同 identity）。
- **deploy**：更新指针（单键原子）+ 可选预载提示；目标环境（dev / 线上）只是不同的
  store / router / 指针表实例。
- **verify**：health / smoke 门禁，按 bundle 验证。
- **rollback**：指针指回旧 buildId。

watch 只是 deploy 的自动触发器，不拥有流程。

## 与既有文档的关系

- [`runtime-deployment-topology.md`](runtime-deployment-topology.md) 的"每个 profile 只有一个
  active RuntimeAssembly / 全量加载"部分由本文取代为"按 buildId 懒加载、多版本并存"。
- [`managed-dev-watch.md`](managed-dev-watch.md) 中 committed generation、activation CAS、
  expectedGeneration 的契约由本文取代为"指针更新 + 幂等 deploy"。
- [`release-registry.md`](release-registry.md) 的 release lifecycle 保持；本文把
  "dev lifecycle 的原子切换"具体化为指针更新。

## 迁移路径

1. 指针表落地：在 typed pointer store 中新增 release 指针键 `(profile, serviceId, version)`，
   由 publish 写入（与 deployment 记录同事务）。
2. runtime 懒加载：按 buildId 构建 image 的路径复用现有 loader；注册从"一个 active assembly"
   扩展为"已加载 buildId 集合 + 能力通告"。
3. router 派发切换：候选集并入"能力者"；保留 fail-closed。
4. 移除 activation 协调层：activation_state 仓库、coordinator、epoch、lease、snapshot 提交
   逐个下线；watch 改走指针 + deploy。
5. 存量：旧 committed 世代可视为"全部版本键的当前值"一次性迁移进指针表。

## 开放问题

- 懒加载 image 的内存上限与逐出策略（本地策略，先不做）。
- 同版本覆盖的窗口期是否需要在线上默认开启预载提示。
- release 指针与 package/service pointer 的复用与命名冲突。
