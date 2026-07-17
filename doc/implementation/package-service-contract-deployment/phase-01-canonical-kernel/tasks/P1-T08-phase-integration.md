# P1-T08：Phase 01 集成与 Gate

## 目标

按DAG合并T01–T07，处理机械冲突、重建fixtures并在最终代码状态执行阶段gate。不得以“集成修复”为名
新增或改变语义。

## 依赖与 worktree

- integration worktree固定为`/Users/geek/workspace/skiff-package-service-phase-01`，branch
  `codex/package-service-phase-01`。
- 完成依赖T03–T07；T01/T02已作为其前置进入checkpoint。
- 本任务只在integration branch提交。

## 集成职责

1. 按T01/T03/T04、T02、T05、T06、T07批次顺序合并；每批后运行最小compile/check再发布下一checkpoint。
2. 冲突只按任务文档owner边界解决。若冲突暴露semantic缺口，退回对应任务Agent，不在T08设计。
3. 使用仓库生成器重建受identity/schema变化影响的fixtures，不手工伪造hash。
4. 检查最终production路径和phase plan所有反向搜索，更新checker allowlist只能附带明确owner/删除阶段。
5. 新增`phase-result.md`记录最终commit、验证证据、已删除临时项和仍保留legacy ledger；不得把未完成条款
   写成follow-up后判定通过。
6. 所有task worktree合入后由主Agent清理；T08不得合并main。

## 最终 gate owner

在候选稳定后只运行一次：

```bash
cargo fmt --all -- --check
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
git diff --check
```

先做廉价structure/search与focused tests，再运行昂贵selector。记录格式：

```text
层级 | 命令 | owner | commit | 结果 | 覆盖范围
```

任何相关代码变化使对应证据失效；只重跑受影响层级。

## 阶段完成检查

- phase plan §1九项全部有代码与测试证据。
- 不存在未合并task branch、dirty task worktree或未解释production legacy命中。
- `PublicationAbiUnit`等允许临时对象没有新增owner/算法，legacy ledger删除阶段仍成立。
- 所有gate PASS，或唯一失败有main同环境可复现证据且聚焦replacement完整覆盖；新回归不能豁免。
- 提交自验收矩阵和phase-result commit，供A01只读验收。
