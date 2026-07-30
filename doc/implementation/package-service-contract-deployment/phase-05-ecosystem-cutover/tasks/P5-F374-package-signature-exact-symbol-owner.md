# P5-F374 Package signature exact symbol owner

状态：Ready。

## 直接父节点

- `P5-F372-package-signature-local-slot-owner-audit-result.md`

只按父节点已冻结的不变量实现。除非代码中的直接引用无法解释某个局部机制，不需要向上读取阶段级权威设计。

## 目标

修正PackageArtifact public callable signature丢失local nominal module owner的问题：

1. producer在写入Package Local ABI前，把不能提升为PackageSchema的local/publication nominal规范化为
   exact `ServiceSymbol { module_path, symbol }`；
2. consumer删除package-global unique-slot猜测，对任何残余ownerless `LocalType` fail closed；
3. 补齐全部嵌套类型路径，尤其是当前producer遗漏的内层`AnyInterface`；
4. 用真实`std.http.stream`和最小package发布链证明歧义消失。

## Production边界

允许修改：

- `compiler/projection/src/package_artifact/callables/normalization.rs`
- `compiler/projection/src/package_artifact/callables/mod.rs`
- `compiler/projection/src/package_artifact/visible_types.rs`
  - 仅在需要复用/抽取现有exact module-slot-to-symbol helper时；
- `compiler/source/src/type_resolution_model/shape_assignability.rs`
- 上述owner的直接测试；
- official std build identity golden。

不得修改：

- artifact model、DTO、schema/version、identity算法或prefix；
- compiled handoff、lowering、std source；
- runtime、request、Host、Router、test-runner；
- Internals或skiff-packages production source。

若实现需要新增artifact字段/variant，或exact symbol无法由producer现有module-slot事实唯一确定，停止并返回
`TASK_SCOPE_EXPANDED`，不要扩大写入范围。

## Producer要求

- `PackageSchema` promotion成功路径保持不变。
- promotion miss时，使用callable的真实source module及该module的type table得到exact symbol；禁止使用
  public/display path猜测。
- direct、parameter、return以及所有递归形状均执行相同规范化。
- 至少覆盖：
  - `PackageTypeRef::{Local, Container, Nullable, AnyInterface}`；
  - 内层`TypeRefIr`的Builtin、AppliedNominal、Record、Union、Nullable、Function、AnyInterface；
  - public alias与source module不同的情况。
- missing module-slot、ambiguous映射、private/nonexported symbol和错误owner均fail closed。
- 写出的`PackageLocalAbi.publicSymbols`中递归反搜raw `LocalType`必须为零。

## Consumer要求

删除`rehydrate_package_signature_local_type`对package全部module按slot寻找唯一owner的fallback：

- raw `LocalType`只有一个候选时也拒绝；
- 有多个候选时同样拒绝；
- `PublicationType`、`ServiceSymbol`、`PackageSymbol`和`PackageSchema`的owner-safe路径保持有效。

错误信息应明确指出artifact producer写出了ownerless package signature type，不再把“当前是否恰好唯一”当作
合法性。

## 测试与真实验证

正例至少覆盖direct parameter/return、Container、Nullable、外层/内层AnyInterface，以及
Builtin/Record/Union/Function/AppliedNominal；schema-eligible nominal仍为PackageSchema。

负例至少覆盖：

- raw LocalType单候选和双候选均拒绝；
- missing module-slot或symbol；
- private/nonexported symbol；
- 错误owner；
- public alias与source module不同，且不得按public/display path猜测。

运行：

```bash
cargo test -p skiff-compiler-projection \
  package_artifact::callables::normalization::tests
cargo test -p skiff-compiler-source package_signature
cargo test -p skiff-compiler --test compiler_owned_std_type_resolution std_http_stream
cargo test -p skiff-compiler \
  official_std_authoring_and_record_writer_are_fixed_and_deterministic
cargo test -p skiff-artifact-identity \
  callable_parameter_return_and_suspend_mutations_change_local_abi_without_throw_set
cargo check -p skiff-compiler-projection -p skiff-compiler-source -p skiff-compiler
git diff --check
```

随后使用临时artifact/store重新生成以下链，不修改其他仓库source或稳定instance：

```text
std
├── http-session ──> track
└── llm-api ───────> llm-providers
```

验收：

- `std.http.stream`公开签名返回exact `std.http.HttpClientStreamHandle` ServiceSymbol；
- fresh std publicSymbols递归raw LocalType为零；
- llm-providers不再报slot 7 ambiguous owners；
- 记录五个fresh build/Local ABI receipt；确认除std外的Local ABI是否保持不变；
- worktree clean，本地commit；不merge/rebase/push，不操作stable/live。

结果写入`P5-F374-package-signature-exact-symbol-owner-result.md`。新Agent执行，不复用F372审计Agent，不派
子Agent。
