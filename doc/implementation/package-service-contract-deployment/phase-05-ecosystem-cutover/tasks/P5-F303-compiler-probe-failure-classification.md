# P5-F303 Compiler combined probe failure classification

状态：Ready。

## 输入

- 失败结果：`P5-F302-applied-nominal-compiler-combined-probe-result.md`
- 实现父节点：`P5-F301-applied-nominal-package-public-consumer-result.md`
- generic policy父节点：
  `P5-F293-generic-nominal-type-ref-owner-audit-result.md`

精确代码状态：当前integration HEAD；production与F302候选相同。

## 角色与边界

这是一次只读failure classification，不是实现、验收或gate。不得修改/提交文件，不运行完整测试，
不操作stable/live。

## 必须回答

1. 四个std websocket公开类型的source/File IR `type_params`是否非空；若为空，哪条F301 production
   路径把它们误判为generic，输入事实是什么。
2. 明确区分：
   - declaration自身拥有type parameters；
   - non-generic declaration字段/branch内部使用builtin container或fully-instantiated generic nominal。
   当前fail-close policy只应拒绝哪一种，父结果中是否已有答案。
3. 从source → lowering package ABI handoff → compiled/projection-input → package public validation追踪
   exact symbol/field，给出最小production owner与应补的正/负测试。
4. 搜索同类误判是否影响std之外的production package/public schema路径，列出完整明确范围，不把
   fixtures当production。
5. 统计compiler tests中旧`BoundaryErrorContract`、`BoundaryOperationContract.errors`的所有真实
   构造点，确认mechanical修复能否与production修复并行且文件不重叠。
6. 给出finding wave的最小leaf任务划分、写入文件范围、前置、combined probe失效面；若发现公共语义
   缺口而不是既定policy实现错误，明确标为需要用户决定。

只允许`rg`、文件读取和git只读命令；返回精确`file:line`与结论，不承接实现。

