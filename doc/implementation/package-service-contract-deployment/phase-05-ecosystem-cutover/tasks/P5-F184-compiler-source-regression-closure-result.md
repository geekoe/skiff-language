# P5-F184：Compiler Source 与 Lowering 回归闭合结果

状态：Completed

## 失败分类

基线命令 `cargo test -p skiff-compiler-source --lib --no-fail-fast` 精确得到
`215 passed / 13 failed`。13 项没有通过跳过或 fallback 消除，分类如下。

| 数量 | 失败 | 首个错误 owner | 结论 |
| --- | --- | --- | --- |
| 2 | `direct_scalar_parameter_field_store_has_only_write_effect`、`stream_spawn_database_and_callback_escape_lanes_are_explicit` | 普通 impl 的 `Boxed.clear` / `Boxed.name` 内 `self.value` | production 回归 |
| 4 | 四个 any-interface / `Host` expression type 测试 | `ImplMethod { type_name: "Host", method: "name" }` 内 `self.label` | 与上一组相同的 production 回归 |
| 6 | 三个 package interface contract 测试中的两个、四个 type-resolution package interface 测试 | exported package type index 未收录 interface | production 回归 |
| 1 | `compiler_known_interfaces_stay_outside_source_exact_conformance_ownership` | `type ShortActor implements Actor<string>` | Actor 已不是 interface，旧样例失效 |

另外，在修复 package interface 索引后，正探针暴露了一个更深的正式错误：当使用方模块名与
package interface 源模块名相同时，使用方的 `LocalType { type_index }` 会被误解释为 package 的
local type slot。该错误会把 `api.Host` 的 receiver 错认成 package 的 `api.Reader`。

## 实现

- `self.field` 只有在 receiver 的静态类型事实明确为 actor 时才查询 actor state；普通 record 和
  generic record impl 继续走静态 record 字段解析。
- package 公共类型索引统一收录 interface 的名称、类型参数、源模块和公开路径；后续仍须命中
  canonical package interface fact，未加入名字猜测或未知类型兜底。
- package interface 方法比对区分：
  - 使用方的本地 type slot；
  - package interface 自有的 local type slot。
  两者不再因同名模块和相同 slot 编号碰撞。
- 旧 Actor 样例改为 `actor ShortActor id string {}`；`Actor<string>`、
  `std.actor.Actor<string>` 和 actor 声明本身均明确不是 interface selector。
- lowering 测试通过统一测试入口显式初始化 `CompilerPlatformSources`；正式编译代码仍要求调用方
  先提供平台上下文，不增加隐式默认 registry。

## 正负探针

- Actor receiver 既有测试仍得到专用 `ActorMethod` target。
- 新增同一测试覆盖普通 `User`、泛型 `Box<T>` 和 `UserActor` 的 `self.field` 静态解析。
- 普通类型和 actor 的未知字段均继续失败关闭。
- package interface 正探针覆盖泛型实参替换、公开 alias 类型和同名模块的 local slot 隔离。
- package interface 方法缺失、签名不匹配、未知 interface 继续失败关闭。
- callable effects 的 unknown / dynamic / capability 负探针均保留。

## 验证

- `cargo test -p skiff-compiler-source --lib --no-fail-fast`
  - `229 passed / 0 failed`
- `cargo test -p skiff-compiler-lowering --lib --no-fail-fast`
  - `42 passed / 0 failed`
- `cargo test -p skiff-compiler --test runtime_slots --no-fail-fast`
  - 与本任务直接相关的普通 impl、generic impl、string/bytes receiver 探针通过；
  - 全文件余下 2 项在进入被测逻辑前被 P5-F183 的旧 std schema fixture 阻断：
    `exact package requirement ... has no resolved schema or canonical store resolver`。
- `cargo test -p skiff-compiler --test runtime_slots
  generic_impl_receiver_call_lowers_to_static_executable -- --exact`：通过。
- `cargo test -p skiff-compiler --test runtime_slots
  user_impl_receiver_call_lowers_to_static_executable -- --exact`：通过。
- Router `compilerGeneratedManifestCompatibility` fixture
  - 编译已越过本任务修复的 source/lowering；
  - fixture 仍调用已删除的旧 compiler CLI `<root> --out --manifest-out`，当前 CLI 只接受
    `package|assembly build|publish`，属于独立 authoring fixture cutover。
- `git diff --check`：通过。
- `cargo check --workspace`：通过。

## 不变量

- 未恢复 `ActorRef` 或 `Actor<T>` source interface。
- 未增加 unknown call、动态 receiver、source 名字猜测或 package/source fallback。
- 未修改 Runtime、Router、Package schema store 或 contract/boundary 失败关闭语义。
