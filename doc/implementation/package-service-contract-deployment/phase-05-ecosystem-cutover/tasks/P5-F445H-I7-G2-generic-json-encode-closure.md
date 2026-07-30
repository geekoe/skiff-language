# P5-F445H I7 G2 generic JSON encode closure

## Parent evidence

I7 M5 ran AIHub's 51 default hermetic tests on Skiff
`b4bdbddb8761bcf053258eef5b87b778c3299b7a`. Thirty-four cases failed with
`unsupported native target std.json.encode`; the first was
`provider selection prefers body provider`.

The captured invocation was exact `std.json.encode`, with
`T0 = TypeParam(T)` and a self substitution `T -> TypeParam(T)`. Eval admitted
this as the existing plan-free encode fallback, but
`eval_native_prepared_call` called `return_plan()` before dispatch and made that
fallback unreachable.

## Scope

- Eval prepared native invocation and return materialization;
- direct JSON dispatcher regression coverage;
- focused generic encode/decode and fail-closed controls;
- this task and result record.

Do not change compiler/artifact generations, Internals, stable services, MongoDB,
or network state.

## Required behavior

1. Exact `std.json.encode` with no native plan reaches the JSON dispatcher's
   dynamic encoder.
2. Its fixed builtin `string` result is materialized without inventing a plan;
   any non-string result fails closed.
3. Concrete encode plans keep their existing path.
4. `std.json.decode`, unknown targets, and all other natives still require a
   plan.
5. Existing local, package-symbol, nested-container generic encode and generic
   decode controls remain green.

## Gate

Run focused Eval/native tests, full Eval tests, Eval tests check, formatting,
and diff checks. Then hand the commit to the I7 M owner to rerun the exact AIHub
51-case hermetic suite.
