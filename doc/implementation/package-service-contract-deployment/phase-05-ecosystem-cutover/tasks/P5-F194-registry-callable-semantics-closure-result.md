# P5-F194：Registry Callable Semantics 闭合结果

状态：Completed

## 直接父任务

- `P5-F194-registry-callable-semantics-closure.md`

## 首因与修复

真实 Registry receipt 的八个阻断没有来自动态调用：

- 四个 `*Put` 经 `*PutAttempt` 进入 `db transaction value`，旧 transfer 无条件把 transaction
  结果标成 unknown；
- 四个 `*PointerCas` 同样被 transaction unknown 污染，并把 transaction 内已知的 DB 写入、
  provider 内部比较和 fresh receipt 外层合并成保守的全部 effect。

F196 已让 transaction value 保留最终表达式的真实 provenance，并按 DB 的 BSON 编码边界处理
静态字段写入。F194 没有把 receipt 伪装成 fresh：receipt 内嵌的 caller candidate alias 仍保留在
source facts 中。

boundary projection 现在结合真实 value plan 消费这些事实：

- 参数必须全部是 `DetachedValueGraph + CanonicalValue`；
- 只有 Database lane 可由 DB 编码物化，其他 capture/callback/stream/spawn/native/external lane
  继续拒绝；
- 只有包含 fresh 外层、caller parameter 内嵌来源、无 unknown target 且返回 plan 为 canonical
  detached graph 时，返回编码才消化内嵌 alias；
- provider 内部、仅伴随已物化 Database lane 的 same-heap 使用不再被误当成跨 boundary 身份承诺。

deployment 会从 PackageArtifact 的完整 effects/provenance 和 operation value plan 重新执行同一套
校验，不能靠伪造 Available discriminant 绕过。

直接返回 caller 参数、unknown/dynamic target、非 Database escape、无来源证明的 same-heap
依赖仍 fail closed。

## 真实 Registry 结果

使用 F187 Registry 源码、canonical std bootstrap 和隔离 artifact root 重新执行真实
`package build`：

```text
Service API for skiff.run/registry
Available: 20
Package-only: 0
```

四个 Put、四个 PointerCas 以及十二个 Read/History 均为 Available；没有删除 operation、修改
Registry API、复制返回值或放宽 unknown target。

`npm run test:registry` 的源码检查 4/4 通过，canonical package build 再次得到 20/20。其后的
隔离 runtime 存储测试在测试设施启动阶段因临时 Router 退出而停止，尚未进入任何 Registry
fixture；同一轮 workspace check、compiler source、projection 和 deployment 验证均通过。

## 验证

```text
cargo test --offline -p skiff-compiler-source callable_effects
38 passed

cargo test --offline -p skiff-compiler-projection
26 passed

cargo test --offline -p skiff-deployment
51 passed

cargo check --offline --workspace
passed

真实 Registry package build
20 Available / 0 Package-only

Registry source checks
4 passed

git diff --check
passed
```
