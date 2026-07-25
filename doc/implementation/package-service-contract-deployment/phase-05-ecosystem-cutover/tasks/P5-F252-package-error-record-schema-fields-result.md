# P5-F252 Package error record schema fields result

## Result

Completed. The failure was not caused by `ErrorPayload` records losing their
fields. It was caused by nested Package public paths being qualified from the
last dot instead of the dependency root.

For the source type
`llmProviders.chatgptPlan.OauthSession`, the compiler incorrectly derived the
Package root `llmProviders.chatgptPlan`. It then qualified the public field type
`chatgptPlan.OauthError` as
`llmProviders.chatgptPlan.chatgptPlan.OauthError`. The source diagnostic
displayed this as `chatgptPlan.chatgptPlan.OauthError`, so field access failed
with `unknown field code/message`.

`package_root_from_type_name` now takes the segment before the first dot. The
same field therefore resolves to the exact public type
`llmProviders.chatgptPlan.OauthError`.

## Exact evidence

The inspected llm-providers source was commit
`815d407d25d2ff590e8952c960a826182d6ab509`. Its API exports
`chatgptPlan.OauthError` as
`chatgpt_plan.model.ChatgptPlanOauthError`, whose source declaration contains
the fields `code: string` and `message: string`.

The failing and passing compiler runs used the same exact retained graph:

- llm-api build:
  `82dbeb3098cad2bb4998c7c25e4a90a4e32ca23054feebc06a97ae9f39f847a2`
- llm-api ABI:
  `bd6a9b390bb51496299f6668283be4ec402e20d96bafd10eca876fbb7e4ef0a3`
- llm-providers build:
  `f40e885642aaa681b2e892570245082f7f4de0a28cf459e584eb34991f0c27cc`
- llm-providers ABI:
  `d41669ca811565ced55a53d9676807f53e0f80f7d76f8fd7110b0bf7f90a3a54`
- llm-providers schema index:
  `fa89c8a593b16009c4d75711b2e3186b863079d319d8bcc42d47c044dd75802f`
- `chatgptPlan.OauthError` schema type ID:
  `2df001721c2725b526829d0eaf0c92d712b1ee45fa530589d63825217f8dffa4`

Both the PackageArtifact implementation descriptor and the canonical stored
schema descriptor contain `code` and `message`. Thus declaration, schema
closure and artifact storage were already correct and exact; the defect was
only in dependency source projection. The existing exact schema storage tests
continue to reject missing, mismatched and tampered records rather than
substituting another shape.

After the fix, fresh Relay compilation against that exact std -> llm-api ->
llm-providers store crossed both field accesses and produced:

- Relay build:
  `117af71a55a29338fd63d0330e38dd4166cbee8e8c10e09043dd4690c56f8f15`
- Relay ABI:
  `a219ebd81a60894c766cd81092dbc6cac13c6f8d1510ddc62aeca6b803cf3d58`
- Service API projection: 17/17 functions available

## Coverage

Added focused coverage for:

- deriving only the dependency root from a nested Package type;
- qualifying a cross-Package nested record field exactly once;
- preserving fields on both an ordinary public record and a record that
  implements `ErrorPayload`.

Validation completed:

- `cargo test -p skiff-compiler-source`: 280 passed
- `cargo test -p skiff-compiler --test package_std_schema`: 8 passed
- focused Package artifact tamper test: 1 passed
- `cargo test -p skiff-deployment`: 52 passed
- `cargo test -p skiff-compiler-projection-input`: 7 passed
- F245 Relay projection suite with the F249 runtime: 23 passed, 0 failed
- `cargo check --workspace`: passed
- `git diff --check`: passed

The first Relay test launch did not enter the test suite because the isolated
Router worktree lacked installed local dependencies (`tsx` was missing).
Installing the lockfile offline and rerunning the identical command produced
the recorded 23/23 result.
