# P2-T02：Sound Effect / Provenance Analysis

## 目标

在 compiler source 建立 sound conservative may-effect/provenance fixed point，替换所有 callable 永久
`AnalysisPending` 的占位。精度不是目标；任何不能证明安全的路径必须保守拒绝 boundary。

## 依赖与 worktree

- 依赖 T01 checkpoint。
- 建议 branch：`codex/package-service-p2-t02-effect-provenance`。
- 可与 T03、T04 并行。

## 完成态

1. 从现有 name/type/call resolution 提炼 typed call-target facts；effect analyzer不重解AST display path。
2. 为每个 callable建立local seed，至少覆盖caller-reachable write、return alias、throw alias、escape、
   same-heap identity、unknown/dynamic/native target、suspend。
3. 对递归 call graph按SCC/fixed point传播；调用 package dependency时只导入canonical callable facts，不
   读取provider service/deployment。
4. 参数映射、return/throw、callback/stream capture、spawn/DB/queue/native/external target保守处理。无法
   证明的callee为Unknown或显式unsafe may facts，不能填false。
5. 简单纯primitive/detached-data wrapper产生Analyzed安全事实，使Phase 02至少存在可用boundary callable。
6. source到compiled/projection-input handoff继续使用稳定 executable key，不依赖display name。
7. `AnalysisPending` 只允许出现在明确无法分析的测试/诊断路径；正式成功的package compile对所有API
   callable有Analyzed或结构化Unknown原因。
8. 新分析器按call graph、transfer、provenance、tests拆文件；不扩大现有千行文件。
9. 本任务是波次 2 唯一 `compiler/source/**` owner：负责用 T01 carrier 表达 contract operation target；不读取
   provider artifact，也不分配 runtime binding slot。

## 写入范围

- T01 checkpoint之后的全部 `compiler/source/**` 改动，包括effect/provenance/call-target模块、必要的source
  model字段、最小facade export及直接tests。
- 不修改 artifact wire、compiler input/lowering、projection/emission或driver。

## 验证

```bash
cargo test -p skiff-compiler-source
cargo test -p skiff-compiler-compiled -p skiff-compiler-projection-input
git diff --check
```

聚焦测试必须覆盖direct/transitive、recursive SCC、参数write、return/throw alias独立、escape、callback/
stream/spawn/DB/native/unknown、跨package导入和简单safe wrapper。

## 回报

提交commit、自验收矩阵、每个effect字段的保守规则、仍返回Unknown的语法/target及原因。若某个false无法
给出证明，任务FAIL而不是降精度通过。
