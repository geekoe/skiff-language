# P2-R03：Canonical Export Link 的精确 Payload Symbol

状态：port；以旧 commit `6248394` 为只读证据，将 canonical payload-symbol 修复移植到 clean checkpoint。
不得移植 legacy presentation/adapter test 或依赖。

## 目标

Package export map 的 key 表达 public/package path；`TypeExport.symbol`、`ConstExport.symbol` 和
`ExecutableExport.symbol` 表达 `file + index` 指向的真实 File IR declaration symbol。两者不得
混用，任何 consumer 不得通过 suffix 或其它弱匹配隐藏 producer 错误。

## 范围

- 修正 `compiler/projection/src/package_unit_artifacts/exports/**` 中 type/const payload symbol 的投影。
- 保留 map key 的 std/public path 特殊规则，不改 public ABI 寻址。
- 不修改 identity 算法、runtime 或 service binding，也不新增旧 artifact presentation。

## 验收

1. public path 与 declaration symbol 不同时，map key 保留 public path，link symbol 精确等于 payload name。
2. std type/const 直接 fixture 通过 canonical materialization 的精确校验。
3. 错误 file/index/symbol 继续 fail closed；不引入 suffix 匹配或第二 symbol owner。
4. projection/emission 聚焦测试、`cargo check -p skiff-compiler`、`git diff --check` 通过。
