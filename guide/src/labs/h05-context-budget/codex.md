# Codex의 context budget

> [!NOTE]
> 확인한 소스: [openai/codex](https://github.com/openai/codex) commit `afb436d` (2026-10-04). Rust로 작성된 `codex-rs` 기준이다.

## 현재 context 측정

마지막 API 응답이 알려 준 token 수에, 그 응답 뒤에 추가된 기록(tool 결과 등)의 크기를 더한다. 추가분은 tokenizer 없이 model에게 보이는 bytes를 4로 나눠 추정한다([history.rs `get_total_token_usage`](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/core/src/context_manager/history.rs#L922)).

## window와 압축 시작 기준

| 값 | 기본 | 출처 |
| --- | --- | --- |
| 사용 가능 window | model의 context window × 95% (`effective_context_window_percent`) | [openai_models.rs](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/protocol/src/openai_models.rs#L515) |
| 자동 압축 기준 | 설정값 `model_auto_compact_token_limit`과 window의 90% 중 작은 값 | [openai_models.rs](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/protocol/src/openai_models.rs#L526) |
| model 정보가 없을 때 | window 272,000 token | [model_info.rs](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/models-manager/src/model_info.rs#L115) |

기준은 대화 전체에 적용하거나, 앞선 압축 이후에 늘어난 부분에만 적용하도록 고를 수 있다([context_window.rs](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/core/src/session/context_window.rs)).

## 압축하는 시점

- turn을 시작하기 전에 기준을 넘었으면 압축한다.
- model이 tool을 호출해 다음 요청이 이어져야 하는데 기준을 넘었으면, 그 사이에 압축한다.
- turn이 끝날 때 window의 N%를 넘었으면 압축하도록 설정할 수 있다(기본은 꺼짐).
- 사용 가능 window에 닿으면 기준과 상관없이 압축한다.

([turn.rs](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/core/src/session/turn.rs#L1304))

## 압축할 때 남기는 것

대화 전체 뒤에 [요약 지시](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/prompts/templates/compact/prompt.md)를 붙여 같은 model에 보내고, 마지막 응답을 요약으로 쓴다. 지시는 진행 상황과 결정, 제약, 남은 일, 이어서 작업하는 데 필요한 데이터를 담은 인계 요약을 요구한다.

새 기록은 다음으로 다시 만든다([compact.rs](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/core/src/compact.rs#L656)).

```text
[초기 context] [최근 user 메시지들 (최신부터 최대 20,000 token)] [요약]
```

model의 응답과 tool 결과는 남기지 않는다. 요약 앞에는 "다른 model이 작업을 시작했고 이 요약을 남겼다"는 안내문을 붙인다. OpenAI provider에서는 서버의 `/responses/compact`로 압축하고, 다른 provider에서는 위의 방식을 쓴다.

## tool 출력 상한

model마다 정한 상한(기본 10,000 bytes)을 넘으면 가운데를 잘라 앞과 뒤를 남기고, 원래 token 수와 전체 줄 수를 앞에 적는다([output-truncation](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/utils/output-truncation/src/lib.rs#L23)).

```text
Warning: truncated output (original token count: N)
Total output lines: M
```

## cache

- `prompt_cache_key`에 session ID를 넣어 같은 대화의 요청이 같은 cache를 쓰게 한다([client.rs](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/core/src/client.rs#L580)).
- 실행 환경이나 설정이 바뀌면 앞쪽 메시지를 고치지 않고, 바뀐 부분만 새 메시지로 뒤에 덧붙인다(`context/world_state`).
- tool 출력은 기록에 들어갈 때 이미 상한 안으로 줄어 있어서, 그 뒤로는 고칠 일이 없다.
- 압축하면 기록 전체를 다시 만들기 때문에 cache도 처음부터 다시 쌓인다.

## 실험 기능: token budget

`features.token_budget`을 켜면 압축까지 남은 token이 정한 값 아래로 내려갈 때 model에게 알리고, model이 `get_context_remaining` tool로 남은 양을 묻거나 `new_context_window` tool로 새 window를 요청할 수 있다. 현재는 개발 중인 기능이고 기본으로 꺼져 있다([features](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/features/src/lib.rs#L1793), [config](https://github.com/openai/codex/blob/afb436df8b70bb5bc57b86d9a3e829968988cd21/codex-rs/core/src/config/mod.rs#L1210)).
