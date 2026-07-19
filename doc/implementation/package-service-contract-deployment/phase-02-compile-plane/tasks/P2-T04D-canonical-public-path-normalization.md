# P2-T04D：Canonical Public-path Normalization Consumer

## 目标

删除PackageArtifact export link中的第二套std public-path normalization，使compiled handoff与projection
export target共同消费T04B既有canonical owner。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package-local ABI 与 Service ABI”及
“Compiler 与 Projection 流水线”章节。

## 依赖与写域

- 依赖T04B、T04C及F09D finding。
- 独占`compiler/projection-input/src/package_callable_signatures.rs`的canonical helper可见性边界、
  `compiler/projection/src/package_artifact/export_links/**`及直接测试。
- 不修改source、lowering、compiled semantic facts、artifact schema、identity或integration fixtures。

## 完成态

1. `export_links`直接消费`canonical_package_public_path`或无逻辑委托，不再本地复制std package/path分支。
2. std与非std public path行为、compiled handoff key、export target和现有fail-closed检查保持。
3. 反向搜索production path只有一个normalization规则owner；不新增compatibility、fallback或第二种路径格式。

## 聚焦验收

- 运行projection-input与export-links的std/non-std路径聚焦测试、最小check、changed-file rustfmt和
  `git diff --check`；不运行R10I、F09D或宽gate。

## 执行合同

- DAG：波次9h projection consumer repair；与T03H2并行，二者共同解除R10I/F09D复验。风险：中。
- worktree：`/Users/geek/workspace/skiff-p2-t04d-public-path-owner`；分支：
  `codex/p2-t04d-public-path-owner`；从F09D失败候选创建。
- 启动后5分钟内完成第一次实际代码修改；修改前不跑测试或扩大搜索。若crate依赖方向阻止直接复用，立即
  回报精确依赖缺口，不复制第三套规则。
- 提交一个聚焦commit和自验收矩阵，不push。
