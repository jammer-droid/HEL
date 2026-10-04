# Record Schema Changelog

## record-v0 (eval-v0, H0)

- 필드: `run`, `harness`, `model`, `outcome`, `usage`(input/output token, model 호출 수, wall time), `events`(tool 호출과 인자), `validity`, `artifacts`.
- `outcome.termination`: `completed`, `max_turns`, `max_output_tokens`(출력 token 한도에 걸려 응답이 끊김), `timeout`, `error`.
- 용도: H0의 판정(출력 정확 일치, read tool 1회 호출), token 기준 비용 기록, 응답한 model 확인.
