# P5-F369 Registry ErrorPayload marker cleanup

状态：Ready（skiff-packages独立consumer；与F368并行）。

## 直接父节点

- `P5-F287-std-error-surface-migration-result.md`
- `P5-F291-open-error-compiler-consumer-checkpoint-result.md`
- 当前生态迁移DAG：`P5-H36-external-ingress-implementation-dag.md`

父节点已冻结语言中不存在`ErrorPayload` marker。本leaf只迁移官方Registry源码残留，不改变Registry的
20-operation ServiceContract或错误业务字段。

## Exact base与范围

- skiff-packages integration：
  `609551f0a65bfcc814ed4c894e4c333b4ffb10f1`
- Skiff toolchain：使用包含本task的
  `/Users/geek/workspace/skiff-phase-05-integration`，返回实际commit/tree。
- 当前精确命中：
  - `registry/pointer_store.skiff`；
  - `registry/model.skiff`。

## 必须完成

1. 两个错误type只删除`implements ErrorPayload`；保留类型、字段、抛出/捕获和序列化行为。
2. skiff-packages production `.skiff`中`implements ErrorPayload`归零；不得添加replacement marker。
3. 使用fresh isolated artifact root bootstrap canonical std并真实发布Registry service package。
4. 验证Registry ServiceContract仍精确20个operation，所有operation Available；运行现有Registry source、
   receipt和脚本聚焦测试，先枚举非零。

允许写入仅为上述两个`.skiff`文件及为精确receipt断言确实需要的Registry局部测试。禁止修改manifest API、
contract surface、其它package、Skiff、Internals、stable/live或共享workflow。下一失败若要求其它owner，
返回`TASK_SCOPE_EXPANDED`。

- worktree：`/Users/geek/workspace/skiff-packages-p5-f369-error-payload-marker-cleanup`
- branch：`codex/p5-f369-error-payload-marker-cleanup`
- production/tests一个commit；clean，不merge/rebase/push。
- 启动5分钟内开始修改；返回exact commit/tree、changed files、非零测试、真实receipt与20-operation证据。
