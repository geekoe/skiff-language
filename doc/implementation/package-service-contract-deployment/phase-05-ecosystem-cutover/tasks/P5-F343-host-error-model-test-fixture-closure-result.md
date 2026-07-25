# P5-F343 Host error-model test fixture closure result

状态：`PASS`（runtime-host 旧错误模型测试与夹具已迁移到冻结语义；完整 host suite
`275/275` 通过；无 blocking；未修改 task 状态，未 push，未承接后续节点）。

## 候选与写入边界

- worktree：`/Users/geek/workspace/skiff-p5-f343-host-test-closure`
- branch：`codex/p5-f343-host-test-closure`
- task production 起点 commit：
  `29bcb43adb6647ed87add4dfcbd2212352b96a4a`
- task production 起点 tree：
  `b7bc3697f02f620e4c15ec480feb530189ea628c`
- 实际开发起始 HEAD：
  `0bd80a4fec7bc99e72aaaa9fca070e660f8e7a5f`
- 实际开发起始 tree：
  `40bd9f0dc595549b53a54bbc6b076de22e43953a`

写入仅包含 runtime-host test module/test fixture以及本 result。没有修改 production、
artifact/model/eval/request/transport、Router、telemetry、Cargo/lockfile、设计、父 task/result或
task状态；没有增加 compatibility alias、`#[ignore]`或条件屏蔽。

除任务预列文件外，完整 suite继续揭示并迁移了以下同一组 runtime-host旧 fixture：

- `runtime/host/src/loader/assembly_admission/tests/execution/{resolver.rs,runtime.rs,
  scenario.rs,callback_native.rs}`
- `runtime/host/src/host/router_session/tests/runtime_assembly_request.rs`

## 起点 44 错误矩阵

先保存起点 `cargo test -p skiff-runtime-host --no-run`日志，并按物理文件去重。总计44条
rustc诊断、31个唯一问题；`execution/artifacts.rs`同时被普通 execution test module与
router-session fixture复用，因此其13个问题被报告两次：

| test fixture | 唯一问题 | rustc诊断 | 旧语义类别 |
| --- | ---: | ---: | --- |
| `eval_capability_adapter/error.rs` | 1 | 1 | `TypeIdentity` |
| `capability_context/stream_runtime/tests.rs` | 1 | 1 | `TypeIdentity` |
| `error/tests.rs` | 5 | 5 | `TypeIdentity`；旧 `UserException`构造 |
| `assembly_admission/tests/execution/artifacts.rs` | 13 | 26 | operation error set、callable throw set、discriminator、call/throw source site |
| `assembly_admission/tests/execution/async_stream_cancel.rs` | 3 | 3 | error contract、旧 typed payload访问 |
| `assembly_admission/tests/full_chain.rs` | 4 | 4 | error/throw set、call source site |
| `runtime_assembly_request/fixture.rs` | 4 | 4 | error/throw set、call source site |
| **合计** | **31** | **44** | |

任务盘点的4处旧 `TypeIdentity`均已迁移；部分后续使用因 unresolved import而没有单独形成
rustc诊断。

## 冻结语义迁移

1. Platform catch统一改为有限
   `PlatformBuiltinErrorIdentity::<variant>.catch_identity()`；request-local nominal fixture使用
   当前 `CatchIdentity::Nominal(LocalExecutionTypeIdentity)`。没有恢复旧 alias或任意 builtin
   字符串身份。
2. 所有 `BoundaryOperationContract.errors`、`BoundaryErrorContract`和
   `PackageCallableSignature.throw_types`均从 fixture删除。跨 service需要恢复的
   `AsyncFailure`/`StreamError`与 callback类型只通过 owner package的公开
   `PackageSchemaIndex`、schema type record和精确 implementation link表达；operation没有
   throw/error set。
3. 每个旧 `CallIr`与`StmtIr::Throw`都补了有限
   `InstructionSourceSite::Synthetic(CompilerGeneratedTestHarness)`；没有空来源或虚构源码位置。
4. `TypeDeclIr.discriminator`已删除。generic stream fixture显式传入`T=bool`，不依赖已删除的
   authoring推断；callback fixture补齐与公开 schema精确对应的 interface implementation type。
5. typed resolver现在按 artifact reference重建并校验精确`PackageSchemaIndex` identity；
   full-chain空 index同样 fail closed。request fixture提供合法 trace，使本地 throw生成的
   correlation满足当前 `RequestException`不变量。
6. async unary与service stream真实错误路径均断言
   `UserException(RequestException)`：跨 service仍保留 fixed service carrier，公开可链接类型在
   caller恢复 nominal catch与本地 typed payload，payload/correlation只经 typed API读取。stream
   provider原 generic assert错误已改为公开 nominal `StreamError` throw。
7. callback mapping与缺 capability断言从 display string改为当前 typed platform identity；
   mapping payload message从 caller heap typed value读取。router trusted-test fixture移除不会被
   execution消费的旧 effect double，保留 trusted `test_effects_enabled`进入 canonical
   execution的断言，符合当前 unused-double fail-closed规则。

## 删除或重写的失效断言

以下三个测试只验证已删除 JSON envelope/debug metadata spoofing结构，当前
`RequestException + RuntimeValueCarrier + source/stack/correlation`模型不存在对应合法输入，因此
删除：

- `user_exception_payload_ignores_legacy_metadata_spoofed_http_error_type`
- `user_exception_payload_ignores_legacy_metadata_spoofed_decode_error_type`
- `user_exception_debug_name_remains_display_only_envelope_data`

旧 `user_exception_payload_includes_erased_payload_message`没有简单删除，而是改写为
`user_exception_payload_redacts_local_value_and_exposes_only_correlation`：先通过 typed API确认
request-local payload与 correlation，再确认 generic host payload不泄漏私有 value或本地类型地址，
只投影 correlation。

## Selector、验证与反向证据

最终候选验证：

```text
cargo test -p skiff-runtime-host --no-run
  PASS: 3 test executables built

cargo test -p skiff-runtime-host
  PASS: lib 267 passed, 0 failed
  PASS: active_runtime_assembly 2 passed, 0 failed
  PASS: p5_f340_service_error_host 6 passed, 0 failed
  TOTAL: 275 passed, 0 failed

cargo check -p skiff-runtime-host
  PASS

git diff --check
  PASS
```

完整 selector非零。另对本次全部 Rust写入运行`rustfmt --edition 2021 --check`，结果 PASS。
repo-wide `cargo fmt --all -- --check`仍会命中任务写入边界外3个既有未格式化文件：
`compiler/driver/authoring/package_publication/tests.rs`、
`compiler/tests/service_conformance.rs`与`compiler/tests/websocket_ingress.rs`；没有把这些
无关格式化变化保留进候选。

反向搜索 runtime-host test范围，以下旧 symbol/field/accessor均为0命中（负向文本 fixture除外）：

```text
TypeIdentity
BoundaryErrorContract
BoundaryOperationContract.errors
PackageCallableSignature.throw_types
TypeDeclIr.discriminator
UserException::{from_typed_payload,from_envelope}
UserException::{error_payload,envelope}
```

没有运行 workspace/root、stable或 live验证。

## Blocking

Blocking：无。

P5-F340记录的44条 test-only consumer断点已全部关闭；production与fixed service carrier语义未被
改写。完整 runtime-host suite与既有P5-F340 6项回归现均通过。本任务不表示后续 gate或 Phase 5
完成。
