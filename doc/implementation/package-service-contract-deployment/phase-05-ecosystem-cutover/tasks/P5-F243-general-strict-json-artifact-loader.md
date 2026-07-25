# P5-F243 General strict JSON artifact loader

## Context

Relay full-suite activation rejects a captured, otherwise valid FileIR record:

```text
generation number must use canonical unsigned integer syntax at JSON offset 12270
```

The byte at that offset belongs to a valid FileIR number literal
`{"kind":"number","value":0.1}`. The filesystem artifact loader reuses
`strictActivationJson` for every artifact record, and that parser applies the
generation-number restriction to every JSON number.

## Required implementation

- Separate general strict JSON parsing from activation generation validation.
- General artifact JSON accepts canonical JSON negative numbers, fractions and
  exponents.
- Preserve strict UTF-8, duplicate-key, surrogate, trailing-input and malformed
  number rejection.
- Activation generation fields remain unsigned safe integers with canonical
  syntax, validated at their field boundary.
- Do not weaken activation protocol validation or special-case FileIR.

## Acceptance

- Loader regression covers FileIR `0.1`, negative and exponent values.
- Negative tests cover leading zero, malformed fraction/exponent, duplicate
  key, invalid UTF-8/surrogate and trailing input.
- Activation generation negative/fraction/exponent/unsafe-integer tests remain
  rejected.
- Relay full service crosses the captured FileIR activation.
- Router tests/type-check, relevant integration tests, result and commit.
- No push, stable operation or disk cleanup.
