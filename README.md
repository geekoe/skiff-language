# Skiff（轻舟）

Skiff，AI 时代的后端编程语言。

中文名“轻舟”，寓意：AI 写后端，已过万重山。

Skiff 的目标很直接：**让 AI 一次就能写对后端，让开发者只需专注业务。**

## 设计目标

### 让 AI 一次就写对

Skiff 让服务 API、数据模型、数据库读写、后台任务、配置、测试替身和发布边界都有明确语法和规则。
AI 不需要猜“这个项目的框架约定是什么”，也不需要在一堆 SDK 和胶水代码之间推断隐含行为。

### 让开发者只关注业务

后端通用能力不应该每个服务重复搭一遍。Skiff 把数据库、队列、服务调用、流式响应、日志、配置、
测试和发布都纳入语言模型。开发者写业务对象、业务流程和业务边界，平台负责把它们运行起来。

### 低成本资源共享

Skiff 的服务是可编译、可加载、可路由的服务单元，不是“一写一个服务就复制一套基础设施”。同一台
机器可以通过一套 Skiff 运行环境提供很多服务：它们共享路由、运行、观测、队列调度和数据库连接等
平台能力，同时保留各自的服务身份、版本、配置和数据库命名空间。

这让小服务不需要承担独立进程、独立中间件和独立运维脚本的固定成本。服务数量增长时，平台基础设施
可以复用，业务隔离由语言和运行边界保证。

### 低成本扩展

扩展不应该要求业务代码改成“分布式代码”。Skiff 的目标是把多台机器抽象成一个可用的执行池：不同
机器上的运行节点注册自己能执行的服务版本和目标，路由层负责选择可用实例、切流、平滑下线和取消。

对业务代码来说，服务仍然按一个服务来写；需要更多容量时增加机器或运行节点，而不是把每个函数改成
手写分布式调度。

### 热部署和安全回滚

Skiff 服务编译成服务产物后由运行环境加载。更新服务产物或版本指针后，路由层和运行节点可以重新加载
新服务；新请求进入新版本，已经进入旧版本的请求继续完成。回滚也是移动版本指针，而不是
删除历史或手工还原机器状态。

## 为什么需要一门后端语言

通用语言写后端时，业务代码只是系统的一部分。一个可运行服务还需要：

- HTTP / WebSocket 入口和跨服务调用。
- 数据库对象、查询、事务和租约。
- 后台任务、队列、定时唤醒和取消。
- 配置、日志、trace、测试替身和 live smoke。
- API 发布、schema 兼容、服务版本和热加载。

这些能力如果都靠库和框架拼接，AI 需要同时理解很多隐含约定，最容易出错的也正是这些地方：
忘记事务边界、误用队列、漏掉测试替身、写出不可恢复的后台任务、让接口和数据库 schema 漂移、
把日志当业务事件、扩容时才发现服务和机器绑定太死。

Skiff 的做法是把它们收敛进语言：代码描述业务，语言描述边界，平台负责运行。

## 这些价值从哪里来

### 内置数据库

Skiff DB 是 service-owned typed object database。业务代码面对的是 `db object`、typed query、
projection 和显式写入操作，而不是 collection 字符串、Mongo filter 或 update operator。

```skiff
type User {
  id: string
  name: string
  visits: number
}

db object User {
  name "user"
  primary key(id)
}

const users = db find many User {
  fields { name, visits }
  where visits > 0
  order id asc
  limit 20
}
```

读取结果是 snapshot，持久修改必须显式使用 `db insert`、`db update`、`db upsert`、`db delete`
等操作。事务和租约也属于语言模型，用来表达原子更新和单执行者推进。

### 内置任务、中间件和调度

`spawn` 用来提交后台函数调用，让当前 request 继续执行。参数必须是可恢复值，不能把 request-local
资源、live connection 或不可持久化的远程能力塞进后台任务。

durable queue / timer 是平台调度机制，用于同步请求、后台任务、实体事件推进和未来唤醒。业务关键
事实应该落在 service-owned DB 中，后台任务只是唤醒和推进。

### 内置服务边界

Skiff 的 public API 不靠源码里的随手导出，而由 `api.yml` 显式声明。package 和 service 共享同一套
public API graph；service 会把公开函数和公开实例方法投影成可路由 operation。

这让 API、schema、依赖和发布身份可以被记录和检查。跨服务调用不靠字符串 service locator 临时查找，
而是绑定到发布时确定的协议身份。

### 内置动态能力

`interface` 是名义能力契约。需要可流动的动态能力时，用 `any I`，并在装箱点显式写 `as I`。

```skiff
interface ToolProvider {
  function listTools(self: Self) -> Array<ToolDefinition>
}

type LocalTools implements ToolProvider {
  name: string
}

function chooseProvider() -> any ToolProvider {
  return LocalTools { name: "local" } as ToolProvider
}
```

同一个 package 可以接受 `any I` 参数，由调用方传入本地实现或远程 public instance。AI 不需要发明一套
provider 注入约定；能力传递就是语言的一部分。

### 内置流式响应、超时和取消

`Stream<T>` 表达 server / source stream。返回 `Stream<T>` 的 operation 用 `emit` 输出有序 chunk。
超时、断线、调用方取消和 stream consumer 提前退出都会进入统一的 cancel / error 语义。

`concurrent` 是结构化并发，不是随手起后台线程。并发 lane 会受 effect metadata 和 mutable root 规则约束，
避免 AI 写出看似并发、实际数据竞争或顺序不确定的代码。

### 内置测试和观测

Skiff 只有一种测试源码语义：`test` block，且只出现在 `*.test.skiff` 文件中。测试替身按 stable target id
匹配，必须返回 schema-closed payload 或抛标准错误。

`std.log`、trace、metric 和 health 是平台观测能力。它们用于诊断，不承担业务正确性；可靠业务事件应进入
DB、queue、timer 或后续 durable event 能力。

### 支持热部署的服务模型

Skiff service 会被编译成服务产物并加载到运行环境。更新服务产物后，router / runtime 可以重新加载服务
实现，让服务改动快速生效。语言层的 API、schema 和依赖关系让热加载不是“把新代码塞进去”，而是在明确
边界下替换可执行服务。

## 与普通框架的区别

普通框架把大量后端规则放在文档、目录结构、注解、运行时约定和团队经验里。Skiff 把这些规则前移到语言：

- 数据库不是外部 SDK，而是 typed service-owned DB。
- 后台任务不是随手发消息，而是可恢复参数和受控调度。
- 服务调用不是字符串路由，而是发布时绑定的 operation。
- 资源共享不是靠把多个应用硬塞进一个进程，而是服务作为可加载单元共享同一套运行环境。
- 扩容不是改业务代码，而是增加运行节点，由路由层统一调度。
- 测试替身不是 monkey patch，而是按 stable target id 匹配。
- 日志不是业务事件总线，观测和业务状态有明确边界。
- 热部署不是绕过类型和 schema，而是基于 artifact 和 identity 重新加载。

这就是 Skiff 对 AI 友好的关键：AI 不需要猜“这个项目怎么约定”，它可以按语言规则生成可检查的后端代码。

## 阅读路线

- 语言语法：`doc/reference/syntax.md`
- 类型和边界：`doc/reference/static-semantics.md`
- interface / 动态能力：`doc/reference/interface.md`、`doc/reference/any-interface.md`
- runtime：`doc/reference/runtime.md`
- 标准库 surface：`doc/reference/std-surface.md`
- DB / spawn / queue：`doc/reference/db.md`、`doc/reference/spawn.md`、`doc/reference/queue.md`
- 发布和 ABI：`doc/reference/publication-api-yml.md`、`doc/reference/publication.md`
- 测试：`doc/reference/testing.md`

## 仓库入口

- `compiler/`、`syntax/`、`artifact-model/`、`artifact-identity/`：compiler 和 artifact identity。
- `runtime/`：Rust runtime crates 和 host process。
- `router/`：TypeScript service router 和 runtime control plane。
- `telemetry/`：TypeScript telemetry process。
- `scripts/`：CLI 和本地 instance 工具。
- `std/`、`prelude/`：Skiff 标准库源码。
- `test-runner/`：Skiff package 和 service 测试基础设施。
- `doc/`：canonical language、runtime 和 architecture 文档。

## 当前状态

Skiff 仍处于发布前阶段。当前仓库不为旧语法、旧 artifact 或旧配置格式提供兼容层；实现和文档
都以 `doc/reference/` 中的目标语义为准。部分能力在 reference 中是目标态设计，落地状态以当前
实现和测试为准。

开发协作说明见 `AGENTS.md`。许可证见 `LICENSE`。
