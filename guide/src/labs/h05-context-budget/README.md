# H5 — Context Budget

> [!NOTE]
> - 시작 상태: [`h05`](https://github.com/jammer-droid/HEL/tree/h05) · 완료 상태: [`h06`](https://github.com/jammer-droid/HEL/tree/h06)
> - 논문: [§9.5 Threshold Compaction](https://arxiv.org/html/2609.00006v1#S9.SS5), [§16.5 Memory and Context](https://arxiv.org/html/2609.00006v1#S16.SS5), [§16.10 A Minimum Viable Harness](https://arxiv.org/html/2609.00006v1#S16.SS10)

```bash
git checkout -b my-h05 h05
```

Codex와 DeepSeek Harness는 context budget을 어떻게 측정·배분하고, 한도에 가까워지면 무엇을 남기고 버리는가? 각 방식의 장단점은 무엇인가?

## 들어가며

### 논문이 본 context budget

[Part III 개요](../../parts/part3-context.md)에서 본 것처럼 논문이 조사한 시스템의 대부분은 context 사용량이 정해 둔 제한에 도달하면 compaction(컨텍스트 압축)을 한다. 오래된 대화를 model이 쓴 요약으로 바꾸는 방식이다. 그 지점을 정하려면 두 가지가 먼저 필요하다. 지금 context를 얼마나 쓰는지 측정하는 방법과, model의 context window 중 어디까지를 대화 기록에 쓸지 정한 기준이다.

[§9.5](https://arxiv.org/html/2609.00006v1#S9.SS5)에 정리된 기준은 시스템마다 다르다. Claude Code는 window보다 13,000 token 아래, Gemini CLI는 window의 50%, OpenCode는 input 한도에서 출력 몫을 뺀 지점에서 압축을 시작한다. 사용량을 재는 방법도 다르다. OpenCode는 tokenizer 없이 글자 수를 4로 나누고, Gemini CLI는 API가 돌려준 token 수로 추정값을 보정한다. Codex는 기준과 별개로 thread마다 token budget을 두고 남은 양을 model에게 알린다(현재 Codex 소스에서는 기본으로 꺼진 실험 기능이다).

[Recommendation 7](https://arxiv.org/html/2609.00006v1#S16.SS5)은 window보다 일정량 아래에서 압축을 시작하고, 최근 기록은 원문으로 남기라고 권한다. [§16.10](https://arxiv.org/html/2609.00006v1#S16.SS10)의 최소 harness 예시는 tool 출력을 25,000자에서 자르고, 글자 수로 추정한 token이 120,000을 넘으면 압축하며 최근 30%를 남긴다. 이 수치들은 각 시스템이 정한 값이고, 논문은 어떤 값이 더 나은지 비교하지 않았다. 그래서 이번 Lab에서는 실제 harness가 이 값을 어떻게 정하고 무엇을 남기는지 소스에서 직접 확인한다.

### 이번 Lab에서 다룰 context budget

context budget(컨텍스트 예산)은 model의 context window를 어디에 얼마나 사용할지 분배하는 예산을 의미한다. 매 요청에는 system prompt와 tool 정의처럼 고정된 부분, 지금까지의 대화 기록, model이 이번에 쓸 출력 자리가 함께 들어간다. 대화 기록이 늘어나면 남은 공간이 줄고, 한도에 도달하면 harness는 기록 일부를 줄이거나 압축해야 한다.

이번에는 Codex와 DeepSeek Harness의 소스를 읽고 다음을 비교한다.

- 지금 사용 중인 context를 어떻게 측정하는가
- window 중 어디까지를 쓰고, 어디서 압축을 시작하는가
- 큰 tool 출력을 어떻게 줄이는가
- 압축할 때 무엇을 원문으로 남기고 무엇을 버리는가

두 harness는 모두 소스가 공개되어 있다. Codex는 Rust로 작성되어 `hel`과 구조를 비교하기 쉽고, DeepSeek Harness는 `hel`과 같은 DeepSeek API를 사용한다. 압축을 실제로 구현하고 실행해 보는 것은 H6에서 하고, 이번에는 `hel`이 호출마다 사용한 context 크기와 cache가 적용된 token을 기록하도록 만든다. H6에서 압축을 구현한 뒤 그 전과 같은 기준으로 비교하려면 이 기록이 먼저 필요하다.

### 참고 자료

- [Codex의 context budget](codex.md): 측정, 압축 기준과 시점, 남기는 것, tool 출력 상한, cache (확인한 commit `afb436d`)
- [DeepSeek Harness의 context budget](deepseek-harness.md): 같은 항목 (확인한 commit `5badb15`)
- **Claude Code**는 소스가 공개되어 있지 않아 [공식 문서](https://code.claude.com/docs/en/costs)로만 확인했다. 한도 근처에서 자동으로 압축하고, `/compact`에 남길 내용을 지시할 수 있으며, `/context`와 `/usage`로 사용량과 cache 통계를 보여 준다(2026-10-04 확인).

## 이번에 해볼 것

다른 harness의 방식을 보기 전에 `hel`이 지금 어떤 상태인지부터 확인한다. `hel`은 DeepSeek API의 `deepseek-flash` model을 쓴다. 이 model의 context window와 가격, API가 돌려주는 token 정보를 확인하고, 실제 실행 기록에서 호출마다 context가 어떻게 늘어나는지 본다.

그다음 `hel`이 호출마다 사용한 context 크기와 cache가 적용된 token을 실행 기록에 남기도록 만든다. 마지막으로 Codex와 DeepSeek Harness가 같은 문제를 어떻게 다루는지 비교한다.

## 결과 확인

### 1. deepseek-flash의 사양과 가격

[DeepSeek 가격 페이지](https://api-docs.deepseek.com/quick_start/pricing)에 나온 `deepseek-flash`의 사양은 다음과 같다(2026-10-04 기준).

| 항목 | 값 |
| --- | --- |
| model 버전 | DeepSeek-V4.1-Flash |
| context window | 1M token |
| 최대 출력 | 384K token |
| thinking mode | 기본 사용, 끌 수 있음 |
| tool 호출 | 지원 |

가격은 1M token 단위이고, input은 cache(캐시) 적용 여부에 따라 나뉜다.

| 구분 | 할인 시간대 | 기본 시간대 |
| --- | --- | --- |
| input (cache hit) | $0.003 | $0.006 |
| input (cache miss) | $0.15 | $0.3 |
| output | $0.6 | $1.2 |

- 기본 시간대는 평일 UTC 01:00–04:00, 06:00–10:00이고, 나머지 시간과 주말은 할인 시간대다.
- cache hit input은 cache miss input의 50분의 1 가격이다. 같은 대화 기록을 매 호출 다시 보내는 agent loop에서는 앞부분이 cache에 hit했는지가 비용을 크게 바꾼다.
- 최대 context window가 1M token이라 Lab에서 진행하는 task는 이 한도에 한참 못 미친다.

### 2. 요청과 응답의 구조

H4에서 저장한 실행 기록에서 요청과 응답 하나를 꺼내 실제 DeepSeek이 어떤 응답을 하는지 확인할 필요가 있다. 아래는 본문 검색 작업의 첫 호출이다. (긴 문자열은 줄였고, 실행 디렉터리 경로는 `/path/to/run`으로 바꿔 적었다.)

`hel`이 보내는 요청에는 model 이름, 출력 한도, tool 정의, 메시지 목록이 들어간다.

```json
{
  "model": "deepseek-flash",
  "max_tokens": 8192,
  "tools": [{ "type": "function", "function": { "name": "bash", "...": "..." } }],
  "messages": [
    { "role": "system", "content": "Environment:\n- OS: macos ..." },
    { "role": "user", "content": "Find the default RETRY_LIMIT used by ..." }
  ]
}
```

응답은 다음과 같다.

```json
{
  "model": "deepseek-flash",
  "choices": [{
    "finish_reason": "tool_calls",
    "message": {
      "role": "assistant",
      "content": "",
      "reasoning_content": "Let me explore the repository.",    // model이 추론한 내용
      "tool_calls": [
        { "function": { "name": "bash", "arguments": "{\"command\": \"cd /path/to/run && ls -la && echo \\\"---\\\" && ls src/\"}" } },
        { "function": { "name": "bash", "arguments": "{\"command\": \"cd /path/to/run && grep -rn \\\"RETRY_LIMIT\\\" . 2>/dev/null\"}" } }
      ]
    }
  }],
  "usage": {
    "prompt_tokens": 462,                                   // 요청에 들어간 input 전체
    "prompt_cache_hit_tokens": 0,                           // cache hit에 성공한 token
    "prompt_cache_miss_tokens": 462,                        // cache hit에 실패한 token
    "prompt_tokens_details": { "cached_tokens": 0 },        // OpenAI 형식으로 cache hit token 제공
    "completion_tokens": 186,                                   // 출력에 들어간 token
    "completion_tokens_details": { "reasoning_tokens": 6 },     // 출력에 사용된 reasoning token
    "total_tokens": 648
  }
}
```

- `reasoning_content`는 thinking mode에서 model이 답하기 전에 쓴 내용이다.(= 추론 과정) 이 token은 `completion_tokens` 안에 `reasoning_tokens`로 포함되어 output 가격으로 계산된다.
- `prompt_tokens`는 이번 요청에 들어간 input 전체이고, `prompt_cache_hit_tokens`와 `prompt_cache_miss_tokens`로 나뉜다. `prompt_tokens_details.cached_tokens`는 OpenAI 형식으로 같은 값을 한 번 더 준다.
- 매 요청에는 대화 기록 전체가 들어가므로 `prompt_tokens`가 그 호출 시점의 context 크기다.

### 3. 호출마다 늘어나는 context와 cache

같은 실행의 세 호출을 이어서 보면 다음과 같다.

| 호출 | 보낸 메시지 | prompt_tokens | cache hit | cache miss | 출력 |
| --- | --- | --- | --- | --- | --- |
| 1 | system, user | 462 | 0 | 462 | 186 |
| 2 | + assistant, tool 결과 2개 | 4,409 | 640 | 3,769 | 125 |
| 3 | + assistant, tool 결과 1개 | 4,591 | 4,352 | 239 | 2 |

- 첫 호출에서 model은 디렉터리 목록과 저장소 전체 `grep`을 한 번에 요청했다. `grep` 결과가 9,983 bytes여서 두 번째 호출의 context가 462에서 4,409 token으로 늘었다.
- 두 번째 호출에서는 앞부분 640 token만 cache hit에 성공했고, 새로 붙은 tool 결과는 모두 cache miss였다. 세 번째 호출에서는 두 번째 호출까지의 내용 4,352 token이 cache hit에 성공했다. H4 기록 65개 호출의 cache hit 값은 모두 128의 배수였다.
- 세 번째 호출의 출력은 `7` 하나였고, 이 실행은 정답으로 끝났다.


### 4. context window의 구성

`deepseek-flash`의 context window는 1M token이고, 한 요청의 input과 output을 합친 길이가 이 안에 들어가야 한다([API 문서](https://api-docs.deepseek.com/api/create-chat-completion)의 `max_tokens` 설명). `max_tokens`는 output의 상한이고 1에서 384K 사이로 정한다. 정하지 않으면 thinking mode에서 64K가 기본이다. `hel`은 매 요청에 `max_tokens: 8192`를 보낸다.

한 요청이 window를 쓰는 모양은 다음과 같다.

```text
|<──────────────────────────── context window 1M token ─────────────────────────────>|
|<────────────────── input (prompt_tokens) ──────────────────────>|<── output 자리 ──>|
| tool 정의 | system | user | assistant | tool 결과 | ... ... ... | | reasoning + 답    |
|<──── 매 요청 같은 앞부분 ────>|<──── 호출마다 뒤에 붙는 대화 기록 ──────>| | max_tokens 8,192 |
```

- **앞부분**: tool 정의, system prompt(실행 환경, `HEL.md`), 사용자 지시. 한 실행 안에서는 매 요청 같다.
- **대화 기록**: model의 응답(assistant)과 tool 결과가 호출마다 뒤에 붙는다. tool을 쓰는 요청에서는 이전 응답의 `reasoning_content`도 context에 다시 들어간다([thinking mode 문서](https://api-docs.deepseek.com/guides/thinking_mode)). `hel`은 응답 message를 그대로 돌려보내므로 추론 내용도 다음 input이 된다.
- **output 자리**: 이번 응답의 reasoning과 답이 쓰는 자리다. input이 커져서 input + `max_tokens`가 window를 넘으면 요청이 거절된다.

§3의 세 번째 호출을 이 그림에 넣으면 input 4,591 token에 output 2 token이고, 그중 앞부분은 tool 정의·system·user를 합친 첫 호출의 462 token이다. 나머지 4,129 token은 두 번의 호출에서 붙은 대화 기록이다.(window 1M에 비하면 0.5%도 쓰지 않았다.)

### 5. harness가 조절할 수 있는 것

window의 크기와 가격은 model이 정한다. harness가 조절할 수 있는 것은 매 요청에 무엇을 얼마나 넣는지와, 그 내용이 cache hit에 성공하는지다.

**cache hit는 앞부분이 그대로일 때만 성공한다.** DeepSeek의 [context caching](https://api-docs.deepseek.com/guides/kv_cache)은 기본으로 켜져 있고, 요청마다 user 입력이 끝나는 지점과 model 출력이 끝나는 지점을 cache 단위로 저장한다. 그리고 다음 요청이 그 단위 전체와 앞부분부터 똑같아야 hit로 계산된다. 저장에는 몇 초가 걸리고, 쓰지 않으면 몇 시간에서 며칠 사이에 지워진다. 조건이 맞아도 hit가 항상 보장되지는 않는다.

§3의 두 번째 호출 cache hit 640 token은 첫 호출의 input 462와 output 186을 합친 648과 비슷하다. 앞 요청의 응답까지 하나의 cache 단위로 저장되고, 대화 기록을 뒤에 덧붙이기만 하면 그 단위가 다음 요청의 앞부분과 일치한다. 반대로 앞쪽의 한 글자라도 바뀌면 그 지점부터 다시 cache miss가 된다.

H4에서 기록한 65개 호출로 보면 다음과 같다.

| 호출 | input 합계 | cache hit | hit 비율 |
| --- | --- | --- | --- |
| 실행의 첫 호출 (18개) | 11,697 | 0 | 0% |
| 두 번째 이후 호출 (47개) | 107,334 | 69,888 | 65% |
| 전체 | 119,031 | 69,888 | 59% |

첫 호출은 18번 모두 cache hit에 실패했다. system prompt에 들어가는 작업 디렉터리 경로가 실행마다 달라서 앞부분이 실행끼리 같지 않은 것이 원인 중 하나로 보인다. 같은 실행 안의 다음 호출들은 앞 요청을 이어 붙였기 때문에 input의 65%가 cache hit였다.

**새로 붙는 내용은 한 번은 cache miss다.** 큰 tool 결과는 처음 들어갈 때 miss 가격으로 계산되고, 그 뒤 호출에서는 hit 가격으로 계속 실려 간다. hit 가격이 miss의 50분의 1이어도 window는 그대로 차지한다. 그래서 harness가 다룰 지점은 두 가지다.

- **cache hit 비율**: 앞부분을 바꾸지 않고 뒤에 덧붙인다. 실행마다 달라지는 정보를 앞쪽에 두면 실행 사이의 cache를 잃는다.
- **input 크기**: 한 번에 들어오는 tool 결과를 제한하고, 쌓인 기록을 줄일 시점을 정한다. 다만 오래된 기록을 지우거나 압축하면 바뀐 지점부터 cache를 다시 만들어야 한다.

두 지점은 서로 부딪친다. input을 줄이려고 기록을 고치면 cache hit 비율이 떨어진다. Codex와 DeepSeek Harness가 이 둘을 어떻게 조절하는지는 다음에 비교한다.

### 6. `hel`의 실행 기록에 context와 cache 남기기

§3의 값은 raw log를 직접 열어서 계산했다. 현재 실행 기록(`record.json`)에는 input 합계만 있어서, 실행끼리 cache hit나 context 크기를 비교하려면 매번 raw log를 다시 읽어야 했다. 그래서 실행 기록의 `usage`에 세 값을 추가했다.

| 필드 | 값 |
| --- | --- |
| `cached_input_tokens` | 응답마다 받은 `prompt_cache_hit_tokens`의 합 |
| `peak_context_tokens` | 가장 큰 `prompt_tokens`. 실행 중 한 번에 쓴 최대 context |
| `last_context_tokens` | 마지막 호출의 `prompt_tokens`. 실행이 끝났을 때의 context |

세 값은 사용자 지시 하나를 처리하는 실행 안에서 계산한다. 실행 하나는 model 호출 여러 번으로 이루어지고, 호출마다 `prompt_tokens`와 cache hit를 받는다.

```text
                       호출 1   호출 2   호출 3          실행 기록 usage
prompt_tokens             462    4,409    4,591
  ├ cache hit               0      640    4,352  ─ 합 ──▶ cached_input_tokens   4,992
  └ cache miss            462    3,769      239

prompt_tokens 합계 ──────────────────────────────▶ input_tokens          9,462
prompt_tokens 최댓값 ────────────────────────────▶ peak_context_tokens   4,591
마지막 호출의 prompt_tokens ─────────────────────▶ last_context_tokens   4,591
```

`input_tokens`와 `cached_input_tokens`는 비용을 보는 값이다. 같은 앞부분을 호출마다 다시 보낸 양까지 모두 더하므로 호출이 늘수록 커진다. `cached_input_tokens`를 `input_tokens`로 나누면 그 실행의 cache hit 비율이 된다(위 실행은 53%).

`peak_context_tokens`와 `last_context_tokens`는 window를 얼마나 차지했는지 보는 값이다. peak는 실행 중 input이 가장 컸던 순간이고, `peak + max_tokens`가 window 안에 들어야 요청이 거절되지 않는다. last는 마지막으로 성공한 요청의 input이다. 마지막 응답은 들어 있지 않으므로, 대화를 이어 가면 다음 요청은 last에 마지막 응답과 새 입력을 더한 크기가 된다. 실패한 호출은 token 수를 받지 못해 계산에서 빠진다.

지금 `hel`은 기록을 뒤에 덧붙이기만 하므로 마지막 호출이 가장 크고 peak와 last가 같다. 둘이 갈라지는 것은 기록을 줄일 때다.

```text
호출      1      2      3      4 (압축 뒤)    5
input    462  4,409  4,591    1,200        1,350
                     ▲ peak                    ▲ last
```

(압축 뒤의 수치는 설명을 위한 예시다.) peak로는 압축 전에 window를 얼마나 썼는지, last로는 압축 뒤 얼마나 줄어든 상태로 끝났는지 본다.

`hel`은 응답을 받을 때마다 `prompt_tokens`와 cache hit를 순서대로 쌓는다.

```rust
pub fn exchange(&mut self, exchange: &Exchange) {
    self.model_calls += 1;
    match &exchange.response.usage {
        Some(usage) => {
            self.input_tokens += usage.prompt_tokens;
            self.output_tokens += usage.completion_tokens;
            self.contexts.push(Context {
                tokens: usage.prompt_tokens,
                cache_hit: usage.prompt_cache_hit_tokens,
            });
        }
        None => self.usage_complete = false,
    }
    // (중략: 응답 model 기록, raw log 저장)
}
```

`prompt_cache_hit_tokens`는 DeepSeek가 주는 필드라서 `Option`으로 받는다. 한 응답이라도 이 값이 없거나 호출이 실패하면 합계를 믿을 수 없으므로 `cached_input_tokens`를 `unavailable`로 남긴다.

```rust
fn cached_metric(&self) -> Metric {
    let hits: Option<u64> = self.contexts.iter().map(|c| c.cache_hit).sum();
    match hits {
        Some(hits) if self.usage_complete => measured(hits as f64),
        _ => Metric::unavailable(),
    }
}
```

호출별 값은 실행 기록에 넣지 않고 raw log에 둔다. `evals report`가 raw log를 읽어 실행마다 한 줄로 보여 준다. H4의 기록에 실행하면 §3의 실행 아래에 다음 줄이 붙는다.

```text
- context per call: 462 → 4409 → 4591 · cache hit: 0 → 640 → 4352
```

H4의 실행 기록에는 새 필드가 없으므로 표의 `cached · peak ctx` 열은 `—`로 나온다. 이전 기록은 그대로 읽히고, 새 필드는 없으면 `unavailable`로 본다.

대화형 모드에서는 입력 하나가 끝날 때마다 마지막 호출의 context를 stderr에 한 줄로 보여 준다.

```text
[context: 4,591 tokens · cache hit 4,352 (94%)]
```

같은 값을 넣은 결정적 test로 기록과 표시를 확인했다.

```text
test output::tests::records_cache_hits_and_context_sizes ... ok
test output::tests::peak_context_can_come_before_the_last_call ... ok
test output::tests::cache_hits_are_unavailable_when_a_response_omits_them ... ok
test output::tests::a_failed_call_makes_cache_hits_unavailable ... ok
test output::tests::no_context_before_the_first_successful_call ... ok
test output::tests::formats_thousands ... ok
```

### 7. Codex와 DeepSeek Harness의 context budget

두 harness의 자세한 동작은 하위 페이지([Codex](codex.md), [DeepSeek Harness](deepseek-harness.md))에 정리했다. 여기서는 §5의 두 지점, budget을 어떻게 나누는지와 cache hit를 어떻게 지키는지만 비교한다.

| | Codex | DeepSeek Harness |
| --- | --- | --- |
| 현재 context | 마지막 응답의 token 수 + 그 뒤 추가분 추정(bytes ÷ 4) | session 기록으로 계산, 추정은 글자 수 ÷ 4 |
| 압축 시작 | window의 90% (설정값이 더 작으면 그 값) | window의 80% 또는 window − 출력 예약 − 65,536 중 작은 값 |
| tool 출력이 들어올 때 | 10,000 bytes를 넘으면 가운데를 자름 | 12,500 token을 넘으면 파일로 저장하고 앞·뒤만 보냄 |
| 압축할 때 남기는 것 | 최근 user 메시지(최대 20,000 token) + 요약 | 최근 구간(window의 약 16%) 원문 + 오래된 구간의 요약 |

**budget 설계.** 두 harness 모두 window 전체를 대화 기록에 쓰지 않는다. 출력 자리와 여유분을 먼저 떼어 두고, 그보다 앞에서 압축을 시작한다. 큰 tool 출력은 기록에 들어가는 순간 상한 안으로 줄인다. 차이는 압축할 때 드러난다. Codex는 model의 응답과 tool 결과를 모두 버리고 사용자 메시지와 요약만 남긴다. DeepSeek Harness는 오래된 구간만 요약하고 최근 작업은 tool 결과까지 원문으로 둔다.

**cache를 지키는 방법.** 두 harness 모두 기록을 뒤에 덧붙이기만 하고, 앞부분은 압축할 때까지 고치지 않는다. tool 출력을 들어올 때 미리 줄여 두면 나중에 그 결과를 고칠 일도 줄어든다. Codex는 설정이나 실행 환경이 바뀌어도 앞의 메시지를 고치지 않고 바뀐 부분만 새 메시지로 덧붙인다. DeepSeek Harness는 요약 요청 자체가 cache hit에 성공하도록 원래 대화와 같은 앞부분으로 보내고, 오래된 tool 결과를 줄이는 작업도 압축 기준을 넘었을 때만 한다.

압축하는 순간에는 두 harness 모두 cache를 잃는다. Codex는 기록 전체를 다시 만들어서 cache를 처음부터 쌓고, DeepSeek Harness는 바뀐 구간부터 다시 쌓는다. 기록을 많이 줄일수록 window는 넉넉해지지만 다시 계산할 부분이 커진다.

## 돌아보기

### 변경 사항

`hel`의 실행 기록만으로 실행마다 cache hit 비율과 window를 가장 많이 쓴 순간을 볼 수 있게 됐다. 전에는 raw log를 열어 호출마다 더해야 했다. `evals report`는 표에 cache hit 합계와 최대 context를 보여 주고, 실행마다 호출별 context와 cache hit를 한 줄로 적는다. 대화형 모드에서는 입력 하나가 끝날 때마다 지금 대화가 차지하는 context가 stderr에 나온다. model에게 보내는 요청은 바뀌지 않았다.

조사한 내용을 바탕으로 H6에서 만들 context 관리 구조를 정했다. budget은 DeepSeek Harness의 기준을 따르고, 요청은 다음 순서로 구성한다.

```text
|<──────────────────────────── context window ───────────────────────────────>|
| system · tool 정의 · HEL.md |      요약       |    최근 구간 원문    | output 자리 |
|    고치지 않음 (cache 유지)    | 오래된 구간을 대체 |  window의 약 16%   | max_tokens |
```

| 단계 | 언제 | 하는 일 | 누가 |
| --- | --- | --- | --- |
| 1. 큰 tool 결과 저장 | tool 결과가 들어올 때 | 12,500 token을 넘으면 전체를 파일로 저장하고, 앞·뒤와 파일 경로만 기록에 넣는다 | harness 코드 |
| 2. 기준 확인 | 매 요청 전 | context가 `min(window × 0.8, window − 출력 예약 − 65,536)`을 넘었는지 본다 | harness 코드 |
| 3. 이전 tool 결과 줄이기 | 기준을 넘었을 때 | 8,192자를 넘는 이전 tool 결과를 앞 4,096자와 뒤 1,024자만 남긴다. 기준 아래로 내려가면 여기서 끝낸다 | harness 코드 |
| 4. 오래된 구간 요약 | 그래도 기준을 넘을 때 | 오래된 구간을 model에게 따로 보내 요약을 받고, 그 구간을 요약 하나로 바꾼다 | model |

1번과 3번은 harness 코드가 정해진 길이에서 문자열을 자르는 일이다. 4번의 요약은 model이 대화를 읽고 새로 쓰는 글이다. compaction(컨텍스트 압축)은 1~4번 전체를 가리키고, 그중 model이 글을 새로 쓰는 단계가 4번의 요약이다. harness는 system prompt, tool 정의, 요약할 구간을 원래 대화와 똑같이 보내고 마지막에 요약 지시를 붙인다. model이 돌려준 답(`content`)이 요약이 되고, 대화를 이어 가는 model은 이 요청을 보지 못한다. 그래서 요약에는 API 호출 한 번의 비용과 시간이 들고, 무엇을 남길지는 model이 판단하며, 같은 대화라도 실행마다 다른 요약이 나올 수 있다.

`deepseek-flash`의 window(1M)로 계산하면 압축 기준은 약 800K token이어서 지금까지의 작업은 닿지 않는다. H6에서는 기준을 낮춰 압축을 일찍 일으키고, 그 뒤에도 작업을 이어 갈 수 있는지 확인한다.

### 트레이드오프

오래된 구간을 요약으로 바꾸면 그 뒤의 최근 구간은 내용이 같아도 앞이 바뀌었기 때문에 한 번은 cache miss로 다시 계산된다. 요약 요청 자체도 비용이다. 최근 구간을 원문으로 남기는 만큼 한 번에 줄어드는 양이 작아서, Codex처럼 거의 모두 버리는 방식보다 압축이 자주 일어날 수 있다.

### 논문의 내용 또는 다른 harness와 비교하면

논문의 [Recommendation 7](https://arxiv.org/html/2609.00006v1#S16.SS5)은 window보다 일정량 아래에서 압축을 시작하고, 최근 기록은 원문으로 남기고, 이전 요약에 이어 붙이라고 권한다. 정한 구조는 이 권고와 같다. 논문은 기준값을 비교하지 않았으므로, 이번에는 같은 DeepSeek API를 쓰는 harness가 cache를 기준으로 정한 값을 따랐다.

Codex는 압축할 때 대화 전체를 model에게 보내고, 사용자 메시지와 요약만 남긴 채 나머지를 버린다. 기록이 크게 줄고 구조가 단순하지만, 최근에 읽은 파일 내용도 함께 사라지고 cache를 처음부터 다시 쌓는다.

DeepSeek Harness는 오래된 구간만 요약하고, 요약 요청도 기존 cache hit에 성공하도록 보낸다. 요약 전에 tool 결과를 줄여 보고, 그것으로 충분하면 model을 부르지 않는다. 대신 구간을 고를 때 tool 호출과 결과를 떼지 않아야 하고, 이전 요약과 합치는 지시까지 다뤄야 한다.

Claude Code도 공식 문서상 한도 근처에서 자동으로 압축하고 `/compact`로 남길 내용을 지시할 수 있지만, 기준과 남기는 범위는 소스로 확인할 수 없었다.
