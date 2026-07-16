# P1-T01：收敛 Linkable / Recoverable Value 契约

状态：`ready`
类型：架构前置任务
依赖：P1-T00
执行者：独立文档 Agent，一份提交

## 目标

消除当前文档中“boundary-passable / 可序列化 / recoverable / service-owned concrete value”之间
的含混，使后续 DTO、effect 和 projector 能从 canonical 文档得到唯一答案。

## 已授权的结论

Agent 应把以下结论写成 canonical 契约，而不是重新发散讨论：

1. `LinkableValuePlan<Lane>` 表示值现在可在指定 lane 中 materialize。它是 lane-scoped plan，
   不是脱离用途的“可序列化”布尔值；即时 service-call lane可以使用 ordinary detached data或
   带 owner/route/lifetime的callback capability。
2. `RecoverableValuePlan<Lane> = LinkableValuePlan<Lane> + FutureValidityPlan`。后者证明 request
   结束、runtime reload或未来恢复时，identity、resolver和权限仍有效。
3. 即时 service call 不要求 recoverable。DB、spawn、queue、显式 recoverable slot 等跨 request
   lane 才要求 recoverable。
4. `any I` 与 native handle 的 Boundary ABI 形态都是 request-scope callback capability；不传
   method table/native object。默认不能进入 recoverable lane。
5. 所有 concrete code/type/method implementation 的 owner 都是 package/code identity；service
   deployment 只拥有 activation、config/state、capability endpoint 和路由身份。
6. 当前实施只提供 in-process service binding，但仍执行逻辑 Boundary ABI；future remote 可以
   复用同一 linkable contract。Local Code ABI 不要求 Linkable 或 Recoverable。
7. Phase 01 callback/stream value plan只定义owner、operation surface、route kind、request
   lifetime、失效、item/error/cancel channel；callback重入调度、stream buffer/backpressure和
   cancellation enforcement属于Execution Contract，延后Phase 03且不进入Phase 01 ABI identity。

## 范围

必须同步：

- `doc/architecture/package-code-and-service-deployment.md`
- `doc/architecture/recoverable-value.md`
- `doc/architecture/compiler-publication-pipeline.md`
- `doc/architecture/runtime-compiler-shared-artifact-types.md`

允许只修改直接涉及上述 owner/value plan 的段落，不做全文润色。

## 必须明确的事实

- 三条 lane：Local Code、即时 Service Boundary、跨 request Recovery。
- 同一值可对一个lane linkable、对另一个lane不可用；plan必须携带lane/carrier/lifetime语义。
- ordinary data、`any I`、native handle、callback、stream 在三条 lane 上分别允许何种 carrier。
- callback capability 的 owner、route、request lifetime、取消和失效行为。
- ABI/value plan与runtime execution policy的分界；不得让T09依赖尚未选择的调度策略。
- package code owner 与 service activation owner 的区别；删除或改写
  `LocalConcreteOwner::Service` 一类 service-own-code 表述。
- `LinkableValuePlan` 与现有 linked type/linker 名称的边界，避免把 code linking 和 value
  projection 混为一谈。
- compiler 产出 value plan；linker/dispatcher 只消费，不重做类型推断。

## 非目标

- 不修改 Rust/JS 代码。
- 不确定最终源码关键字、YAML 字段或 remote transport protocol。
- 不要求 recoverable 支持 request callback 的持久恢复。
- 不重写所有 recoverable 文档示例，只修直接矛盾与必要术语。

## 验收标准

- 四份 canonical 文档对上述七条结论无冲突。
- `recoverable-value.md` 不再暗示 service 是 concrete code owner。
- 文档明确说明 callback capability “可即时链接但默认不可恢复”的原因是 lifetime/resolver，
  而不是宿主语言缺少文件或某个通用 OS 例子。
- Phase 01 后续 Agent 不需要自行决定普通 service 参数是否必须 recoverable。
- T09不需要先决定callback重入调度或stream buffer策略即可生成稳定value plan。
- 没有为未来 remote 设计 wire 格式。

## 聚焦验证

```bash
rg -n "LinkableValue|RecoverableValue|LocalConcreteOwner|callback capability|Boundary ABI" \
  doc/architecture/package-code-and-service-deployment.md \
  doc/architecture/recoverable-value.md \
  doc/architecture/compiler-publication-pipeline.md \
  doc/architecture/runtime-compiler-shared-artifact-types.md
git diff --check
```

## 停止条件

若现有语言设计无法决定以下任一点，整理成一个具体问题后询问用户，不要自行选择：

- callback capability 是否允许越过当前 request；
- runtime reload 后是否存在稳定 callback resolver；
- service deployment 是否应拥有 concrete code/type identity；
- in-process service call 是否允许绕过 detached/capability boundary semantics。

## 提交

提交信息建议：`docs: define linkable and recoverable value layers`
