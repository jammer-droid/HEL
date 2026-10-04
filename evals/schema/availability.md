# Field Availability by Harness

> record-v0 기준. 값: `measured` / `derived` / `unavailable` / `?`(아직 확인 안 됨)  
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
| `events` | measured | measured (`tool_use` block) |
| `events[].ok` | measured | measured (`tool_result.is_error`) |

## 메모

- claude-code 열은 Claude Code 2.1.288 기준이다. 추출은 `crates/evals/src/claude_code.rs`의 collector와 적합성 test가 담당한다.
- `derived` 값이 생기면 계산 방법을 여기에 적는다.
