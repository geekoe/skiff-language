# P5-F289 Wave 2 std/artifact task-contract review

状态：Ready。

## 直接父节点

- 被评审任务：
  - `P5-F287-std-error-surface-migration.md`
  - `P5-F288-open-error-artifact-contract-consumers.md`
- 两任务沿F279/F280/F284追溯唯一权威设计与A1冻结DTO。

启动时先读本任务与两个被评审叶子；需要依据时沿各自父链向上读取。

## 评审目标

独立、只读判断两个任务能否在各自授权范围与基线上完成，不修改production。至少核对：

- F287列出的compiler-known interface、prelude registry、std/api、source-layout、VSC与native fixture是否覆盖
  Skiff repo真正`ErrorPayload`/旧InternalError owner，且不会误删业务DTO或runtime generic code；
- F287在pre-A1 base上的命令是否真实、非零、无需越界修改；其提交能否干净合入A1而不覆盖shared DTO；
- `std.service.InternalError`字段、public path、SchemaClosed/nameability要求是否完整，是否误做bare prelude；
- F288是否覆盖所有artifact/contract identity/normalization/closure/admission consumer，是否遗漏一个会让授权
  crate自身无法编译的production文件；
- F288对File IR、Package build、Local ABI、ServiceProtocol marker/prefix的owner是否精确；不变identity domain
  是否正确；
- 两任务及F285/F286/F278/F281之间是否存在未声明的同文件写入；
- 测试命令是否匹配真实crate/非零测试，临时compiler/runtime断编译是否被合理限制而非用作越界借口；
- 是否存在新增用户设计决策。

## 输出与边界

直接返回：

- 总体`PASS`，或分别列F287/F288 blocking/non-blocking finding；
- 每个blocking finding的文件/symbol证据、为什么现有范围不可执行、最小合同修改；
- 是否需要重新安排依赖顺序；
- 是否存在新增用户设计决策。

不创建result文档，不修改文件，不运行build/test/format，不提交、不push、不操作stable。一次性评审后停止。

worktree：`/Users/geek/workspace/skiff-p5-f289-review`  
branch：`codex/p5-f289-review`
