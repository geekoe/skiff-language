# P5-F276 Contextual same-heap audit result

状态：Audit complete；实现等待 PackageArtifact semantic-facts wire 扩展批准。

## 结论

AIHub 的 `managedLlm.streamChat` 与 `managedLlm.validateChat` 在 fresh canonical store 中稳定为
6/8 Available，唯一原因都是 `requiresSameHeapIdentity`。真实链为：

```text
Map.get
  -> std.json.merge(Fresh)
  -> applyProviderOptions
  -> managedResponsesBodyJson
  -> encodeResponsesBody
  -> stream/validateManagedChat
```

首次精度丢失在
`compiler/source/src/callable_effects/transfer/call.rs`：callee 的 aggregate identity 位为真且
actual 是 direct caller reference 时，立即把该位上浮，早于返回 provenance 被精确 Fresh
consumer 消除。

## 为什么不能只改内存分析

当前内部只保留 `same_heap_identity_parameters: BTreeSet<u32>`：

- 不保存 parameter projection；
- `into_summaries` 不序列化；
- 跨 Package `from_semantic_facts` 无法恢复。

仅靠现有 wire，以下两种 callable 的已发布 facts 不可区分：

```text
A: return map.get(key)
B: map == map; return map.get(key)
```

A 的返回 identity 条件在精确 Fresh/detached consumer 后可消除；B 已发生独立 identity
observation，绝不能消除。因此内部特判会在跨 Package replay 时不健全。

## 建议结构

为 `CallableProvenanceSummary::Analyzed` 增加必填、严格 wire：

```text
sameHeapIdentity:
  unscopedObservation: bool
  observedOwners: SameHeapIdentityOwner[]
  returnOwners: SameHeapIdentityOwner[]
  directReturnOwners: SameHeapIdentityOwner[]
```

owner 只允许：

- caller parameter；
- caller parameter + 结构化 projection path。

路径复用现有 `ValueProjectionPath` canonical/长度规则。`directReturnOwners` 必须是
`returnOwners` 子集；aggregate `requiresSameHeapIdentity` 由结构化状态严格派生。缺失字段、
非法路径、错误 parameter、集合关系或 aggregate 不一致全部失败关闭，不提供兼容读取。

内部模型相应要求：

- `AbstractValue` 同时携带 reachable/direct identity owner；
- 完整 `CallerReference { parameter, path }` 取代裸 parameter set；
- equality、caller graph write 等进入 independent observed owner；
- Fresh/detached native 不复制输入 token；
- heap store/project/materialize 使用与 provenance 相同的投影、提升和降级规则；
- unknown/dynamic 保持 unknown/unscoped；
- SCC 对结构化集合做单调 fixed point；
- builtin/native registry 显式声明“随返回传播”或“独立观察”，不得按函数名特判。

## 必须保持的正负边界

- AIHub `Map.get -> std.json.merge`：alias 与 identity token 均被 Fresh consumer 截断，8/8。
- 直接返回 `Map.get`：保留 direct owner 并拒绝。
- 存入 Fresh wrapper 再返回：owner 仅 reachable，可 materialize；之后投影该字段会提升为
  direct 并重新拒绝。
- caller write/escape、reference equality：独立观察不能被 Fresh consumer 清除。
- Fresh/alias conditional、unknown/dynamic、跨 Package replay、SCC/递归继续失败关闭。
- Fresh wrapper 内含 caller child 时，field projection 必须恢复 caller owner。
- 现有 heap cycle/unsupported store 算法不放宽。

## Wire 与 identity

- PackageArtifact semantic-facts wire 与 PackageBuildId 必然变化。
- Package Local ABI 不变。
- boundary eligibility threshold 与 operation contract 规则不变。
- AIHub 新增两个 Available operation 后，其 ServiceContract surface、protocol identity 和相关
  deployment/assembly identity按正常规则变化。
- 不修改 DSL、AIHub/Agine/std 源码或 runtime materialization 语义。

主 Agent 已向用户请求批准该严格 wire 扩展；不接受只在当前编译器内存状态做局部特判。

审计 worktree 未修改文件；基线聚焦测试通过。

