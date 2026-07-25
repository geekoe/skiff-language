# P5-F343 Host error-model test fixture closure

状态：Ready（test-only）。

## 直接父节点

- H consumer实现与完整 suite精确断点：
  `P5-F340-service-error-host-consumer-result.md`
- 当前错误模型与 shared checkpoint复验：
  `P5-F339-response-error-schema-reacceptance-result.md`

父节点已沿引用链连接唯一权威设计。本任务只把 runtime-host旧测试迁移到已经冻结的开放错误通道、
`CatchIdentity`、typed throw source和 fixed service carrier；不得增加兼容 API或改 production。

## 起点与当前失败

- 起点 commit：`29bcb43adb6647ed87add4dfcbd2212352b96a4a`
- 起点 tree：`b7bc3697f02f620e4c15ec480feb530189ea628c`
- `cargo check -p skiff-runtime-host`与 F340 focused 6/6均 PASS。
- `cargo test -p skiff-runtime-host --no-run`当前有44条 test-only编译错误，归并为：
  - 旧`TypeIdentity`4处；
  - 已删除`UserException::{from_typed_payload,from_envelope}`及`error_payload/envelope`断言；
  - 已删除`BoundaryErrorContract`与`BoundaryOperationContract.errors`；
  - 已删除`PackageCallableSignature.throw_types`；
  - 已删除`TypeDeclIr.discriminator`；
  - `CallIr`与`StmtIr::Throw`缺冻结后的必填 typed `site`。
- 同一个`execution/artifacts.rs`被两个 test module path复用，因此诊断会重复；不得复制出第二 fixture。

## 语义迁移规则

1. 不重新引入`TypeIdentity`alias；platform catch使用
   `PlatformBuiltinErrorIdentity.catch_identity()`，local/package nominal使用当前`CatchIdentity`。
2. 不重新引入 error/throw sets；删除 fixture中的 operation `errors`与 callable `throw_types`。
   需要跨服务保留名义类型的 fixture，应通过类型 owner的公开 schema事实表达，而不是 operation声明。
3. 所有 throw/call fixture补真实或有限 synthetic `InstructionSourceSite`；不得填无来源的空值。
4. 旧`UserException` JSON envelope/typed-payload构造必须改为当前
   `RequestException` + `RuntimeValueCarrier` + source/stack/correlation模型，或删除已经专门验证旧
   metadata spoofing/旧 envelope结构且与当前语义无效的测试。
5. 更新断言到当前原则：
   - package内部任意 nominal value可抛；
   -跨 service只由 fixed envelope承载；
   - public可链接类型可在 caller恢复 nominal catch；
   -私有/未链接类型不通过旧 `UnhandledServiceError` display/details泄漏；
   -没有 operation throw set。
6. async typed-error真实路径不得仅为了编译改成 generic payload；应断言当前
   `UserException(RequestException)`或`FixedServiceFailure`的实际合法结果，并验证 payload/correlation
   通过 typed API取得。若 fixture已经被更专门的 error-channel测试覆盖，可删除重复旧断言并在 result说明。

## 写入边界

只允许修改以下 test代码；即使文件位于`src`下也不得改非`#[cfg(test)]` production段：

- `runtime/host/src/eval_capability_adapter/error.rs`的 test module；
- `runtime/host/src/error/tests.rs`；
- `runtime/host/src/capability_context/stream_runtime/tests.rs`；
- `runtime/host/src/loader/assembly_admission/tests/execution/{artifacts.rs,
  async_stream_cancel.rs}`；
- `runtime/host/src/loader/assembly_admission/tests/full_chain.rs`；
- `runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs`；
- 若编译继续揭示同一 runtime-host旧 fixture，可修改该精确 test文件并在 result列明。

禁止修改任何 production、artifact/model/eval/request/transport/Router/telemetry、Cargo/lockfile、设计与父
task/result。不得用 `#[ignore]`、条件屏蔽或删除整个测试模块绕过。

## 验证

先保存当前44错误的唯一文件/类别矩阵，再迭代到：

```bash
cargo test -p skiff-runtime-host --no-run
cargo test -p skiff-runtime-host
cargo check -p skiff-runtime-host
git diff --check
```

完整 suite selector必须非零并报告通过数量。反搜上述已删除 symbol在 runtime-host test production范围内
归零（负向文本 fixture除外）。不得运行 workspace/root/stable/live，不 push。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f343-host-test-closure`
- branch：`codex/p5-f343-host-test-closure`
- 新的一次性开发 Agent；
- 新增`P5-F343-host-error-model-test-fixture-closure-result.md`，写明每类旧语义如何迁移、删除了哪些
  已失效断言及原因、full suite数量与剩余 blocker；
- 提交并返回 commit，不修改 task状态，不承接后续节点。
