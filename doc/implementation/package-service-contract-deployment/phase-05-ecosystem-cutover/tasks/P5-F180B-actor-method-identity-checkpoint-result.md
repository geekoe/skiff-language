# P5-F180B：Actor Method Identity Checkpoint Result

状态：Completed

## 直接父任务

- `P5-F180A-actor-executor-gap-audit-result.md`

## 结果

CP1 已冻结并落地：

- `ActorMethodIdentity` 由声明 module、Actor 名和方法名生成稳定身份。
- Actor ABI 覆盖 id、字段、公开方法参数/返回、`maySuspend` 和 runtime ABI。
- Actor implementation identity 只覆盖公开方法入口可达的规范化 executable、const 和 type 闭包；
  source span、索引和无关代码不参与，递归与 SCC 使用整体排序图哈希。
- 另一个 Actor 的 implementation identity 不污染当前 Actor identity；调用仍精确固定其 ABI、
  implementation 和 method identity。
- `ActorPublicMethodIr` 是纯公开 ABI；私有方法入口独立保存在声明的 implementation map，入口编号不污染
  ABI。
- source、File IR 和 linked program 分别使用专用 `ActorMethod` / `ActorDispatch` target。最终 dispatch
  plan 只引用声明 owner 和三类 identity，不含 `ExecutableAddr`。
- Actor 名义类型可作为普通签名类型，但仍只表现为 service symbol，不进入 type table、`TypeAddr` 或
  record descriptor。

本任务没有实现 method wire、Router dispatcher 或 Runtime Actor executor。Eval 对最终
`ActorDispatch` 明确报告 executor 尚未实现，不回退到普通 executable 调用。

## Mutation matrix

- id、字段、参数、返回、`maySuspend`、runtime ABI 变化：Actor ABI identity 变化。
- 方法 IR、可达 executable、const、type 变化：implementation identity 变化。
- 无关函数、无关 Actor、不可达 const/type、source span变化：目标 implementation identity 不变。
- executable/const/type 向量及本地索引重排：identity 不变。
- self recursion、相互递归和 SCC：identity 稳定可计算。
- 声明 owner、ABI、implementation、method、入口越界或方法表不一致：link/deserialize fail closed。

## 验证

- `cargo test -p skiff-artifact-model --lib`：113/113 PASS
- `cargo test -p skiff-artifact-identity --lib`：79/79 PASS
- `cargo test -p skiff-runtime-linked-program --lib`：18/18 PASS
- `cargo test -p skiff-runtime-linker --lib`：25/25 PASS
- `cargo test -p skiff-compiler-source actor_ --lib`：5/5 PASS
- compiler lowering 真实 Actor declaration + impl + caller fixture：PASS
- `cargo check --workspace`：PASS
- `git diff --check`：PASS

验证基于已合并的 integration checkpoint `6a334ac`。无用户决策阻断。
