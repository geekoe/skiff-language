# P5-F441M Live test execution gap audit result

状态：`DECISION_CLOSED / REMOVE_NONREQUIRED_CASES`。

本节点只读追踪F441I保留的两个execution blocker，并由用户关闭处置：不新增test input或expected
platform-error语法，删除两条没有交付价值的case及其专用helper。

## 1. Current事实

### `__skiffPayload`

`runtime/live-tests/internal/operation.live.test.skiff`第二个case使用未声明的
`__skiffPayload`。它不是语言builtin：

- `syntax::TestDeclaration`没有input；
- test overlay把case生成为零参数函数；
- canonical test gateway和runtime dispatch固定发送JSON `null`；
- 真正lowering该case会报unresolved identifier。

F441I的integration probe因此只discover两个operation case，没有compile；这不是可执行测试证据。

### Expected platform error

`file_live.live.test.skiff`的over-limit case以末尾`assert false`隐式假定前面的
`ResourceLimitExceeded`会终止执行，但test AST/runner没有terminal expectation。

仅增加runner字段也不足：assembly test Router seam当前会把runtime `response.error`压成控制面失败，
不能严格区分预期平台错误与超时、断线等基础设施失败。

## 2. 被拒绝的扩张

曾核对过一种通用语义候选：

```skiff
test "..." input payload: string = "..." { ... }
test "..." expects platformError "ResourceLimitExceeded" { ... }
```

它需要修改testing reference、syntax AST/parser、test overlay/gateway/dispatch，以及expected-error所需的
Router error-preserving seam。用户判断这两条覆盖没有价值，并明确选择删除，因此：

- 不新增上述语法或上下文词；
- 不增加live-only flag、旧JSON config或按test name硬编码；
- 不修改Router/test-runner production去支持未采用的expectation；
- 不保留`__skiffPayload` magic identifier。

## 3. 已批准的最小后继

删除：

1. `live operation dispatch crosses runtime binary payload boundary`；
2. `live file runtime rejects stream above file guard limit`；
3. 只被上述case引用的normal-source helper。

随后更新真实root integration：

- operation剩余case必须真正compile，不再只discover；
- file剩余case继续compile；
- canonical root总case数按实际source收敛；
- reverse search证明magic identifier、over-limit helper和旧“later execution owner”断言归零。

其它file lifecycle cases与source owner不在用户本次“这两条”范围内，保持不变。无需修改public testing
reference，因为没有新增语言或runner语义。
