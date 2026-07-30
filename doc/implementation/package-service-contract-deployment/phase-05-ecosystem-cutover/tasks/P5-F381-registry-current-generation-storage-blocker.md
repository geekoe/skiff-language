# P5-F381 Registry current-generation storage blocker

状态：TASK_SCOPE_EXPANDED（实现checkpoint完成；动态测试被Skiff test fixture阻断）。

## 已完成的scoped checkpoint

- worktree：
  `/Users/geek/workspace/skiff-packages-p5-f381-registry-current-generation-storage`
- branch：
  `codex/p5-f381-registry-current-generation-storage`
- commit：
  `8b15fcd2c2cadc964eb0ffa1575cd519c9eeb65f`
- tree：
  `4bf21fb53e040cd064e4ed6fee21e0561c373599`
- worktree clean；尚未合入skiff-packages integration。

三个changed files已经完成：

- 四类immutable current generation：
  PackageArtifact `v7/v8/v6`、ServiceContract `v4`、ServiceDeployment `v2`、
  RuntimeAssembly `v2`；
- 四类pointer均有fresh identity、两次CAS、read和ascending history；
- 保留content conflict、malformed identity、CAS mismatch、candidate/release mismatch与非法limit负例。

Registry source `5/5`、fresh receipt `1/1`、combined authoring `6/6`通过；20 roots/receipt IDs/contract
operations/deployment bindings保持`20/20/20/20`。

## 动态阻塞

隔离Router已经越过F378并成功启动，但8个Registry runtime cases一个也未执行。test-runner在fixture
assembly阶段fail closed：

```text
canonical test fixture failed: invalid canonical fixture:
package-test ingress is not yet migrated to deployment gateway entries
```

精确owner：

- `test-runner/src/package_test_assembly.rs:77-80`
- 同文件`:239-252`

后继必须先迁移Skiff package-test canonical fixture；完成后从clean F381 checkpoint重新运行真实
`npm run test:registry`，8个runtime cases全部通过前不得合流。
