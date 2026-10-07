# Field Availability by Harness

> record-v0 기준(H13 `events[]` 선택 필드 포함). 값: `measured` / `derived` / `unavailable` / `?`(아직 확인 안 됨)  
> collector를 만들거나 record 필드를 추가할 때 이 표를 함께 갱신한다.

| 필드 | hel | claude-code |
| --- | --- | --- |
| `model.actual` | measured (응답의 `model`) | measured (stream-json assistant message의 `model`) |
| `outcome.termination` | measured (`finish_reason`, HTTP 오류, timeout) | measured (`result.subtype`, `stop_reason`, runner timeout) |
| `outcome.final_output` | measured | measured (`result.result`) |
| `usage.input_tokens` | measured (응답의 `usage.prompt_tokens` 합) | measured (`result.usage`의 input + cache read + cache creation) |
| `usage.output_tokens` | measured (`completion_tokens` 합, reasoning 포함) | measured (`result.usage.output_tokens`) |
| `usage.model_calls` | measured (HTTP 요청 수) | derived (assistant message의 고유 `id` 수) |
| `usage.wall_time_ms` | measured | measured (runner가 프로세스 시간 측정) |
| `usage.cached_input_tokens` | measured (`usage.prompt_cache_hit_tokens` 합. 한 응답이라도 없거나 호출이 실패하면 unavailable) | measured (`result.usage.cache_read_input_tokens`) |
| `usage.peak_context_tokens` | measured (성공 호출 `prompt_tokens`의 최댓값) | unavailable (stream-json의 요청별 usage 미수집) |
| `usage.last_context_tokens` | measured (마지막 성공 호출의 `prompt_tokens`) | unavailable (같은 이유) |
| `events` | measured | measured (`tool_use` block) |
| `events[].ok` | measured | measured (`tool_result.is_error`) |
| `events[].exit_code` | measured (`bash` 결과 첫 줄 `exit=<code>`) | unavailable (Bash tool 결과에 exit code가 따로 없음) |
| `events[].error` | measured (실패한 호출의 결과 텍스트, `record::error_excerpt`) | unavailable (미수집) |

## 메모

- claude-code 열은 Claude Code 2.1.288 기준이다. 추출은 `crates/evals/src/claude_code.rs`의 collector와 적합성 test가 담당한다.
- `derived` 값이 생기면 계산 방법을 여기에 적는다.


## H9 재시작 run collector (eval-v8)

hel의 각 stage record는 위 표와 같다. run 전체를 합칠 때 input/output/cached token·model_calls는 stage 합(derived), peak_context는 stage 최댓값(derived), last_context는 마지막 stage 값, wall_time은 전체 프로세스 경과시간(measured)이다. 필요한 stage 값이 unavailable이면 합계·최댓값도 unavailable이다. 원본 stage record는 보존하며, 단일 프로세스의 기존 수집은 바꾸지 않는다.
