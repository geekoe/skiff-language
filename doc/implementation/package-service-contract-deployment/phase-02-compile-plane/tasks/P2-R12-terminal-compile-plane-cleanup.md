# P2-R12：Terminal Compile-plane Cleanup

状态：absorbed；不执行。

旧方案试图在已经合入兼容链的 integration tree 上做大规模删除。现改为从 `9ca2547` 创建新的 terminal
integration，因此这些提交根本不进入新分支 ancestry。

- clean base 上原有的旧 compiler publication owner 由 T05 直接终态替换。
- compiler integration fixture 由 R10 重建。
- production 反向搜索和旧 DTO producer 归零由 T07 验收。
- runtime/router/test-runner 保持 clean base 原状，Phase 02 不修改它们。

不得重新派发 R12，也不得把旧 integration tail cherry-pick 到新分支后再清理。
