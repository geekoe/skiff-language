# Skiff 语言概览

> 用业务语言写后端，把服务边界、平台能力和运行规则交给 Skiff 固定下来。

## 定位

Skiff 是一门面向后端业务的语言。它让开发者直接描述服务 API、业务类型、数据读写、后台协作、HTTP / WebSocket 入口、配置、日志和测试，而不是从框架、SDK、进程、连接池、路由表和部署脚本开始拼后端。

Skiff 不是通用编程语言，也不是某个 Web 框架外面套一层新语法。它的重点是把后端服务最容易反复出错的边界变成语言和平台都能理解的东西：

- 这个服务对外暴露哪些 API。
- 请求和响应是什么类型。
- 哪些数据可以跨服务、入库或返回给客户端。
- 哪些操作会访问数据库、外部 HTTP、WebSocket、日志或后台工作。
- 超时、取消、stream、错误和并发如何发生。
- 测试代码和生产代码如何隔离。

因此，Skiff 代码不只是“能运行的函数”。它同时描述业务逻辑和运行边界，让人和 AI 都能在较少上下文里判断这段后端代码会怎样被调用、怎样失败、怎样被测试、怎样发布。

## 你写的是什么

一个Skiff Package由`.skiff`源码、`package.yml`和必需的`api.yml`组成。Service首先是
Package：同一个root额外放置`service.yml`，并可提供`config.yml`、按环境选择的
`config.<profile>.yml`和ignored `config.<profile>.secret.yml`。

`.skiff` 文件里写业务类型、函数、接口实现、测试、数据库对象和对平台能力的调用。源码以文件为模块，当前服务或包内的跨文件访问走 `root.*` 内部绝对路径，外部包通过配置里的 alias 引入。

文件是组织单位，不是包内 privacy 边界。`root.*` 可以引用当前 Package production source set 的顶层
声明，包括未公开 helper。Skiff source 不使用 `export` 决定可见性；`api.yml` 是 Package public API 和
service-to-service API 的唯一公开符号入口。

`package.yml` 拥有 Package id/version以及package、service dependency。`service.yml`增加service id、
`serviceCalls` public-path选择和HTTP/WebSocket等外部入口，包括route、handler与协议适配信息；
`serviceCalls`只引用`api.yml`中的公开名字，不重复source path、签名或类型。一个service的三层配置文件
都以canonical Package ID为根key，同时容纳service自身和dependency Package的业务配置；它们不配置
数据库名、state binding或平台resource policy。

Skiff 从 `api.yml` 与已类型检查的 Package API 确定性生成 ServiceContract。跨服务调用按该contract和
deployment binding调用目标服务，而不是靠运行时字符串、临时service locator或“差不多兼容”的猜测。
HTTP/WebSocket则按`service.yml`生成独立gateway entry，不会因为handler相同而变成service operation。

## 语言表面

Skiff 的语法故意克制。它优先让业务代码清楚、稳定、容易生成和修复，而不是追求所有编程技巧都能表达。

当前主要表面包括：

- `type`、`alias`、`interface`、`impl` 用来表达业务数据和能力约束；interface 的独立语义见
  `reference/interface.md`。
- `function` 写普通业务逻辑，参数和返回类型显式声明。
- `test` 写测试，测试代码不会进入生产代码。
- `db object` 描述服务自己的持久对象。
- `concurrent`、`serial`、`timeout(...)`、`emit`、`throw`、`catch`、`rethrow` 显式表达并发、超时、stream 和错误。
- `std.*`、`config` 和普通 package 调用平台能力或复用能力。

这些限制让代码边界更清楚。例如普通 block 不产值，`if` 是语句，需要产值时使用 `value` 或 `match`；跨服务 payload 和持久化数据必须是能被明确检查的类型；并发和超时不会因为某个普通函数调用被偷偷引入。

## 服务和包

Skiff 区分 package 和 service。

Package 是唯一源码编译单元，也是复用单元。它适合放通用类型、工具函数、SDK wrapper、业务共享逻辑和
可测试的能力封装。调用 Package 是同一 linked program 内的本地调用，不经过 service boundary；Package
只声明自己读取的typed local config path和代码能力，不拥有某个部署环境中的实际配置值、数据库或外部
入口。

Service 是 Package 的运行角色和 activation/resource owner。它可以从 Package API生成service-to-service
contract，并通过`service.yml`拥有HTTP/WebSocket入口。跨service调用即使第一版在同一runtime进程内执行，
仍保持独立heap materialization、错误、stream、取消与ActivationContext边界，不能退化成Package直接调用。

这个区分会影响代码组织：

- 想复用逻辑，优先抽 package。
- 想提供服务间能力，先在`api.yml`中公开callable，再把其public path列入
  `service.yml.serviceCalls`。
- 被选择的callable必须能进入service boundary；其签名引用的公开types由compiler自动闭合，不重复列一份
  Service API。
- 想共享类型或 helper，优先把它们放在可直接依赖的 Package API 中。

## 平台能力

Skiff 把常见后端能力做成语言能理解的平台入口，而不是让每个应用自己拼 SDK。

`std` 是内建平台标准库 root，不是普通 package。源码可以直接访问 `std.http.*`、`std.json.*`、`std.crypto.*`、`std.log.*`、`std.websocket.*` 等能力；`import std` 只是保留的显式写法，不是使用 `std.*` 的前提。

`config`是当前Package的只读局部配置视图。源码以`config.require<T>("local.path")`或
`config.optional<T>("local.path")`读取自己的分区，不能读取另一个Package的配置。不同Package的值写在
同一份service配置文件中，但由canonical Package ID分区；配置变化形成独立runtime配置快照，不改变代码、
deployment或assembly identity。详见`reference/config.md`。

`db object`面向宿主service唯一、由平台按profile与service identity派生的数据库。开发者不选择
数据库名；同一service中的所有Package共享该数据库，同时保留各自精确Package/schema/collection identity。
业务代码描述对象schema、查询和显式写入操作，而不是暴露连接串、Mongo filter或update operator。

actor 是内存面的可寻址常驻对象，目标态契约见 `architecture/actor-model.md`；后台提交见 `reference/dispatch.md`。长时间业务事实必须落到数据库、队列、timer 或业务自己的持久状态；数据面的单写者用 `db object` 的 lease 表达，见 `reference/db.md`。

`std.log` 和 telemetry 用于诊断，不用于业务正确性。审计、扣费、任务进度、可靠通知和 outbox 这类事实不能靠日志送达来保证。

## 运行边界

Skiff 的运行规则偏保守：没有显式写出的并发、后台执行、超时或共享状态，代码就不应该被读成“可能偷偷发生”。

跨服务调用按精确 API 契约路由。API 的参数、返回和相关类型变化会影响调用方是否还能安全调用；Skiff 不假装自动适配不同协议。

测试和生产分开。`*.test.skiff` 中的测试、helper 和测试替身只参与测试，不进入生产服务，也不改变生产 `root.*` 解析、校验或发布身份。测试可以复用生产语义，但不能改变生产发布结果。

请求是一段有限生命周期。请求结束后，请求内的临时对象、stream、错误信息和资源句柄都应该结束。需要跨请求保留的状态，要明确放进服务数据库、持久队列、timer 或业务自己的持久状态。

HTTP 和 WebSocket 是服务入口，不是普通函数调用的别名。Skiff 平台负责连接和外部协议适配，service 代码处理已经进入服务边界的请求、消息或 stream。

## 设计取舍

Skiff 更偏向“清楚、可检查、可发布”，不偏向“所有写法都支持”。

- 少一些隐式行为，换来更稳定的生成、修复和 review。
- 明确区分 package 复用和 service 远程调用，避免把部署边界藏在普通函数里。
- 把平台能力做成内建入口，但要求它们说清楚失败、取消、超时、测试替身和持久化边界。
- 把业务正确性放在持久状态里，把 telemetry 留给诊断。
- 不为尚未发布的旧格式背兼容包袱，让语言可以继续向正确模型收敛。

## 仍在收敛的能力

Skiff 还在发展中，尤其是 DB / data storage、work、file / command、WebSocket、actor / session、长时间任务和发布运维相关能力。介绍这些能力时应区分已经稳定的规则、当前实现和未来方向。

这不改变 Skiff 的核心方向：开发者写的是后端业务和服务边界，Skiff 负责把这些边界固定成可测试、可发布、可运行的服务。
