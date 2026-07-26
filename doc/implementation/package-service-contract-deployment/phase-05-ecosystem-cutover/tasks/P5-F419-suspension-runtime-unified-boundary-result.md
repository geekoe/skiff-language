# P5-F419 Suspension runtime unified boundary result

状态：Complete（N3 implementation；combined focused gate待integration owner执行）。

N3 已在 `runtime/**` 完成 service boundary lane统一、deadline逐跳暴露、unary / stream
cancel-deadline-provider竞争、旧 protocol summary consumer删除，以及F415 exact 13个mapping fixture补债。
本节点按并行开发约束停留在独立分支；需要经过 compiler N1与deployment N2合并后的selector仍被当前
base中的旧消费者挡住，因此不宣称workspace或ecosystem稳定。

## 1. 锚点、提交与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| integrated N0 checkpoint | `c597e3c0e5ecb9d1711b1a25a2660ea9cc972a60` | `715ef42385e58b518e278ef082d78d0ed32b6f79` |
| N0 implementation | `57d0a5551aaa62e5a71655050478c1447f94324d` | `a1035bfa02fa745368d5bcd6d8ebbc3d9b54722b` |
| accepted F415 | `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d` | `a2a10789acfc53f190abefcf02447ccdbb598b80` |
| N3 implementation parent | `16c17b7d020d90ff5c97ad314f4ceeceaaa363c6` | `fae922e19dbb8ada9ae513b3b0c861c06adf6f2f` |
| N3 implementation | `615a1ee055a72de5e660cde578e9aafbd02d7e91` | `bbe5a443044c030e21d010abae4139ba2358e8d6` |

三次 ancestry gate均成功：

```text
c597e3c0e5ecb9d1711b1a25a2660ea9cc972a60 ancestor=yes
57d0a5551aaa62e5a71655050478c1447f94324d ancestor=yes
7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d ancestor=yes
```

implementation commit：

```text
615a1ee055a72de5e660cde578e9aafbd02d7e91
runtime: unify service suspension boundary
```

该提交修改27个文件（799 insertions / 564 deletions），全部位于 `runtime/**`。没有修改
artifact-model、artifact-identity、compiler、deployment、router、scripts、test-runner、
cross-system fixture、ecosystem source或设计；没有 merge、rebase、push，也没有访问stable/live。

## 2. Unified boundary数据流

终态调用链：

```text
dispatch_in_process_boundary
  ├─ Unsupported stream -> typed RuntimeError::Unsupported
  └─ Unary | ServerStream
       -> async_stream_cancel::execute_service_call
            -> initial execution-budget poll
            -> unsupported callback typed gate
            ├─ Unary
            │    -> canonical parameter materialization
            │    -> owned provider activation
            │    -> biased wait(cancel, deadline, provider)
            │    -> canonical result/error materialization
            └─ ServerStream
                 -> open RequestStreamLease before parameter materialization
                 -> capture OwnedExecutionControl + owned provider context
                 -> detached ProviderStreamTask
                 -> item / terminal / terminal-publication biased waits
                 -> cancel request + stream on cancel/deadline
                 -> task guard and stream lifetime release
```

`ordinary.rs` 现在只保留 `execute_package_direct`；service executor与
`validate_ordinary_operation` 已删除。Unary与ServerStream不再按contract cancellation或
`may_suspend` 分叉。Ready unary provider仍是select中的provider分支，可以在第一次poll直接返回；
没有增加synthetic yield。

Host full-chain fixture把 code-free operation contract与concrete executable summary分开保存。
同一unary contract分别用provider concrete `may_suspend=false/true` admission并执行同一boundary
lane；callback owner concrete summary也可与code-free callback shape独立变化。

## 3. Cancel、deadline与provider竞争

`deadline() -> Option<std::time::Instant>` 已逐跳加入：

```text
ExecutionControlApi / ExecutionControl
OwnedExecutionControlApi / OwnedExecutionControl
  -> runtime/request ExecutionBudget + borrowed/owned control
  -> runtime/host eval capability adapter borrowed/owned views
  -> eval test double
```

request borrowed、owned、owned再borrow的视图返回同一个exact `Instant`。detached stream task和
boundary stream sink都保存 `OwnedExecutionControl`，不只保存raw cancellation token。

等待顺序均使用 `tokio::select! { biased; ... }` 和
`tokio::time::sleep_until(tokio::time::Instant::from_std(deadline))`：

| wait | priority |
| --- | --- |
| unary provider | request cancellation → deadline → provider |
| stream terminal | consumer cancellation → request cancellation → deadline → provider |
| stream item publication | request cancellation → deadline → provider publication |
| stream terminal publication | consumer cancellation → request cancellation → deadline → publication |

deadline分支调用既有typed execution-budget projection，结果保持
`RuntimeError::ExecutionBudgetExceeded { reason: DeadlineExceeded, ... }`，同时cancel provider
request。若deadline分支选中后才到达cancel，fallback仍固定为typed `DeadlineExceeded`，不会降成
`Cancelled`；同时ready时则因biased顺序由cancel获胜。

新增测试证据覆盖：

- Ready unary初次poll返回；
- pending unary分别由provider、request cancel、deadline唤醒；
- cancel + expired deadline + ready provider时cancel优先；
- deadline typed error与provider request cancel signal；
- stream terminal、item、publication的deadline typed结果；
- stream item/publication的cancel-over-deadline顺序；
- request cancel与deadline后的provider task、stream registry和lease清理；
- provider task counter exact-once guard。

## 4. 旧 protocol summary consumer与保留检查

已从runtime copy、accessor、branch和fixture删除：

```text
BoundaryCancellationContract
BoundaryOperationContract.cancellation
BoundaryOperationContract.may_suspend
CallbackContractOperationProjection.may_suspend
```

反向搜索在 `runtime/**` 对上述owner/访问路径返回零。没有给contract/schema补default、alias或
兼容分支。

callback只删除 executable summary与contract operation summary equality，仍保留：

- exact callback contract / schema record / owner identity lookup；
- receiver call ABI与method slot映射；
- parameter数量、parameter shape与return shape；
- exact native adapter preimage与declared operation bounds。

WebSocket只删除 `executable.may_suspend` equality，仍保留：

- pinned contract operation lookup；
- no package-local type params；
- exact parameter count/name/shape；
- return shape与package-owned Context record identity。

WS测试把concrete executable summary从false改为true后仍接受，随后shape mutation仍拒绝；
callback full-chain fixture把owner summary改为true后仍到达exact owner method。native selector的
mapping / package identity / ABI mismatch负例全部继续通过。

以下HTTP gateway concrete checks原样保留且对应文件无diff：

```text
runtime/request/src/http_gateway_target.rs
  linked.may_suspend == signature.may_suspend
runtime/eval/src/runtime_http_gateway.rs
  resolved.executable.may_suspend == callable.signature.may_suspend
```

actor、native、builtin、ExecutableIR、CallableMayEffects、Package/publication callable等concrete
summary owner也未删除。

## 5. F415 exact 13个mapping fixture

| file | 新增initializer | 取值 |
| --- | ---: | --- |
| `ordinary/tests/service_error_consumer.rs` | 4 | exact empty |
| `ordinary/tests/source_inline_effect_e2e.rs` | 3 | 两个exact empty；动态binding clone同edge requirement mapping |
| `ordinary/tests.rs` | 4 | exact empty |
| `service_error_channel/tests.rs` | 2 | exact empty |
| 合计 | 13 | `4 + 3 + 4 + 2` |

同一dynamic dependency edge的binding使用
`requirement.collection_name_mapping.clone()`；其余fixture确实没有mapping，显式使用
`BTreeMap::new()`。没有给model struct增加default或删除字段。

以下production owner `git diff --exit-code` 为PASS，exact validation / projection原样保留：

```text
runtime/linked-program/src/shared_image.rs
runtime/linker/src/assembly.rs
runtime/loader/src/runtime_assembly/graph_validation.rs
runtime/host/src/loader/active_assembly_context.rs
```

静态反搜仍可见requirement-vs-binding equality、mapping drift、unknown source与active edge
projection owner。

## 6. 验证证据与实际计数

Cargo命令使用共享target：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

### 6.1 可独立运行的listing与测试

| selector / command | listing | execution |
| --- | ---: | --- |
| `skiff-runtime-capability-context execution_control` | 1 test / 0 benchmarks | 1 passed / 0 failed |
| `skiff-runtime-model callback_projection` | 3 tests / 0 benchmarks | 3 passed / 0 failed |
| `skiff-runtime-native callback_adapter` | 7 tests / 0 benchmarks | 7 passed / 0 failed |
| supplemental `skiff-runtime-boundary service_value_plan` | 未单独listing | 14 passed / 0 failed / 167 filtered |
| `cargo check -p skiff-runtime-capability-context -p skiff-runtime-model -p skiff-runtime-native` | — | PASS |
| `cargo fmt --all -- --check` | — | PASS |
| `git diff --check` | — | PASS |

### 6.2 当前并行base的out-of-scope阻断

按任务要求先执行各selector的 `-- --list`。以下target在生成listing前就被N1/N2旧消费者阻断，
因此没有把D93基线计数冒充本节点实际计数，也没有在listing失败后运行对应selector：

| selector | listing结果 / first error |
| --- | --- |
| `skiff-runtime-request execution_budget` | BLOCKED：`deployment/src/projection/eligibility.rs:2:31` E0432 unresolved `BoundaryCancellationContract` |
| `skiff-runtime-eval assembly_execution` | BLOCKED：`compiler/core/src/package_interface_methods.rs:50:9` E0026 deleted `InterfaceMethodSignature.may_suspend`；并发还报告deployment E0432 |
| `skiff-runtime-linker assembly` | BLOCKED：deployment `eligibility.rs:2:31` E0432 |
| `skiff-runtime-loader runtime_assembly` | BLOCKED：deployment `eligibility.rs:2:31` E0432 |
| `skiff-runtime-host assembly_admission` | BLOCKED：compiler/core `package_interface_methods.rs:50:9` E0026 |

compiler/core同一旧consumer还报告：

```text
package_interface_methods.rs:71:9   E0560 initializer may_suspend
package_interface_methods.rs:199:9  E0560 initializer may_suspend
package_interface_methods.rs:244:9  E0560 initializer may_suspend
package_interface_methods.rs:244:29 E0609 access method.may_suspend
```

deployment同一旧consumer还报告：

```text
eligibility.rs:58:40  E0609 contract.may_suspend
eligibility.rs:302:18 E0609 contract.cancellation
```

required八package `cargo check`同样在
`deployment/src/projection/eligibility.rs:2:31` 首错退出。上述文件均在本任务禁止写入范围，
且正由并行N1/N2 owner迁移；本分支没有越权修补、merge或cherry-pick。integration owner需要在
N1/N2/N3汇合后重新执行五个未列出selector及required八package check。

### 6.3 静态验收

| 检查 | 结果 |
| --- | --- |
| 三个required ancestor | PASS |
| 改动只在implementation的 `runtime/**` | PASS（27 files） |
| old runtime protocol summary owner/access reverse search | PASS（0 matches） |
| exact mapping initializer diff count | PASS（13；4/3/4/2） |
| 四个production mapping owner unchanged | PASS |
| HTTP gateway concrete summary equality仍存在且文件unchanged | PASS |
| callback/WS shape、target、ABI checker仍存在 | PASS |

## 7. 未运行项与交付边界

未运行：

- 被N1/N2旧消费者挡住的request、eval、linker、loader、host实际selector；
- N1/N2/N3 combined focused gate（由integration owner执行）；
- workspace/full isolated；
- stable instance、service watch、chat smoke或任何live验证。

没有清理共享Cargo target，没有改stable instance配置，没有merge/rebase/cherry-pick/push。

结论：P5-F419 N3 runtime implementation已完成并提交；独立可运行的11个required selector tests与
14个supplemental boundary tests通过，静态owner/mapping验收通过。剩余验证项是已精确定位的并行
N1/N2 integration gate，不是扩展本节点production scope的理由。
