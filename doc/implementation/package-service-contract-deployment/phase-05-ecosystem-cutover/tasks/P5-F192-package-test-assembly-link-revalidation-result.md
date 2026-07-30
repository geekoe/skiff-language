# P5-F192：Package Test Assembly Link 复验结果

状态：Completed

## 结果

真实 Package test 的 canonical artifact/deployment/assembly/link 链已恢复。没有修改
aliyunoss、http-session、openai 或 track 的业务源码来绕过 Runtime。

最初的 HTTP 409 已保留并定位到完整底层链：

```text
whole-assembly candidate link failed
→ failed to link the canonical assembly execution image
→ failed to link std.actor.getOrCreate
→ std.actor.getOrCreate T0 must be a nominal actor ServiceSymbol
```

根因是 std.actor 的泛型 native 声明在定义体内仍以 `TypeParam` 表示 T0，assembly
linker 却按已经具体实例化的 Actor 调用校验。linker 现在只对当前可见的泛型参数推迟
Actor owner metadata；运行时在具体调用处规范化类型替换并精确解析 Actor 声明。未知泛型、
缺失或歧义 Actor 声明继续失败关闭。

后续暴露的第二个 canonical 缺口是 `AssemblyExecutionImage` 丢失 Package ID、文件和公开
类型的 link overlay，导致 std HTTP native signature 无法解析 `skiff.run/std` 类型。执行
image 现在从已准入的共享 Package code slots 和 type index 构造精确 overlay，并拒绝重复
Package ID 和缺失类型导出。

第三个缺口是 canonical RuntimeAssembly ingress 固定清空 test effect doubles。测试运行器
现在严格读取共享或 Package 本地的 `skiff.test-doubles.json` 中 config 与 effect doubles；
config 只匹配当前精确 Package closure 的已声明 requirement，歧义和重复继续失败关闭。
仅 Router control 端的
`runtimeAssembly` test dispatch 可以携带 doubles，并且必须匹配当前 active assembly 中的
精确 ingress 与 `ContractOperationId`。公开 ingress 仍拒绝 test controls。Runtime
canonical request bridge 将这些 doubles 传入无 legacy program 的 assembly interpreter。

真实 database Package 随后暴露出执行期仍从 `Interpreter::program_projection()` 读取旧
`RuntimeProgram` 的缺口。DB result plan、recoverable plan 与命令执行现统一从 request
已经钉住的 `RuntimeExecutionProjection` 取 type view；普通 assembly DB 操作不再回退旧
program。需要旧 recoverable behavior hook 的路径仍显式失败关闭，未伪造 legacy projection。

隔离 Runtime 的失败输出现在附带经过脱敏、限长的 Router/Runtime 日志尾部；assembly
prepare/commit 日志使用完整 anyhow cause chain，不再只显示顶层 409。

## 验证

- aliyunoss：6/6 通过。
- http-session：19/19 通过；track：4/4 通过（使用 F187 worktree 中尚待包侧收口的
  database state 声明）。
- openai 已越过原 assembly 409，但被独立的 test boundary `UnknownCallTarget` 拒绝；
  该问题属于正在收口的 callable/base64 语义任务，不在 F192 linker 链内，不能在本任务
  吞掉或伪装成通过。
- http-session/track wrong exact PackageArtifact ref：失败关闭探针通过。
- Actor 泛型 assembly link、RuntimeAssembly test-control 正负路径、isolated log evidence：
  聚焦测试通过。
- Router type check、相关 Rust crates、workspace check、`git diff --check`：通过。

F190 的 Package database state requirement 投影是 http-session/track 的前置提交；本分支
先保留该独立提交，再提交本结果。
