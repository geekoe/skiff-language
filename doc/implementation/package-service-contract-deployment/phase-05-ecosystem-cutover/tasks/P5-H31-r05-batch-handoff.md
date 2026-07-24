# P5-H31：Phase 5 Current Batch Handoff

状态：Implementation Checkpoint。核心架构与主要consumer迁移已合流，但普通service-call stream、真实service
重验和最终combined/acceptance尚未关闭；当前不是冻结验收候选。未push、未merge main、未操作stable。

旧版handoff中的D41/R05 ready queue已经被后续架构修订和F94–F135实现批次取代，不得继续按旧队列调度，也不得把
旧R05/I30证据当作当前候选的最终证据。

## 权威输入

- 权威设计：
  `doc/architecture/package-service-contract-deployment.md`
- 多Agent工作流：
  `/Users/geek/workspace/multi-agent-development.md`
- 工作区规则：
  `/Users/geek/workspace/AGENTS.md`
- 相关长对话：
  `019f8e59-3176-77d1-9285-5a021b7279aa`

若聊天摘要、历史task或result文档与权威设计和当前代码冲突，以权威设计及用户在上述对话中确认的决策为准。

## 精确代码状态

### Skiff

- main worktree：`/Users/geek/workspace/skiff`
- integration worktree：`/Users/geek/workspace/skiff-phase-05-integration`
- branch：`codex/package-service-phase-05`
- HEAD：`31539e38803002726d97efb1b04e0d1e5accfba1`
- tree：`dc877b32d4cf4de63d75fd4408ad51b6825b61f3`
- Cargo.lock blob：`b3ba25796c6ccd6f901717c5d103aca90b162b8e`

### skiff-packages

- main worktree：`/Users/geek/workspace/skiff-packages`
- integration worktree：`/Users/geek/workspace/skiff-packages-phase-05-integration`
- branch：`codex/package-service-phase-05`
- HEAD：`6e6828c38d6634bf0bd538dbcbd2532815f246c2`
- tree：`ab445e5aee79e36ec903cbd9955c54c40b6d982f`

### Internals

- main worktree：`/Users/geek/workspace/internals`
- integration worktree：`/Users/geek/workspace/internals-phase-05-integration`
- branch：`codex/package-service-phase-05`
- HEAD：`2cf2ebd22b502d0b2069dd6ef5db8ee4dd9032f2`
- tree：`88f8925a07019a053147de246ec54ef3f8fca4e0`

记录本handoff前，以上六个worktree均clean。已完成的F118–F135临时worktree和临时分支已清理；没有运行或中断的
子Agent。恢复工作时应查询实际可用Agent槽位，不在handoff中固化并发上限。

本handoff提交会使Skiff HEAD/tree前进，但只允许改变本文档。恢复工作时必须重新记录实际HEAD/tree/status，并确认从
上述Skiff HEAD到handoff HEAD没有production diff。

## 已确认架构

- Service首先是Package：
  - Package包含`.skiff`、`package.yml`、`api.yml`；
  - Service在此基础上增加`service.yml`和`config.*.yml`。
- `package.yml`拥有package id、人类可读version以及精确package/service dependencies；package可以依赖service，
  aliases共享命名空间。
- version不参与PackageBuildId、ArtifactId、ContractTypeId或其他identity计算。
- `api.yml`是Package和Service唯一公开API owner；满足service-boundary的公开函数自动成为Service API，不满足者保留为
  Package API，并通过CLI/JSON/receipt给出结构化原因。
- `service.yml`只拥有service id和HTTP/WebSocket ingress；禁止developer-owned `contract.yml`、`deployment.yml`，
  禁止`access`、`organizationRole`及service级response byte policy。
- ServiceContract和ServiceDeployment由compiler生成；Service API语言类型复用普通Package类型机制，不引入第二套
  ServiceContract语言类型系统。
- package link绑定精确artifact/build identity；service dependency使用精确service selector，provider deployment可以在
  调用方不重新编译的情况下更新实现。
- Router和Runtime不知道Registry；Registry是`skiff-packages`中可选的官方Registry service，不是语言或std的一部分。
- Router只读取配置的共享`artifactsPath`。正式环境由Registry编译/发布artifact；开发环境由compiler tooling/watch完成。
- Router拥有Mongo配置，并在bootstrap时向Runtime一次性下发`artifactsPath`和`serviceDb.mongoUrl`；Runtime和service
  不自行配置Mongo URL。
- Router HTTP配置为：

  ```yaml
  http:
    port: 4000
    maxRequestBytes: 67108864
    maxResponseBytes: 8388608
  ```

  不存在`limit` key。request ceiling由Router执行；response ceiling由Runtime和Router执行；streaming response按整个
  response lifecycle累计。WebSocket限制是独立议题。

## 当前已合流实现

### Skiff integration

- Service-as-Package manifest/input、`package.yml.services`与严格`service.yml`；
- 自动Service API projection、可用/不可用receipt、生成式ServiceContract和ServiceDeployment；
- package公开DTO闭包及service-owned nominal identity projection；
- canonical local/root/helper call target facts与DB persistence provenance；
- canonical HTTP request/response/stream boundary types及detached materialization；
- version退出identity preimage；
- Router直接从`artifactsPath`加载snapshot，不依赖Registry或compiler sidecar；
- Router内置Mongo activation state owner；
- Router→Runtime bootstrap下发artifacts path、Mongo URL和response ceiling；
- Router request/response ceiling与Runtime response ceiling；
- instance/dev-init/deploy tooling强制显式Router HTTP byte配置；
- legacy service access、organization role和service response policy删除；
- retired `contract.yml`/`deployment.yml` authoring命令、fixtures与test-runner路径清理。

### skiff-packages integration

- Registry迁移为普通service package：
  - 增加`package.yml`和`config.dev.yml`；
  - `service.yml`只保留service专属字段；
  - `api.yml`拥有公开类型与函数；
  - 删除developer-owned contract/deployment及生成器。

### Internals integration

- Agine、AIHub、Codex Relay和Account迁移为service package authoring；
- shared service workflow切换为单一package/service authoring；
- Account service级response policy删除；
- 旧contract/deployment文件删除。

开发Agent的聚焦测试、Router type-check、Runtime response ceiling、manifest/input、bootstrap/session以及authoring结构测试曾
分别PASS；这些证据只证明各自提交的局部边界，不等于当前三仓库候选已通过combined或最终gate。

## 已知剩余DAG

### C0：恢复与机械闭合

1. 在当前integration HEAD重新确认三个仓库status、task事实和剩余production owner。
2. 重跑并修复
   `package_artifact_assign_validate_and_golden_identities`组合golden：
   先证明新值符合version退出identity及最新schema，再更新expected；不得盲改golden。
3. 更新必要task/checkpoint文档，使后续Agent不再沿用旧D41/R05队列。

### C1：共享service-call stream能力

审计并闭合普通service-call的generic/stream contract、compiler lowering、wire和Runtime materialization。HTTP boundary已支持
`HttpResponseStreamEvent`，不能据此推断service-call stream已经完成。AIHub managed LLM stream是该共享能力的首要真实
consumer。

若审计发现需要改变公共类型、stream lifecycle或错误语义，暂停该分支并向用户报告；不得由实现Agent自行发明协议。

### C2：真实service并行重验

在C0完成、相关共享能力可用后，从对应integration checkpoint建立全新独立worktree并行验证：

- Registry：20/20 intended operations Available，真实immutable record/pointer storage测试；
- Codex Relay：17/17 intended operations Available，30条routes与公开handler精确对应；
- Account：修复遗留旧式调用名（已知候选如`httpSession.read`到`httpSession/read`），验证21条routes；
- AIHub：区分interface declaration、instance method、internal helper和真正public executable callable，处理stream以及
  `config.dev.yml` authoring；
- Agine：确认零普通service-call operation可以是合法结果，HTTP/WebSocket ingress与service-call API彼此独立。

每个service task只修改自己的consumer范围。多个consumer若同时要求同一compiler/runtime抽象，停止局部补丁，把共同语义
提升为新的上游checkpoint。

### C3：跨仓库预验收

所有consumer合流后，在精确三仓库候选上由唯一owner运行一次cheap combined integration probe，至少覆盖：

- compiler到共享artifact root；
- package和service dependency精确解析；
- Router snapshot/reload；
- Runtime bootstrap/load；
- Service API availability receipt；
- 关键HTTP与service-call正负路径。

probe失败时先分类、批量修复，不得用重复完整E2E逐个发现blocker。

### C4：冻结、最终gate与独立验收

cheap combined PASS且无在途production写入后才能冻结稳定候选。随后由唯一gate owner组织不重复的分片：

- Skiff combined/gate；
- skiff-packages build/runtime acceptance；
- Internals canonical workflow与service acceptance；
- Router/Runtime/Compiler综合验证；
- Agine `npm run e2e:chat-smoke`；
- 全新只读Phase 5 acceptance Agent。

任何blocking修复都会结束当前stability epoch，只重跑受影响证据和必要combined，重新冻结后再给最终verdict。

### C5：仓库与stable收尾

最终acceptance PASS后：

1. 分别将三个integration分支合并回对应仓库`main`；
2. 删除已合并integration worktree和临时分支；
3. 不push，除非用户明确要求；
4. 只有任务`P5-V01-post-merge-stable-verification.md`且用户明确同意后，才允许操作stable instance。

## 调度约束

- 默认每个新DAG节点使用全新Agent；开发、gate和独立验收角色不复用。
- 每个开发Agent必须收到有界task contract，合同引用权威设计；prompt不重复或补写架构语义。
- 启动前查询实际可用Agent槽位并尽量并行。先完成共享checkpoint，再扇出互不重叠的service consumers。
- Agent完成并合流后立即删除其clean worktree及已合入临时分支；不得积累旧worktree。
- Agent等待窗口使用10分钟，不做一分钟轮询。
- 新的公共契约、CLI、安全边界、配置形状、运行架构或owner变化必须先报告用户；不得隐藏自行决策。
- 不运行stable，不push；跨repo修改分别提交。
