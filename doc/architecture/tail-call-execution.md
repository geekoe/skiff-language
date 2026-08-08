# Tail-call Execution Architecture

本文是Skiff本地尾调用eligibility、递归stack安全与诊断语义的内部架构事实源。用户可观察的tail position、
支持范围、budget、错误与non-tail failure语义只由
[`../reference/runtime.md#tail-call-execution-and-recursive-stack-safety`](../reference/runtime.md#tail-call-execution-and-recursive-stack-safety)
定义；目标bytecode执行机制由[`bytecode-vm.md`](bytecode-vm.md)共同约束。

设计锚定baseline commit
`874ee3a6bd5123d2c54e1a550d07fd99b29b27ad`、tree
`b733e5141115f8cc4b4328bfa8613c96cf67b930`。Skiff尚未发布，不新增旧artifact兼容路径。

## Scope and outcome

完成态必须让direct self recursion、同一Package内的mutual recursion、跨source module mutual recursion及
generic/impl self recursion在满足reference eligibility时由同一bytecode dispatch loop执行frame
replacement。递归次数不得
增加native poll stack、active non-tail program depth或尾调用诊断栈空间。

目标实现服务当前request钉住的exact deployment `buildId` `DeploymentExecutionImage`。树遍历
evaluator迁移期间仍可使用本文件记录的
`Flow::TailCall` trampoline，但production cutover后由bytecode `tail_call_local`替代；不保留legacy/test
projection或fallback。该变化不增加语言关键字、annotation、manifest字段、环境旋钮或用户配置，也不依赖
`tokio::spawn`把普通尾调用拆成新task。

## Tree-evaluator baseline facts

以下是迁移前tree evaluator已经保留的识别事实，用于解释为什么该阶段不新增tail marker；bytecode target
由上一节的emitter/verifier facts取代：

- `StmtIr::Return { value: Option<ExprRefIr> }`精确引用其返回表达式；
- `ExprIr::Call { call: CallIr }`保存target、source site、ordered args和generic type args；
- source/lowering把same-file target降为`LocalExecutable`，跨module target降为
  `PublicationExecutable`；
- 迁移前tree-evaluator linker把两者都归一为
  `LinkedCallTarget::Executable { addr: ExecutableAddr }`；
- linked executable保存params、return type、self type、slot layout、`maySuspend`和body；
- 迁移前evaluator的invocation/projection seam已经拥有env创建、generic substitution、
  explicit/inherited self及argument declaration逻辑；
- `ProgramExecutionContext`已经共享execution control、request heap limits、Actor frame、exception
  correlation与local call stack。

这些事实足以在linked evaluator的`Return`分支做精确识别。删除任何一个事实都会破坏target、argument、
generic/self或source attribution；新增persisted marker则不会提供新的必要信息。

## Canonical owner and cutover

Target bytecode组合只有四部分：

1. Source/lowering按reference识别eligible direct-return call，并保留exact relocation与return plan facts。
2. Canonical artifact ISA新增semantic opcode `tail_call_local`，bytecode emitter为eligible edge发射该
   instruction与exact target relocation；它不是可漂移的boolean marker。
3. Pre-link validator验证opcode/relocation结构；post-link verifier证明exact-local target、return plan、self/
   generic substitution、`NoPending`/`InOut`和cleanup-region eligibility。
4. VM在同一dispatch loop中以单一commit替换当前frame/value segment。

树遍历evaluator阶段不新增File IR `tail`字段、metadata convention或SCC annotation，继续从
`Return -> ExprRef -> Call -> exact linked target`识别并产生internal `Flow::TailCall`。Bytecode hard cut后，
`tail_call_local`有意废止并取代“artifact不得有tail opcode/marker”的旧contract。新ISA
允许且要求PackageArtifact持久化该opcode；仍禁止的只是与opcode并行、可与之漂移的
`is_tail`布尔字段或SCC/metadata marker。新ISA reader不识别该opcode时必须按version fail closed；
runtime不得重新猜测第二套eligibility或fallback为普通call。

`tail_call_local`的relocation只能在当前exact deployment `buildId`的
`DeploymentExecutionImage`内解析成exact executable target。Frame replacement不得改变
`DeploymentOwnerIdentity`或跨service/deployment边界；execution identity中没有`RuntimeAssembly`、
assembly/activation generation或ambient active set。

两代实现都不得只优化self symbol。Exact target自然覆盖mutual/cross-module recursion；不得用
`tokio::spawn`或每hop heap continuation模拟TCO。

## Tree-evaluator migration control contract

迁移期`runtime/eval`使用一个internal-only control result；production bytecode VM不保留该Rust control
shape：

```text
Flow::TailCall(PreparedTailCall)

PreparedTailCall
  target: ExecutableAddr
  projection: RuntimeExecutionProjection
  env: Env
  returnPlan: optional RuntimeTypePlan
  tailSite: InstructionSourceSite
```

Prepared frame只能在caller环境仍存活时创建：

1. 按现有left-to-right顺序求值每个argument一次；
2. 用既有call target validation解析exact executable；
3. 用既有`env_for_call`计算caller addr与generic substitutions；
4. 按既有ABI选择explicit self、inherited self或receiver-first carrier；
5. 比较caller与callee实例化后的return plan；
6. 只有全部成功且没有lexical barrier时返回`Flow::TailCall`。

任何一步不满足eligibility都回退现有普通调用路径，而不是产生部分prepared frame。Argument求值错误在caller
site归因；target/env/arity错误在当前tail site归因。

Trampoline每次迭代：

1. 对一次tail transfer计与普通local call相同的call instruction unit并poll execution budget；
2. 不调用non-tail depth push；
3. 以prepared env执行target callable，保留同一execution control、request heap、Actor frame及request
   capabilities；
4. `Flow::TailCall`替换当前frame后继续loop；
5. `Flow::Return`按共同return plan物化一次并退出；
6. 其它terminal仍由既有`FlowCompletionPolicy`和scope owner处理。

Function-entry、generated chunk、loop、deadline和internal-stop checkpoints保持现有位置。尾跳不能合并、
采样或省略一次迭代的instruction accounting。

## Lexical barriers and propagation

识别不是对任意`Return(Call)`的无条件rewrite。Evaluator必须显式携带“tail transfer能否直接传播到当前
callable exit”的internal context：

- ordinary entry/block、`if`、statement `match`及普通array/map loop可以传播；
- block scope必须先pop局部env，再把prepared frame交给trampoline；
- timeout scope、DB transaction/lease、concurrent scheduler/lane/value、catch/value wrapper以及任何仍有
  commit、rollback、join、winner、terminal arbitration或result transformation的owner禁止传播；
- stream consumer的`return`必须先完成既有best-effort source stop与consumer cleanup；第一版若没有一个
  已验证的cleanup-before-transfer seam，就回退普通调用；
- deferred stream producer、带stream-producing argument的协同consumer及需要
  `prepare_deferred_stream_producer*`的call在第一版一律回退；
- 一个被普通调用进入的新executable重新从其自己的ordinary tail context开始，不能继承caller内部barrier。

语言没有general `defer` AST/IR。代码中的deferred语义仅指stream producer parking；不得为本设计新增cleanup
stack或`defer`关键字。

## Supported and excluded targets

第一版保证只包含`LinkedCallTarget::Executable`。这不是self-only限制：同一Package内所有可形成递归SCC的
local/publication edges在link后都属于该variant，因此direct self、mutual、cross-module、generic function及
静态impl self recursion全部覆盖。

以下target不进入第一版trampoline：

- `PackageDirect`：必须保留Package target validation、test-effect interception、Local ABI lane及optional
  top-level `const` receiver求值。Package dependency graph不能形成普通本地递归SCC；进入dependency callable后，其
  内部`Executable`递归仍可TCO。
- local interface method与top-level `const` receiver executable：动态receiver/payload或frozen const必须先求值；
  进入exact method后的静态递归仍由`Executable`覆盖。
- service、Actor dispatch和callback capability：它们跨deployment/instance/capability owner、fresh heap、
  boundary materialization、wire或continuation boundary。
- native、builtin、receiver builtin及未链接/未知interface target：它们不是普通program frame replacement。
- stream defer、dispatch和emit：它们分别属于scheduler、request或stream terminal owner。

未来扩展到`PackageDirect`或运行时解析出的local interface/top-level `const` receiver时，
必须先把现有validation、
test-effect、receiver和call-site preparation收敛到同一个prepared-frame seam。不能复制dispatch逻辑，也
不能改变reference tail-position定义；该扩展不是本次完成标准。

## Return plan, heap carrier and self

本地尾跳共享同一个`RequestHeap`。Prepared args和self都是owned `RuntimeValueCarrier`，heap handle不clone到
另一heap，也不做service-boundary materialization。

普通callable return会执行`materialize_local_callable_return`。删除中间frame只有在caller与callee的
instantiated `RuntimeTypePlan` canonical-equivalent时安全：

- `None`与`None`等价；
- nominal/catch identity、union branch、representation、container element plan及generic substitution都
  必须参与比较；
- compiler显式插入的representation wrap或其它conversion使返回表达式不再是direct Call，因此自然回退；
- plan不同或无法证明等价时走普通调用，不能积累unbounded return-plan continuation stack；
- terminal value只按共同plan物化一次，并以测试证明对该plan的carrier结果与原普通路径等价。

Generic type args必须在caller env销毁前通过既有`call_type_substitutions`实例化。Impl method沿用现有
explicit-self param、SelfValue slot与inherited self规则；tail replacement不能重新推断receiver，也不能
丢失self carrier的heap/catch identity。

### Bytecode frame replacement safety

`tail_call_local`是已验证artifact的semantic instruction，不是runtime探测后可回退的optimization。
Post-link verifier必须在VM执行前证明：

- target在同exact deployment `buildId` image内，arity、parameter/return plan、self、generic
  substitution与Package Local ABI精确匹配；
- opcode处在可直接退出callable的region depth，没有未完成cleanup/catch/timeout/transaction/
  scheduler owner，也不在unwind path中；
- frame中没有必须由被删除slot继续拥有的callback capture、resource guard、transient root或
  pending/resume state；VM只能在fiber为`Runnable`、`PendingOperation = None`、
  `UnwindState = None`时commit replacement；
- 普通aggregate argument按value semantics做move/share snapshot。Physical backing可共享，但callee不能
  因frame replacement获得可观察mutable alias或指向已删除slot的raw reference。

VM必须在caller frame仍完整存活时，按源码顺序对每个argument求值一次，并把结果放入
不会被source/destination overlap覆盖的rooted prepared segment。求值、分配、target/arity/type检查或budget
检查失败时，caller frame仍可用于正常unwind和call-site归因；不得留下部分新frame。全部准备
完成后，VM以一个commit：

1. 保留旧frame的caller continuation、return destination和exact deployment owner；
2. 完成loan结束/转移与root所有权变换；
3. 截断旧slot/operand/region segment，安装callee frame与prepared arguments；
4. 把pc切到callee entry，中间不暴露可调度或可观察的half-replaced state。

Commit后任何live slot、loan、handle或resource都不得指向已删除frame segment。这一不变量无法证明时，
emitter必须发射`call_local` + `return`，或verifier fail closed；runtime不能把已验证的
`tail_call_local`临时降级成普通call。

### `NoPending` and `InOut`

`InOut`是Package Local ABI的exclusive write-through loan，不是普通value。Tail replacement必须保持其
loan lifetime而不保留caller frame：

- 任何拥有`InOut`形参的callable都必须通过transitive `NoPending`验证。该callable可达的
  `tail_call_local`、argument evaluation与target body都不得到达pending-capable instruction；不能用
  runtime“这次恰好Ready”替代proof。
- target含`InOut`形参时，verifier必须同时证明target是当前image内的exact local executable、其
  Local ABI为`NoPending`、actual是exclusive writable `var` path，且loan不逃逸到callback/resource/
  concurrent sibling。
- caller已有的incoming loan只能在tail edge上结束，或将同一writable-path descriptor与loan
  token线性转移给callee；不允许copy loan或同时保留两个writer。
- 以将被截断的caller local slot为base的新`InOut`实参不能在首版直接进入
  `tail_call_local`；emitter必须保留普通call。未来若支持，必须在ISA中定义可验证的
  typed rehome/transfer，不能让runtime保留raw caller-slot pointer。

任何`NoPending`、loan exclusivity、base lifetime或linear transfer无法证明的edge都不得发射
`tail_call_local`；已损坏artifact在post-link verification阶段拒绝。

## Instruction, depth and scheduler boundaries

当前`MAX_PROGRAM_CALL_DEPTH = 128`作为non-tail native-stack crash fuse保留，但语义收窄为active
non-tail program frames：

- 普通nested call push一次；context值传递使返回后parent depth自然恢复；
- tail transfer不push；
- 独立scheduler task拥有新的native stack，执行callable前必须使用
  `OwnedProgramExecutionContext::borrow_for_scheduled_task()`重置depth；
- continuation仍属于原native call chain时不得清零；
- depth exhaustion保持结构化、不可catch的
  `ResourceLimitExceeded(resource = "programCallDepth")`，并报告实际limit；
- exact值不是language ABI或用户配置。不得用另一个任意常数代替TCO。

Baseline local stream producer task已经正确重置depth；provider stream的独立spawn仍使用普通`borrow()`，
实现阶段必须把“所有真正独立callable scheduler entry”作为有界同类搜索范围。Stream terminal error mapping
是同一baseline中的独立修复，不属于TCO，不得无因果改写。

### 原生栈配置与实测消耗

non-tail普通调用仍以nested `#[async_recursion]` future在worker原生栈上执行，
因此`MAX_PROGRAM_CALL_DEPTH = 128`必须配合足够大的worker栈，guard才有机会在
原生栈耗尽前以结构化错误拒绝。`RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES`
（`runtime/driver/config.rs`）按构建形态区分：

- release（生产部署形态）：64 MiB。实测128层non-tail链约需34 MiB（每层上界
  ≈272 KiB），64 MiB保留约1.9×余量；
- debug（`debug_assertions`，`cargo test`、dev instance与CI使用）：192 MiB。
  unoptimized evaluator帧显著更大，实测每层≈1.04 MiB（64 MiB栈在62层通过、
  63层即原生栈溢出），128层需要超过128 MiB；192 MiB保证128层可执行到guard
  边界且不会在63层左右直接abort进程。

上述数字来自非尾countdown链（route经trampoline不占depth、每个递归调用一帧）的
实测上界，记录见对应driver回归测试。8–16 MiB的“小栈”在release与debug下均不足以
承载128层（release实测下限≈34 MiB），因此小栈证明只以release形态的40 MiB测试
呈现（仍远低于生产64 MiB），debug形态用提升后的worker栈证明guard边界可达。

router session loop（`run_forever` → `run_connected_session_with_bootstrap`）会在
driver线程上内联轮询session-owned child work（`actor.owner.invoke`/`actor.owner.control`
等），这些路径上的non-tail链同样受`MAX_PROGRAM_CALL_DEPTH = 128`保护。driver线程
必须使用与tokio worker相同的`RUNTIME_WORKER_THREAD_STACK_SIZE_BYTES`栈预算，而不是
进程main线程的OS默认栈（约8 MiB）：debug unoptimized帧在到达guard前就可能击穿小栈，
导致整个runtime进程abort。`runtime/driver/main.rs`以该常量创建driver线程并在其上
`block_on`整个runtime驱动future。

## Exception and call-site contract

Tail chain的诊断空间必须常量有界：

- 进入trampoline前已有的真实non-tail caller stack作为固定prefix保留；
- 已消除的tail edge不append到`local_call_stack`；
- target resolution、arity、argument preparation或entry budget在某次tail edge失败时，用该edge的current
  `tailSite`即时归因；
- target body抛出的exception保留其throw/native source及non-tail prefix，不合成被消除的tail frames；
- `rethrow`复用原RequestException、catch identity、correlation、`traceId`和`errorId`；
- resource/cancellation/timeout的现有catchability不因trampoline改变；
- 不得对同一error重复调用call-site promotion而生成重复frame。

这是刻意的tail-call stack语义，不是诊断缺失。若未来需要完整logical tail history，必须使用独立、有界的
telemetry sampling；不能恢复unbounded exception stack。

## Evaluator-to-bytecode migration

树遍历evaluator存活期间，canonical和test path必须共享同一个linked evaluator/trampoline，不能增加
test-only识别。Bytecode production cutover后：

- PackageArtifact保存relocatable `tail_call_local`及exact target relocation；
- exact deployment buildId loader/linker/verifier产生唯一可执行image，不生成
  `RuntimeAssembly`或assembly/activation generation；
- malformed target或不满足eligibility的opcode fail closed，不能在runtime降级成另一projection；
- old `RuntimeExecutionProjection`/`EvalProgramProjection`与tree trampoline一起删除；
- 语义改变的aggregate value/top-level `const`/`InOut` case不以旧evaluator作为等价oracle。

## Verification and completion

### Structural and focused tests

- compiler lowering：direct self、same-file mutual、cross-module publication、generic function及impl self的
  `Return` ExprRef必须直接指向exact Call；wrapped/non-tail forms不得满足该结构。
- emitter/linker/verifier：eligible edge发射`tail_call_local`并解析为同exact-build image target；不新增
  独立tail boolean metadata/SCC convention；非法return plan/region/target、live pending/unwind state、
  `NoPending`/`InOut`或removed-slot lifetime拒绝。
- runtime positive：deployment bytecode真实路径覆盖direct self深递归、same-file和cross-module mutual；
  generic/impl self；branch return；ordered/single argument evaluation。
- runtime negative：binary/wrapper/call-argument/catch/timeout/concurrent/DB/stream defer/service/Actor/native
  不误转移；plan不等价、caller-local `InOut` base或不可证明loan transfer由emitter保留
  普通调用，损坏的opcode由verifier拒绝。
- safety：non-tail depth 128可进入、下一层以`programCallDepth`失败且runtime继续健康；现有tail-recursion
  guard fixture改成真正non-tail recursion。
- budget：无限tail recursion由小instruction limit终止，不返回depth error；有限tail loop的instruction
  accounting与对应普通调用逐hop一致。
- carrier/error：generic/self、nominal/union/representation/container carrier等价；throw/catch/rethrow
  identity与correlation保持；tail stack长度不随100,000 hop增长。
- frame/loan：argument failure留住完caller frame可unwind；commit后无live reference指向removed
  slots；incoming `InOut`结束/线性转移、caller-local loan负例与transitive `NoPending`都有
  focused verifier/runtime test。
- scheduler：所有独立callable task从fresh depth开始；stream terminal独立测试不计作TCO证据。

### Pressure and real path

至少一个runtime test在1 MiB Tokio worker stack上完成100,000次tail hop并验证结果，不能只依赖production
64 MiB stack。至少一个canonical Skiff source fixture必须经过真实
source -> compiler -> bytecode artifact -> deployment load/link/verify -> runtime路径，覆盖大于32层的self或
mutual recursion。

聚焦owner先运行：

```bash
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only skiff-tests
```

修改test registry/tooling时补跑`--only tooling`。稳定候选最终由唯一gate owner运行`pnpm verify`；不得在
开发分支重复运行full gate。

### Completion criteria

只有同时满足以下条件才算完成：

- reference全部eligible target通过同一iterative loop，100,000 hop不增长native/depth/diagnostic stack；
- tail hop保留argument、generic/self、heap carrier、budget、deadline、stop、error与Actor-local语义；
- 所有lexical/owner boundary负例保持普通路径；
- non-tail recursion在native stack耗尽前结构化失败，尾递归不受任意depth常数限制；
- deployment bytecode只有一个`tail_call_local`执行owner，无第二eligibility/trampoline或fallback projection；
- 执行owner是request钉住的exact deployment `buildId`，无`RuntimeAssembly`或generation identity；
- frame replacement为全准备后的单一commit，`PendingOperation`/`UnwindState`、removed-slot lifetime与
  `NoPending`/`InOut` loan安全均由verifier fail closed；
- compiler、runtime、Skiff source真实路径及1 MiB stack压力证据齐全；
- canonical文档与实现一致，没有语言关键字、用户配置、test-only production bypass或stream terminal混入。

## Non-goals

- 不把non-tail call误做tail replacement；non-tail call由VM显式frame stack承载。
- 不承诺PackageDirect、dynamic interface、top-level `const` receiver、service、Actor、callback、native或stream scheduler
  boundary的TCO。
- 不保留无限logical tail-call traceback。
- 不改变instruction limit、deadline、internal stop、request heap、Actor admission/suspension或stream
  terminal语义。
- 不新增源码annotation、关键字、manifest/schema、用户配置、环境变量或legacy compatibility reader。
- 不以提高worker stack、调大/调小depth常数、增加`tokio::spawn`或heap continuation代替tail-call
  recognition and elimination。
