# P5-F237 Relay streaming UTF-8 buffering

## Context

After P5-F235, Relay canonical test
`responses sse usage parser buffers split utf8 chunks` executes and fails with:

```text
std.bytes.DecodeError:
bytes.toUtf8String decode failed: incomplete utf-8 byte sequence from index 0
```

The first transport chunk contains `e4`; the continuation chunk contains
`b8ad0a`. Production Relay decodes the first chunk before a complete UTF-8
sequence exists.

## Required implementation

- Buffer raw bytes across transport chunks.
- Decode only a complete protocol unit at the parser boundary; do not assume a
  transport chunk is valid standalone UTF-8.
- Preserve multiple events per chunk, events split across chunks, trailing
  partial data, EOF, and existing error behavior.
- Keep the fix in the Relay parser. Do not weaken `bytes.toUtf8String`, replace
  invalid data, or special-case the test bytes.

## Acceptance

- Focused tests cover a complete UTF-8 chunk, one code point split across at
  least two chunks, multiple protocol events in one chunk, a delimiter split
  across chunks, invalid completed UTF-8 and EOF with an incomplete sequence.
- The canonical split-UTF-8 Relay case passes through the real test runner.
- Run the complete Relay file and service suites, recording every remaining
  independent failure with exact source location and compiler/runtime target.
- Relevant Internals checks and diff check pass.
- Commit the Internals change; no push, stable-instance operation, or disk
  cleanup.
