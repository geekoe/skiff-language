# P5-F445H-I7-G generic `std.json.encode` RED result

状态：

```text
G_HERMETIC_PROBE=PASS
GENERIC_ENCODE_RED_REPRODUCED=NO
M4_GENERIC_ENCODE_ATTRIBUTION=NOT_PROVEN
PRODUCTION_CHANGED=NO
```

## 1. 冻结输入与写集

| 项目 | Commit / tree |
| --- | --- |
| Skiff baseline | `5c0f8222972e4612224e0660e88e6054874ddd03` |
| Skiff baseline tree | `cf98566873d974a63a9759a2856ecc28efbde5a4` |

本任务只新增 hermetic test、注册该 test module，并新增 task/result 文档。没有修改 compiler、
linker、Eval、native dispatcher 或其它 production；没有读取或修改 Internals。

## 2. 真实链路

fixture 从两个真实 package source 开始：

- model package 公开 `PublicPayload` 与 `makePublic`；
- consumer package 定义 private `LocalPayload` 与下面的 AIHub 等价 wrapper：

```skiff
function encodeJson<T>(value: T) -> Json {
  return std.json.decode<Json>(std.json.encode<T>(value))
}
```

consumer 经过 production compiler，model 经过 canonical package records 与 dependency pointer；
随后 compiler-produced consumer artifact 与完整 dependency closure 一起进入 runtime assembly
linker 和 `Interpreter::execute_runtime_assembly_addr`，没有手写 call target。

测试同时锁定以下链：

1. consumer File IR 中 `std.json.encode<T>` 是 exact std package callable
   `pkg-callable:skiff.run/std:std.json.encode`，`T0 = TypeParam(T)`；
2. dependency package 的 caller concrete T 保持 exact `PackageSymbol`，没有退化为 structural
   record；
3. linker 把 exact std callable 解析为 `PackageDirect`，其 `T0` 仍为 `TypeParam(T)`；
4. std package 的 compiler-generated wrapper 再调用 native target
   `bindingKey = std.json.encode`，其 `T0` 仍为 `TypeParam(T)`；
5. Eval 从 caller generic call substitutions 闭合这两个 wrapper 层后，三种输入均成功到达 JSON
   dispatcher。

## 3. 结果

同一条 hermetic execution 依次证明：

| Case | 结果 |
| --- | --- |
| direct concrete `std.json.encode<string>` | GREEN |
| generic `std.json.decode<T>` | GREEN |
| `encodeJson<LocalPayload>` private nominal record | GREEN |
| `encodeJson<models.PublicPayload>` dependency package symbol | GREEN |
| `encodeJson<Array<LocalPayload>>` nested container | GREEN |

没有任何 case 产生：

```text
unsupported native target std.json.encode
```

因此不能把 M4 的同文错误直接归因为“普通 generic `std.json.encode<T>` 无法完成 runtime
substitution”。当前 probe 没有进入 `RuntimeNativeInvocation::require_plan` 的缺 plan 错误路径，
所以本任务不冻结 production owner，也不授权修改
`runtime/eval/src/native_invocation.rs`、`runtime/native/src/dispatch/json.rs` 或
`runtime/native/src/dispatch/invocation.rs`。

## 4. 下一次 isolated 诊断要求

下一任务必须从 M4 的 exact AIHub 52-case isolated 命令中收窄到首个出现该错误的 test case，
并在同一 frozen Skiff/Internals/artifact identity 上采集一次性诊断日志。日志至少包含：

1. 出错 case 名、完整错误栈和 current `ExecutableAddr`；
2. `resolve_runtime_native_invocation_in_type_view` 的 target name、binding key、原始
   `call.type_args`、`env.type_substitutions`、normalize 后的 `T0`；
3. `resolve_runtime_native_call_plan` 的成功/失败，以及 encode 专属 `None` fallback 的原始错误；
4. 若最终来自 `RuntimeNativeInvocation::require_plan`，记录调用它的 dispatcher/adapter，而不能只
   记录最终字符串；
5. failing artifact 中 caller → std `PackageDirect` → std native wrapper 的 exact package build、
   File IR identity、callable ID 与 type arguments。

只有该日志证明 `T0` 在真实 AIHub 链路丢失，才能建立新的 production RED 并冻结最小 owner。
如果 `T0` 已闭合，则应转查 test-effect/native adapter、旧 artifact/runtime binary 或其它要求 plan
的消费路径，不能修改 generic encode substitution。

## 5. 验证

```text
cargo test -p skiff-runtime-eval compiler_linked_generic_std_json_encode_closes_the_concrete_runtime_plan -- --nocapture
  PASS: 1 passed, 0 failed; 406 filtered out

cargo check -p skiff-runtime-eval --tests
  PASS（只有 baseline 已有 warnings）

cargo fmt --all -- --check
  PASS

git diff --check
  PASS
```

未运行 stable instance、MongoDB、网络、live provider、browser、Internals isolated 或完整 workspace
gate；这些都不属于本 test-only 归因任务。
