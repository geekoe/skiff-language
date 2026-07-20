# P4-F05：Eval Callback Projection ABI Integration

## 权威输入、风险与证据状态

- 执行输入：R01在`9eaea40`的唯一新blocking issue；F03把`ServiceLinkableCapabilityHooks`返回类型改为
  `ServiceLinkableCapabilityProjection`，T03 callback lane shell仍实现旧`CallbackCapabilityCarrier`签名，导致
  eval/host E0053。原F02/F03/F04三项已分别判RESOLVED。
- 风险/验收组：中风险机械integration seam；由R01再次复验，不解锁T06。
- integration边界：只提交task branch，不merge integration/main、不push。

## DAG 与执行约束

- 依赖：F02–F04已合流及R01@`9eaea40` FAIL。
- 解锁：R01 retry。
- branch：`codex/p4-f05-eval-callback-projection-abi`。
- worktree：`/Users/geek/workspace/skiff-p4-f05-eval-capability-abi`。
- 五分钟内真实edit；原T03 owner执行，不实现callback/native具体lane。

## 写入范围与完成态

- 只修改`runtime/eval/src/assembly_execution/callback_native.rs`及assembly target/projection的直接机械接线/测试。
- callback shell method签名、import与typed fail-closed返回必须匹配F03 RAII projection ABI；仍不注册/调用真实
  callback，不消费T06业务语义。
- `RuntimeAssemblyEvalTarget::with_request_activation`必须复用既有assembly-wide execution projection（clone/Arc
  共享immutable storage），不得按provider/context切换重新遍历并复制所有package files/resources。
- eval与host过滤器恢复编译并非零PASS；assembly callback canonical executable仍到达typed checkpoint，无legacy
  fallback或router/outbound。

## 唯一验证 ownership

```bash
cargo check -p skiff-runtime-eval -p skiff-runtime-request
cargo test -p skiff-runtime-eval assembly_execution_projection
cargo test -p skiff-runtime-host typed_execution_fixture
git diff --check
```

不得运行具体callback lane或完整runtime gate。

## 回报

提交一个clean commit，回报ABI前后矩阵、projection复用证据、命令结果与remaining blocker。
