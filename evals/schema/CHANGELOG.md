# Record Schema Changelog

## record-v0 유지 (eval-v8, H9, 2026-10-05)

- 프로세스 재시작 task도 두 지시를 합쳐 run 하나로 수집. 새 필드·필드 의미 변경 없음, schema 변경 없음. stage 원본을 보존하고 합계·최댓값은 derived, 전체 시간은 measured로 기록. 계산법은 availability.md. collector 적합성 test에서 기존 schema로 검증.
- task의 recall_tokens·recall_token check 및 session_mode는 eval 명세 확장. 기존 Lab에는 적용하지 않으며 과거 record 재생성·재측정 불필요.

## record-v0 호환 확장 (H5, 2026-10-04)

- `usage`에 선택 필드 `cached_input_tokens`, `peak_context_tokens`, `last_context_tokens`(metric)를 추가한다. H5에서 cache hit 비율과 호출 시점의 context 크기를 run 단위로 비교하고, H6 compaction 비교에 같은 값을 쓰기 위해서다.
- 필수 필드가 아니어서 기존 record는 그대로 유효하다. 읽을 때 없으면 `unavailable`로 본다(`record` crate의 serde default). `schema_version`은 `record-v0` 유지. 과거 record 재생성 불필요.
- 호출별 context·cache hit는 raw log에 두고 `evals report`가 Runs 절에 표시한다.
- availability: hel은 세 필드 모두 measured. claude-code는 `cached_input_tokens`만 measured(`cache_read_input_tokens`), 나머지 unavailable.

## record-v0 호환 확장 (eval-v1, H1, 2026-10-04)

- `run.condition`에 `variant-<name>`을 허용한다(pattern 확장). H1에서 같은 `hel` 코드로 tool 구성(bash만 / bash + read_file)을 나눠 비교하기 위해서다.
- 허용 값을 넓히기만 해서 기존 record는 모두 그대로 유효하다. `hel`의 시작 상태(`h01`)가 `record-v0`을 쓰므로 baseline을 같은 코드로 측정할 수 있게 `schema_version`은 올리지 않는다. 과거 record 재생성 불필요.
- availability 변화 없음(값을 runner가 정해 넘긴다).

## record-v0 (eval-v0, H0)

- 필드: `run`, `harness`, `model`, `outcome`, `usage`(input/output token, model 호출 수, wall time), `events`(tool 호출과 인자), `validity`, `artifacts`.
- `outcome.termination`: `completed`, `max_turns`, `max_output_tokens`(출력 token 한도에 걸려 응답이 끊김), `timeout`, `error`.
- 용도: H0의 판정(출력 정확 일치, read tool 1회 호출), token 기준 비용 기록, 응답한 model 확인.
