# P5-T09E：Internals Generated RuntimeAssembly / Workflow Closure

> 2026-07-29 authority correction：本任务旧版要求的Internals root `assembly.yml`已经失效。Internals是
> 松散项目集合；本任务只关闭由watch registry、显式service roots或deployment receipts生成
> RuntimeAssembly的workflow，不增加repo-level manifest或集中式environment owner。

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §3–§5、
  §9–§15。
- 依赖：T07、T08、T10、T11、T12均已合入各自integration branch的exact clean commits；解锁I03，
  I03另等待T06 exact tree后才运行。
- 风险：高；environment完整closure、actual deployment root选择、共享build/reload owner。
- branch：`codex/p5-t09e-internals-final-assembly`；worktree：
  `/Users/geek/workspace/internals-p5-t09e-assembly`。
- 使用新的开发Agent；五分钟内编辑共享assembly workflow。任何前置deployment不可由canonical reader
  解析时，回报`TASK_NOT_EXECUTABLE`，不生成identity placeholder或新增manifest兜底。

当前是Wave 3 implementation checkpoint，不是稳定候选。证据锚定三个前置repo exact commits/trees；
四对象schema/path/CLI、任一contract/package/deployment、assembly script/config变化都会使证据失效。

## 写入范围与完成态

独占`scripts/prepare-canonical-assembly.mjs`的最终direct roots/receipts closure、
`scripts/verify-phase05-ecosystem.mjs`、`scripts/run-phase05-ecosystem-probe.mjs`、对应tests与必要root
script/AGENTS接线。不得新增Internals root config，不得修改任何service
contract/package/deployment/source、Skiff production或`skiff-packages` source。

1. workflow从watch registry、命令显式给出的service roots，或前序步骤产出的exact deployment receipts
   取得root set。最终验收集合包含account、registry、Codex Relay、AIHub、Agine五个deployment；
   resolver从exact records闭合全部package/contract/deployment，不从repo layout猜root，也不读取
   `assembly.yml`。
2. workflow按`select explicit roots -> compile packages independently -> validate deployments ->
   resolve one generated assembly`执行；任何missing/tampered/duplicate provider或partial write不请求
   activation prepare。现有临时`assembly.yml`中转必须删除，直接把typed root refs交给resolver。
3. 五个Host mapping仍由local ingress拥有；workflow/probe可分别验证
   `account.skiff.localhost`、`registry.skiff.localhost`、`codex-relay.localhost`、
   `aihub.localhost`、`agine.localhost`到service/version selector的映射及相同path不歧义，但Host不进入
   root set、RuntimeAssembly identity或closure。
4. `--check --no-reload`在linked worktree只写temporary immutable root并验证完整closure；`--activate`
   受primary-worktree provenance guard保护，写完records后只请求router coordinator执行一次exact
   prepare/admit/commit transaction，不直接写activation state或调用旧reload。
5. 输出包含root来源、四类artifact identities、active environment/generation、
   deployment/contract/package closure与source provenance；不含Publication/common kind/serviceAssembly或
   repo-level assembly manifest。
6. 对五个deployments执行config/state/resource/SecretRef requirement closure；尤其验证AIHub六类secret ref
   均已绑定、值未进入任何artifact或日志，missing ref在activation prepare前失败。
7. `verify-phase05-ecosystem.mjs --non-live`从显式roots/receipts输入去重调度T08/T10/T11/T12的affected
   checks、registry/provider/chat self-tests与五deployment closure；拒绝primary/stable reload路径，供T13
   唯一执行。
8. `scripts/run-phase05-ecosystem-probe.mjs --isolated --replicas 1`消费显式`SKIFF_ROOT`、
   `SKIFF_PACKAGES_ROOT`和五个service roots/receipts，用temporary store/router/runtime/Mongo及fake
   upstream执行I03完成态；task只运行其self-test，不运行actual combined probe。

## 唯一聚焦验证 owner

```bash
P5_CARGO_TARGET="$(mktemp -d /tmp/skiff-p5-t09e-cargo.XXXXXX)"
git -C /Users/geek/workspace/skiff-p5-r02-checkpoint status --short
CARGO_TARGET_DIR="$P5_CARGO_TARGET" \
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-r02-checkpoint \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
  node scripts/prepare-canonical-assembly.mjs --check --no-reload
node --test scripts/prepare-canonical-assembly.test.mjs
node --test scripts/verify-phase05-ecosystem.test.mjs
node --test scripts/run-phase05-ecosystem-probe.test.mjs
git -C /Users/geek/workspace/skiff-p5-r02-checkpoint status --short
git diff --check
```

不运行stable build/dev/start/reload或完整gate。提交一个commit并合入Internals integration，回报root来源、
deployment/closure表、独立Host mapping表、pointer不变负例、provenance与自验收矩阵。
