# P5-F389 Compiler→Router HTTP fixture migration blocker

状态：TASK_SCOPE_EXPANDED（任务合同写错；production未修改）。

F389同时要求`typedJson`和零request arguments/external sources，但F357已冻结typed gateway至少有一个
`http.body` formal。fresh publish准确拒绝：

```text
typedJson requires at least one http.body formal
```

该限制由compiler production与直接负例共同保证，不应为fixture修改。执行Agent已恢复全部试改，worktree
clean，无production commit。

正确后继是复用F384已经验证的typed-null adapter：增加private
`__skiffHttpPing(body:null)->string` wrapper，内部调用现有`ping()`，由wrapper承接`http.body`。
这不改变原业务函数或公开API，也不需要zero-input typedJson新语义。
