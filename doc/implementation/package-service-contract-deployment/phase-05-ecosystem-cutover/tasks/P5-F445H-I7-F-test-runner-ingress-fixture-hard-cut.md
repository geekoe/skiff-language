# P5-F445H-I7-F Test-runner ingress fixture hard cut

状态：

```text
PASS
```

## 1. Parent and baseline

- authority:
  `doc/architecture/package-service-contract-deployment.md`
- direct design parent:
  `P5-F445H-I7-D0-service-scoped-ingress-design-result.md`
- canonical parent:
  `P5-F445H-I7-K-service-scoped-ingress-canonical-result.md`
- compiler consumer parent:
  `P5-F445H-I7-C-compiler-ingress-consumer-result.md`
- baseline commit/tree:
  `b9aaed250d23f522165136a4cfa35b127d0c8826` /
  `758fc89311b2f7bbfb8f5d9115eb9aa99d78652d`
- branch:
  `codex/p5-f445h-i7-f-ingress-fixtures`
- worktree:
  `/Users/geek/workspace/skiff-p5-f445h-i7-f-ingress-fixtures`
- integration owner:
  `/root/phase05_integration_steward`

## 2. Scope

本任务迁移`test-runner`拥有的测试夹具、golden和控制请求编码：

- 删除`IngressSelector.host`构造与旧Host authoring fixture；
- package-test control request使用service-local selector，并携带精确
  `ServiceDeploymentRef`；
- HTTP URL只保留合法request authority，不把authority当作路由身份；
- 将当前正向fixture更新到DeploymentArtifact v4、RuntimeAssembly v3和runtime frame v2；
- 保留明确用于验证旧代际拒绝的previous/legacy负例。

聚焦source-receipt测试还直接编译以下非live示例service source fixture，因此同一机械hard cut允许只从
这些`http.yml`删除旧Host字段，不修改其handler、method、path或运行逻辑：

```text
runtime/encrypted-storage-live/default-service/http.yml
runtime/encrypted-storage-live/mapped-service/http.yml
runtime/live-tests/http.yml
```

首要编译断点：

```text
test-runner/src/ecosystem_smoke_fixture.rs
test-runner/src/package_test_assembly.rs
test-runner/src/runtime_execution.rs
```

允许关闭同一`test-runner` owner内的同类旧fixture和上述直接消费的示例source fixture。禁止修改compiler生产代码、
Router、Runtime Host、assembly resolver/loader/linker或其他consumer生产实现。

## 3. Required evidence

1. baseline真实编译失败，证明上述fixture仍引用已删除字段或缺少新wire字段；
2. `cargo check -p skiff-test-runner`通过，并证明Host编译越过本任务三处断点；
3. test-runner相关聚焦测试通过；
4. `cargo fmt --all -- --check`与`git diff --check`通过；
5. 反向搜索确认：
   - test-runner正向fixture不再构造`IngressSelector.host`；
   - 正向wire不再使用runtime frame v1、DeploymentArtifact v3或RuntimeAssembly v2；
   - 旧代际字符串只存在于明确拒绝旧输入的负例。

不得运行stable/live/network/Mongo/OAuth/browser，不得push。
