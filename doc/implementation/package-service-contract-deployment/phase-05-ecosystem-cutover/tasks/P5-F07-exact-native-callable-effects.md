# P5-F07：Exact Native Callable Effects

## 输入、owner与限制

- 依赖：D08完成且F06/R06 PASS；与F06共享`call.rs/tests.rs`，必须串行。独立worktree/branch，一个clean commit，
  不merge/push。
- owner覆盖shared native semantics registry、compiler exact native target/effects mapping与runtime registry一致性校验；
  不改native handler实现、artifact/wire/projection eligibility、F04 fixture或root lock。
- 不按symbol写fixture分支，不从namespace/signature/context/handler存在性推断安全，不全native放行；descriptor缺失
  永远Unknown。

## Shared descriptor

在`artifact-model/src/native_signature.rs`与现有exact `STD_NATIVE_SIGNATURES`同owner新增稀疏
`NativeCallableSemantics` registry，以binding key唯一索引，明确记录mutation、alias、escape、same-heap、unknown、
`may_suspend`与neutral detached return provenance。首批仅包含：

- `std.string.isAsciiDigits`
- `std.string.truncateUtf8Bytes`
- `std.string.encodeQueryComponent`
- `std.string.encodePath`

四项所有mutation/alias/escape/same-heap/unknown为false，`may_suspend=false`，return provenance为Fresh。registry
validator必须拒绝unknown/duplicate key、signature缺失、非`RequiredContext::None`及无真实runtime handler的entry。

## Compiler / runtime消费

1. source target新增`NativeFunction { binding_key }`；只有exact native declaration + binding key进入该variant，raw native
   definition、custom/unknown或动态value仍按原Local/Unknown路径fail closed。
2. callable-effects仅在exact key命中shared descriptor时生成known state；缺descriptor的crypto与所有
   sleep/file/http/telemetry等capability native保持Unknown/RequiresSameHeap。
3. compiled projection/lowering继续产生现有native invocation与exact binding key，不改变FileIR/artifact shape；新增
   variant不得被误降为package/local call。
4. runtime native-contract/table启动或测试时交叉校验descriptor⊂signature、context none、handler registered；
   不修改handler行为。

允许文件限于shared native signature/exports、compiler resolved targets/builder/callable transfer/direct tests、必要
compiled/lowering exhaustive mapping、runtime native-contract/table registry校验与直接tests。

## 正负测试与gate

```bash
cargo test -p skiff-artifact-model native_callable_semantics
cargo test -p skiff-compiler-source exact_context_free_native_uses_shared_callable_semantics
cargo test -p skiff-compiler-source missing_or_capability_native_semantics_remain_fail_closed
cargo test -p skiff-compiler --test std_package_imports truncate_utf8_bytes_projects_available
cargo test -p skiff-runtime-native native_callable_semantics_registry
cargo test -p skiff-runtime-native truncate_utf8_bytes
git diff --check
```

每个filter必须非零。负例覆盖unknown/duplicate/fake capability descriptor、crypto无descriptor、custom native、raw
native definition及原fail-close回归。回报descriptor→source facts→lowering binding→runtime handler矩阵、exact
source/commit/tree、single commit/clean/lock。R07 PASS后F04原命令/fixture/gate必须原样重跑。
