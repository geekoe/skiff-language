# P5-F274 Package typed-throw projection audit result

状态：Audit complete；实现 `TASK_NOT_EXECUTABLE`，等待语言设计决策。

## 结论

F266 记录的 Package callable `throw_types` 恒为空不是单一 projection 漏填。当前语言没有
“函数声明的可抛错误集合”这一事实 owner：

- `FunctionDecl`、`InterfaceOperation` 和 parser signature 都没有 throws 字段；
- syntax reference 只定义参数和返回类型；
- `SourceExecutableSignature` 没有 error set；
- source public callable 与 topLevel implementation callable 的生产路径都只能写空列表；
- native signature 也没有 declared error set；
- provenance 的 `throwsCallerAlias` 描述值图效果，不是业务错误类型声明。

因此不能从函数体、`throw` 表达式或 effect analysis 推断 Package 公共错误契约。这样做会让
跨函数、动态调用和实现替换改变未声明的 ABI，并违反父任务“不增加 fallback”的约束。

## 已完整的下游

若上游能提供具体 nominal error leaf，现有下游大部分已经完整：

- `PackageCallableSignature.throw_types` 是 PackageArtifact 必填 wire；
- public callable normalization 会归一化具体 Package type；
- dependency ingest 精确转交 public/topLevel signature，缺失会失败关闭；
- inline effect type-check 要求实际 typed throw 恰好匹配一个声明项；
- lowering/linker/runtime 已有 linked type plan、detached error materialization、nominal identity、
  `UserException` 和 `catch<T>`。

projection-input 文件中的空列表只出现在测试 fixture；生产空列表的 owner 在 source
signature 和 topLevel reconstruction。

## 需要先决定的公共语义

实现前必须确定：

1. declared throws 的源码语法与签名 owner；
2. artifact 保存源码类型还是展开后的 concrete catch leaves；
3. duplicate、alias-equivalent、union、排序与重叠规则；
4. interface requirement 与 implementation 的 error-set conformance；
5. topLevel 和 native callable 是否同样覆盖；
6. Package throws 是否原样进入 ServiceContract，或使对应 service boundary unavailable。

主 Agent 已向用户提出推荐方案：

```skiff
function fetch(input: Request) -> Response
  throws NetworkError, DecodeError
```

推荐语义为：别名/联合展开成稳定排序、互异的具体 nominal error leaves；函数体及传递调用只能
抛声明集合的子集；接口实现可缩小、不可扩大；public、topLevel、native 共用同一机制；Service
operation 复用 Package declared set。

## Identity 与额外缺口

- 不需要新增 PackageArtifact wire 字段或历史兼容。
- public throw set 会改变 Package Local ABI、PackageBuildId 与 consumer expected Local ABI。
- 进入 service boundary 时也会改变 contract/protocol 内容身份。
- topLevel implementation signature 当前未被 validation/build identity 完整覆盖。若纳入
  declared throws，implementation signature 必须进入 build identity，但不能污染 public Local ABI。
- 多错误不能作为一个未展开 union entry 交给 runtime；必须先成为互异 concrete leaves，否则
  catch-leaf materialization 可能得到多个候选并拒绝。

## 生态与后续测试

当前已迁移 inline effects 中没有 Package `throw:` outcome，所以它不是 F268/F269 的现行动态
blocker；生态中仍有大量实际 `throw`/`catch<T>`，官方 Package 也会传播 provider、protocol、
decode 等错误，故不能把恒空列表当作长期完成状态。

设计确定后的最小矩阵必须覆盖：

- 同 Package public error、跨 Package error、alias、两个互异 error；
- 零声明、未声明错误、结构相同但 nominal identity 不同；
- primitive/interface/空 catch leaves、duplicate/alias-equivalent/overlap；
- private error 的 public consumer、缺失 owner/closure、stale type id；
- interface conformance、topLevel build identity mutation、native declaration；
- Package publish/import → test-service inline typed throw → `catch<T>` 的真实执行链。

审计 worktree 未修改文件、未运行测试。

