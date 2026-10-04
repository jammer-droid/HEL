# Record Schema Changelog

## record-v0 호환 확장 (eval-v1, H1, 2026-10-04)

- `run.condition`에 `variant-<name>`을 허용한다(pattern 확장). H1에서 같은 `hel` 코드로 tool 구성(bash만 / bash + read_file)을 나눠 비교하기 위해서다.
- 허용 값을 넓히기만 해서 기존 record는 모두 그대로 유효하다. `hel`의 시작 상태(`h01`)가 `record-v0`을 쓰므로 baseline을 같은 코드로 측정할 수 있게 `schema_version`은 올리지 않는다. 과거 record 재생성 불필요.
- availability 변화 없음(값을 runner가 정해 넘긴다).

## record-v0 (eval-v0, H0)

- 필드: `run`, `harness`, `model`, `outcome`, `usage`(input/output token, model 호출 수, wall time), `events`(tool 호출과 인자), `validity`, `artifacts`.
- `outcome.termination`: `completed`, `max_turns`, `max_output_tokens`(출력 token 한도에 걸려 응답이 끊김), `timeout`, `error`.
- 용도: H0의 판정(출력 정확 일치, read tool 1회 호출), token 기준 비용 기록, 응답한 model 확인.
