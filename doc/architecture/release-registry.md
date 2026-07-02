# Skiff Release / Registry Architecture

## 本文负责 / 不负责

本文负责定义 Skiff dev sync/reload、registry source revision、package version pointer、service
release version/buildId、runtime activation/dynamic linking、rollback 和 audit 的架构模型。

本文不负责语言概念 reference、完整 manifest / artifact schema、CLI 命令手册、部署脚本、具体端口、
runtime stack 发布或历史兼容格式。Publication 和 artifact 的稳定概念属于 reference 层。

## 1. 操作分层

发布相关操作分成三条生命周期：

- Dev lifecycle：本地编译、同步 dev artifact root、router reload、runtime 重新 activation。
- Package registry lifecycle：package 源码发布、immutable source revision、trusted compile、
  package version pointer 移动。
- Service release lifecycle：service 源码发布、trusted compile、service version pointer 移动、
  runtime activation。

三条生命周期共享 typed artifact 和 runtime linking 模型，但信任边界不同。Dev 可以用本机 compiler
产生 dev artifact；production 只能使用平台可信 compiler；dev artifact 不能作为 production publish
的权威输入。

## 2. Dev Sync / Reload

开发态不叫 publish。开发态只表达“把当前本地输入同步成可 reload 的 dev artifact”。

- sync：从当前 source root 和 profile overlay 编译，写入 dev artifact root，原子更新 dev pointer。
- reload：router 重新读取 artifact root，更新 active snapshot，并把 control state 下发给 runtime。

dev pointer 是 mutable latest：不保留正式版本历史，不做 contract compatibility gate，不表达线上
release，但仍必须指向带有 build identity 的 artifact。

dev reload 与 release routing 共享 runtime lookup 原则：router/runtime dispatch 使用 service id、
activation build identity 和 target，不依赖缺失 build identity 的兼容路径。

profile 只选择本地 overlay，不是发布渠道，也不是 service version。需要表达外部兼容线时，必须进入
service release lifecycle。

## 3. Registry Source Revision

Production package publish 的第一件事是创建 immutable source revision。

source revision 是 registry 对一次 package 源码快照的不可变记录，至少保存 package id/version、
source snapshot 引用、source content hash、package manifest hash、package-to-package dependency
resolution snapshot、publisher、publishedAt 和 registry generation。

source revision 不是 service dependency lock。service manifest 不引用 package source revision；
Service Unit 运行时不固定 package source revision；service release record 可以保存 compile 时看到
的 source revision，用于审计和复现。

Production trusted compiler 从 immutable source revision materialize source root，再输出 File IR
Units 和 Package Unit。用户本地 IR 不进入线上信任边界。

## 4. Package Version Pointer

Package version 是人工维护的兼容版本线。`id@version` 指向当前 source revision / package build，
这个 pointer 可以移动。

Package version pointer 规则：

- 首次发布创建 pointer。
- 同版本 bugfix 可以把 pointer 移到新的 source revision 和 Package Unit。
- registry 不用 ABI identity 等值作为 pointer 移动 gate。
- pointer 移动必须通过 CAS generation 或等价机制防止并发覆盖。
- 每次移动都写入 append-only history。
- 移动后 materialize runtime 可读的 package index，并触发 reload 或 activation refresh。

同版本兼容性由 package 发布者负责。平台安全边界是 trusted compile 成功、immutable artifact blobs
完整写入、pointer history 可审计、runtime activation fail closed，以及 rollback 可以指回旧
revision / build。

Service 依赖 package 时只声明 id、精确 version 和 alias。runtime selection 使用当前 package version
pointer；Service Unit 不把 package build identity、assembly identity、source hash 或 source
revision 当作运行时依赖锁。

## 5. Service Release Version 和 Build Identity

Service version 是 service 外部兼容线，来自 service manifest，是请求、长时队列、定时器和 rollback
操作面对的正式版本键。

Service build record 是 service 源码编译产生的不可变记录。它用于找到 Service Unit、service-owned
File IR Units、protocol / ingress / config metadata 和编译审计信息。

Runtime activation build identity 是动态身份，由 Service Unit、service-owned File IR Units、当前
resolved Package Units、package ABI expectations，以及影响执行的 runtime IR / linking schema 共同
决定。

因此，同一个 service version 可以因为 package 同版本 bugfix 重新 activation 出新的 runtime build
identity，而 service version 不变。

Service version pointer 把 service id + version 指到当前 service build record。它不直接证明当前
activation 已经成功；activation 仍需要解析 package、校验 ABI 并完成 typed link。

只有 service 外部兼容线变化时才提升 service version。修复实现、更新 compatible package 或替换同一
version 的 build，应该通过移动 pointer 完成。

## 6. Service Publish

Production service publish 的权威输入是 service source、service manifest、registry 当前 package
pointers 和平台可信 compiler。

操作模型：

- snapshot service source。
- production 禁止本地 package source override。
- 通过 registry 解析每个 package id@version。
- trusted compiler 读取 immutable package source revisions。
- 输出 Service Unit、service-owned File IR Units 和 service build record。
- 保存 compile dependency snapshot 作为审计信息。
- 移动 service version pointer 到新的 service build record。
- 触发 router/runtime reload 或由 registry/build event 驱动 refresh。

Service Unit 保存 package dependencies 和 package ABI expectations。dependencies 决定 activation
时解析哪些 package id@version；ABI expectations 决定 resolved Package Unit 是否兼容；compile
dependency snapshot 只用于审计，不参与 runtime package selection。

发布失败应尽早暴露：source 不可解析、dependency 不存在、schema closure 失败、operation target
无法链接、artifact 写入不完整或 pointer CAS 失败都属于发布错误。

## 7. Runtime Activation 和 Dynamic Linking

Runtime activation 是 release model 的最后一道安全门。

稳定流程：

- 根据 service id + service version 读取 service version pointer。
- 加载 service build record 和 Service Unit。
- 读取 Service Unit 的 package dependencies。
- 对每个 package id@version 读取当前 package index / pointer。
- 加载 resolved Package Units 和相关 File IR Units。
- 校验 package ABI expectations 和 service 实际 used symbols。
- 构建 link overlay。
- 计算 runtime activation build identity。
- runtime 注册 service id、activation build identity 和可 dispatch targets。

activation 必须 fail closed：package pointer、Package Unit、File IR Unit、ABI compatibility、used
symbol、typed address 或 artifact schema 任一环节不满足，都不能继续 dispatch。

Router dispatch 不应自动 fallback 到 latest service version、旧 package build 或旧 runtime build。
没有匹配 active runtime 时，返回 unavailable，并保留诊断信息。

## 8. Request Routing

线上请求首先选择 service id 和 service version。service id 通常来自受信任 ingress、域名、route
binding 或 `X-Skiff-Service`；service version 来自 `X-Skiff-Version`、受信任入口或 router 默认策略。

路由语义：

- 未知 service version fail closed。
- service id + version 解析到当前 service build record。
- runtime activation 解析出当前 runtime build identity。
- router 只 dispatch 给注册了同一 service id、runtime build identity 和 target 的 runtime。
- pointer 更新后，新请求使用新 activation；已进入旧 activation 的 in-flight request 继续完成。

长时队列、定时器和跨发布保存语义的上下文默认绑定 service version。真正执行时再解析到当时有效的
activation build identity。只有必须完整复现旧实现时，才显式冻结到某个 build record 或 activation。

## 9. Rollback

Rollback 是 pointer 操作，不删除历史。

Package rollback 把 package id@version pointer 指回旧 source revision / Package Unit，递增
generation，materialize package index，并触发 reload / activation refresh。已经运行在旧 activation
的请求按原 activation 完成；新 activation 使用回滚后的 package。

Service rollback 把 service id + version pointer 指回旧 service build record，不改变 package
version pointers，并触发 reload / activation refresh。activation 仍会解析当前 package pointers，
并重新计算 runtime build identity。

如果需要完整回到某次 service 发布时的 package 组合，必须同时执行 package pointer rollback，或使用
平台明确支持的冻结 build replay。默认 service rollback 不隐式改变 package registry 状态。

强制下线或 retire runtime activation 是独立操作。它必须考虑 in-flight request、server stream、旧
WebSocket socket、dependency lock 和 entry identity drain，不能由 pointer rollback 隐式完成。

## 10. Audit 和 Inspect

每次 production 操作都必须留下可审计链路：

- source revision 的发布者、source hash、manifest hash 和 dependency resolution snapshot。
- trusted compiler 版本、build provenance 和 immutable artifact identities。
- package version pointer 的 old/new revision、generation 和 timestamp。
- service version pointer 的 old/new build record、generation 和 timestamp。
- service compile dependency snapshot。
- activation 时 resolved Package Units、ABI expectation 校验结果和 runtime build identity。
- rollback 的目标、发起者和原因。

Audit 信息服务于复现、诊断和回滚：某次发布用了哪些 source / artifact，某个 service version 为什么
activation 失败，以及应该移动哪个 pointer 才能恢复已知可用状态。

## 11. 不变量

- Production IR 只来自平台可信 compiler。
- Immutable source revision 和 artifact blob 一经写入不修改。
- Pointer 可移动，但 history append-only。
- Package version pointer 移动不需要 service 重新编译，但 activation 必须重新校验 ABI。
- Service version pointer 移动不隐式固定 package build。
- Runtime executable source of truth 是 typed units link 后的 RuntimeProgram。
- 缺少 selector、pointer、artifact、runtime registration 或 ABI compatibility 时 fail closed。
- Dev sync/reload 与 production publish 使用相同 linking 模型，但不共享信任边界。
