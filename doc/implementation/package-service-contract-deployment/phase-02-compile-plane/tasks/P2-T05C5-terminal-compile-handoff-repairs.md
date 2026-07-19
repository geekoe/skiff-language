# P2-T05C5：Terminal Compile Handoff Repairs

状态：checkpoint repair tail；由 `84359ab` production probe 升级，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“不变量”
“Compiler 与 Projection 流水线”“Fail-closed 条件”“非目标”。

## 背景与 ownership

合流后的 `cargo check -p skiff-compiler` 只剩两个与并行 cleanup 有关的窄断链：input 仍消费已删除的
`RawSourceOrigin`/`RawSourceFileMeta.origin`；config requirement projection 仍构造已删除的
`ProjectionError::ContractValidation`。

- 独占 `compiler/input` 中 package source origin/service raw-source reader cleanup及直接 tests。
- 独占 `compiler/projection/src/package_artifact/config_requirements.rs` 的准确 error mapping及直接 tests。
- 禁止修改 input-model、source/lowering/core、artifact wire、driver、其它 projection/emission、checker、
  runtime 与 compiler integration tests。

## 完成态

1. package-only raw source不再携带或校验 service/package origin enum；package visibility、module path与文件
   完整性校验保持 fail closed。
2. 无 production caller 的旧 service `read_publication_sources` reader/module/re-export 物理删除，不以 wrapper
   或 compatibility enum 保留。
3. config requirement 投影使用现有 `ProjectionError::InvalidPackageArtifact`，保持结构化 package context；
   不恢复 `ContractValidation` variant。
4. 两类旧 symbol 反向搜索归零，且不改变 R04 config shape语义。

## 验证

- input 与 projection 聚焦测试、`cargo check -p skiff-compiler`；若只被 T05C4 package-call target前置阻断，
  记录精确诊断。
- targeted rustfmt、production 反向搜索、`git diff --check`。
- 不运行 compiler integration tests或 T07 完整 gate。

提交并保持 worktree clean；回报删除 reader/error mapping、测试证据和剩余 compile blocker。
