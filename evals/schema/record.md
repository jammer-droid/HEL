# Run Record — Field Reference

> 현재 버전: **record-v0** (eval-v0)  
> 기계 검증: [record.schema.json](record.schema.json) · 변경 이력: [CHANGELOG.md](CHANGELOG.md) · harness별 수집 가능 여부: [availability.md](availability.md)

collector가 run 하나마다 만드는 `results/<lab>/<run-id>/record.json`의 필드 설명서다. **수집한 사실만** 담고, 판정은 `verdict.json`에 둔다([SPEC.md](../SPEC.md) §5, §6).

필드를 추가하거나 의미를 바꿀 때는 이 문서, `record.schema.json`, `availability.md`, 각 collector와 test, `CHANGELOG.md`를 함께 갱신하고 `schema_version`을 올린다.

H9의 재시작 task(eval-v8)는 여러 프로세스를 하나의 run으로 수집한다. 프로세스별 원본 record는 보존하고, run 합계 계산법은 [availability.md](availability.md#h9-재시작-run-collector-eval-v8)에 따른다. 필드와 run 단위 의미는 그대로다.

## 표기

- **도입**: 처음 추가된 schema version
- **이유**: 어떤 Lab의 어떤 질문 때문에 필요한가
- 수치 필드(`metric`)는 `{ "value": number | null, "status": "measured" | "unavailable" | "derived" }`이다.
  - `measured`: session 기록이나 API 응답에서 직접 얻은 값
  - `derived`: 다른 값에서 계산한 값 (계산 방법을 availability.md에 적는다)
  - `unavailable`: 이 harness에서 얻을 수 없음. `value`는 `null`

## 필드

### 최상위

| 필드 | 타입 | 도입 | 설명 |
| --- | --- | --- | --- |
| `schema_version` | `"record-v0"` | v0 | record 형식 버전 |

### `run` — 실행 식별

| 필드 | 타입 | 도입 | 설명 |
| --- | --- | --- | --- |
| `run.run_id` | string | v0 | `<experiment-id>-<task-id>-<condition>-<rep>` |
| `run.experiment_id` | string | v0 | Lab 정의의 Lab ID. `revision`이 2 이상이면 `<lab>-r<N>` |
| `run.lab` | string | v0 | `hXX` |
| `run.task_id` | string | v0 | `evals/tasks/<task-id>` |
| `run.condition` | string | v0 | `baseline` / `variant` / `variant-<name>` / `external-<harness>`. `variant-<name>`은 한 Lab에서 같은 코드로 여러 설정을 비교할 때 쓴다(H1: tool 구성 비교) |
| `run.repetition` | integer ≥ 1 | v0 | 같은 (condition, task) 안에서의 반복 번호 |
| `run.started_at`, `run.ended_at` | date-time | v0 | 실행 시작/종료 시각. model 변경 추적과 peak/off-peak 구분에 사용 |

### `harness` — 실행한 harness

| 필드 | 타입 | 도입 | 설명 |
| --- | --- | --- | --- |
| `harness.name` | string | v0 | `hel`, `claude-code` 등. `evals/harnesses/<name>` |
| `harness.version` | string | v0 | `hel`은 commit, 외부 harness는 제품 버전 |
| `harness.comparison_class` | `subject` / `same-model` / `reference` | v0 | 결과 해석 등급 ([SPEC.md](../SPEC.md) §7.2) |

### `model` — 사용한 model

| 필드 | 타입 | 도입 | 설명 |
| --- | --- | --- | --- |
| `model.provider` | string | v0 | 예: `deepseek` |
| `model.requested` | string | v0 | Lab 정의에서 요청한 model ID |
| `model.actual` | string \| null | v0 | 응답에서 확인한 실제 model 이름. 요청과 다르면 run은 invalid. **이유**: provider 쪽 별칭이 다른 model로 바뀌는 것을 감지 (H0) |
| `model.params` | object | v0 | reasoning 등 실제로 보낸 설정 |

### `outcome` — 결과

| 필드 | 타입 | 도입 | 설명 |
| --- | --- | --- | --- |
| `outcome.final_output` | string \| null | v0 | harness가 사용자에게 돌려준 최종 텍스트. **이유**: `output_exact_match` check (H0) |
| `outcome.termination` | `completed` / `max_turns` / `max_output_tokens` / `timeout` / `error` | v0 | 종료 사유. `max_output_tokens`는 model 응답이 출력 token 한도에 걸려 끊긴 경우(OpenAI 호환 API의 `finish_reason: length`) |
| `outcome.error` | string \| null | v0 | `error`일 때 메시지 |

### `usage` — 비용 지표

| 필드 | 타입 | 도입 | 설명 |
| --- | --- | --- | --- |
| `usage.input_tokens` | metric | v0 | run 전체의 입력 token 합. **이유**: 비용을 token 기준으로 비교 (H0) |
| `usage.output_tokens` | metric | v0 | run 전체의 출력 token 합 (reasoning token 포함 여부는 availability.md에 기록) |
| `usage.model_calls` | metric | v0 | run 동안의 model API 호출 수 |
| `usage.wall_time_ms` | metric | v0 | run 시작부터 종료까지 걸린 시간 |
| `usage.cached_input_tokens` | metric | v0 확장 (H5) | `input_tokens` 중 provider의 prompt cache에서 처리된 token 합. **이유**: cache hit 비율과 비용 비교 (H5·H6). 없으면 `unavailable` |
| `usage.peak_context_tokens` | metric | v0 확장 (H5) | 성공한 호출 중 가장 큰 단일 요청 input. run이 한 번에 쓴 최대 context. 없으면 `unavailable` |
| `usage.last_context_tokens` | metric | v0 확장 (H5) | 마지막 성공 호출의 input. run이 끝났을 때의 context 크기. 없으면 `unavailable` |

H5 확장 필드 세 개는 선택 필드다. 이전 record에는 없으며, 읽을 때 `unavailable`로 본다. 호출별 값은 record에 넣지 않고 raw log에 둔다(`evals report`가 Runs 절에 호출별 context와 cache hit를 표시).

### `events` — tool 호출

순서대로 나열한 tool 호출 목록. tool을 쓰지 않은 run은 빈 배열이다.

| 필드 | 타입 | 도입 | 설명 |
| --- | --- | --- | --- |
| `events[].seq` | integer ≥ 1 | v0 | 호출 순서 |
| `events[].category` | `read` / `search` / `edit` / `exec` / `other` | v0 | harness 간 비교용 공통 범주. 매핑은 각 harness PROFILE에 정의 |
| `events[].name` | string | v0 | harness의 원래 tool 이름 |
| `events[].args` | object | v0 | tool 인자. **이유**: `tool_calls` check가 대상 경로를 확인 (H0) |
| `events[].ok` | boolean \| null | v0 | tool 실행 성공 여부. 알 수 없으면 `null` |

### `validity` — 유효성

| 필드 | 타입 | 도입 | 설명 |
| --- | --- | --- | --- |
| `validity.valid` | boolean | v0 | 집계에 포함할 수 있는 run인가 |
| `validity.reasons` | string[] | v0 | invalid 사유 (예: `model mismatch: requested deepseek-flash, actual ...`) |

### `artifacts` — 원본 위치

| 필드 | 타입 | 도입 | 설명 |
| --- | --- | --- | --- |
| `artifacts.raw_transcript` | string \| null | v0 | run 디렉터리 기준 원본 session 기록 사본 경로. 정성 평가와 record 재생성에 사용 |
