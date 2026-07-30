# P5-F194：Registry Callable Semantics 闭合

状态：Ready

## 直接父任务

- `P5-F190-database-state-requirement-projection-result.md`

## 问题与目标

真实 Registry 20 个 API 只有 12 个进入 ServiceContract。四个 Put 被
`unknownCallTarget + returnsCallerAlias` 阻断；四个 PointerCas 被
`unknownEffect + returnsCallerAlias + requiresSameHeapIdentity` 阻断。沿真实调用图定位首个污染
callee，修复 compiler/source 精确 effect/provenance transfer，使 20/20 可用。

不得删除 operation、复制返回值、标注虚假 detached/fresh 或放宽 unknown fail-closed。

## 验证

- Registry 20/20 operations Available；
- Put/PointerCas 真实存储语义与返回形状；
- unknown/dynamic/alias/same-heap 负例；
- compiler source/projection 与真实 Registry build/deployment；
- workspace check、diff check；
- 独立提交和 result。

