# P5-T07：`skiff-packages` Canonical Consumer Cutover

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §1–§3、
  §6.1、§9–§10、§13–§15。
- 依赖：R02 PASS的exact Skiff integration commit；与T06/T08/T09A–C并行，解锁R03。
- 风险：中；pure Package source、direct call、test/store harness。
- repo branch：`codex/package-service-phase-05`；integration worktree：
  `/Users/geek/workspace/skiff-packages-phase-05-integration`。task branch：`codex/p5-t07-skiff-packages`，
  worktree：`/Users/geek/workspace/skiff-packages-p5-t07`。
- 使用主Agent记录的exact Skiff worktree/commit；五分钟内编辑`track.skiff`或harness。
- 当前共享状态是R02 PASS的external-consumer checkpoint；完成后只是Wave 3 partial candidate。使用新的
  开发Agent；证据对exact Skiff CLI/artifact interfaces、package source/harness/fixtures或依赖变化失效。

## 写入范围与完成态

独占 `skiff-packages` repo。四个目录仍只是Package source，不新增contract/deployment/assembly。

1. `track/track.skiff`的7个dependency call改为`httpSession/<publicPath>`；
   `httpSession.HttpSession` contract/package type引用仍使用`.`，不做机械全局替换。
2. harness从exact Skiff root调用canonical package build/test入口，不指向`language/scripts/skiff.mjs`、
   不自己编码publication storage segment，不建旧source-symlink package store adapter。
3. 删除不存在`llm` package的`test:llm`及10组orphan doubles；default/live test disposition保持明确，
   普通`npm test`不访问网络。
4. `aliyunoss`/`http-session`/`openai`不因迁移做无关长文件重构；manifest/API只在
   canonical authoring要求变化时修改。
5. DB fixture使用canonical isolated test lifecycle，不静默改接stable Mongo 27017。

## 唯一聚焦验证 owner

```bash
npm run type-check
npm run test:aliyunoss
npm run test:http-session
npm run test:openai
npm run test:track
git diff --check
```

不跑最终`npm test`或live OpenAI；前者归T13，后者只在V01确有必要且已有本机凭据时执行。
提交一个commit并合入`skiff-packages` integration branch，回报source/harness/fixture反向
搜索、DB lifecycle、测试与自验收矩阵。
