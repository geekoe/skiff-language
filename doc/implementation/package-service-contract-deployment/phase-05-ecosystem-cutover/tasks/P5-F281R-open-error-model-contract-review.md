# P5-F281R Open error model task-contract review

状态：Ready。

## 直接父节点与事实链

- 被评审叶子：
  `P5-F281-open-error-channel-shared-model.md`
- 叶子直接父结果：
  `P5-F280-open-service-error-channel-implementation-audit-result.md`
- F280继续引用F279及唯一权威设计。

启动时先读本任务，再读被评审叶子；只在需要核对时沿其父链向上读取。

## 评审目标

独立、只读判断F281是否能在授权范围内完成，不评审或修改production。至少核对：

- shared model checkpoint是否拥有要求改变的真实DTO、strict serde与co-located fixture；
- declaration kind、branch/context identity、runtime nominal carrier、source/synthetic site、
  fixed envelope与删除closed throw-set的完成标准是否足够明确且互相一致；
- 从W1移走artifact identity constants是否消除了隐藏跨owner验证要求，是否遗漏了必须仍由本节点拥有的
  schema constant；
- 指定两个crate的命令是否真实存在、会执行非零测试，是否暗中要求修改未授权consumer；
- 允许的临时编译破坏是否被准确限制，是否存在可避免的dual shape、兼容层或重复owner；
- 后续language/artifact/runtime consumer能否只依赖该checkpoint，而不会各自重新决定公共DTO；
- 任务是否遗漏父结果中会阻断第一次production修改的事实。

## 输出

直接返回：

- `PASS`，或按blocking/non-blocking分类的finding；
- 每个blocking finding的文件/symbol证据、为什么在现有范围内不可完成、最小合同修改；
- 是否存在新增用户设计决策。

不创建result文档，不修改文件，不运行build/test/format，不提交，不push，不操作stable。一次性完成评审后停止。

worktree：`/Users/geek/workspace/skiff-p5-f281-review`  
branch：`codex/p5-f281-review`
