# DeepSeek Harness의 context budget

> [!NOTE]
> 확인한 소스: [deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) commit `5badb15` (2026-10-04). 기능마다 plugin으로 나뉘어 있고, 아래 값은 기본 구성([base cordis.patch.yml](https://github.com/deepseek-ai/deepseek-harness/blob/5badb15009ae1756c3afe0ae0cef1faafc290ccc/packages/bundle/base/cordis.patch.yml))에서 켜지는 plugin 기준이다.

## 현재 context 측정

`dsh-token-meter`가 session 기록을 다시 읽어 계산하며, model을 호출하지 않는다. provider가 알려 준 token 수는 요청 내용이 완전히 같을 때만 다시 쓰고, 나머지는 글자 수를 4로 나눈 값에 구조 overhead를 더해 추정한다. 이 추정은 한중일 문자와 JSON Schema를 실제보다 적게 센다는 한계가 문서에 적혀 있다([token-meter](https://github.com/deepseek-ai/deepseek-harness/blob/5badb15009ae1756c3afe0ae0cef1faafc290ccc/packages/llm/token-meter/README.md)).

## window와 압축 시작 기준

DeepSeek adapter의 기본 window는 1,000,000 token, 출력 예약은 256,000 token이다([defaults.ts](https://github.com/deepseek-ai/deepseek-harness/blob/5badb15009ae1756c3afe0ae0cef1faafc290ccc/packages/llm/llm-deepseek/src/defaults.ts)).

```text
압축 시작 기준 = min(W × 0.8, W − O − 65,536)      W: window, O: 출력 예약
원문으로 남길 최근 구간 = (W − O) × 0.16
```

model별로 비율과 남길 양을 바꿀 수 있다([compaction-basic](https://github.com/deepseek-ai/deepseek-harness/blob/5badb15009ae1756c3afe0ae0cef1faafc290ccc/packages/compaction/compaction-basic/README.md), [config.ts](https://github.com/deepseek-ai/deepseek-harness/blob/5badb15009ae1756c3afe0ae0cef1faafc290ccc/packages/compaction/compaction-basic/src/config.ts)).

## 압축하는 시점

- model을 호출하기 전마다 기준을 넘었는지 확인한다.
- provider가 window 초과 오류를 돌려주면 기준과 상관없이 최대한 줄이고 한 번 다시 요청한다.
- 사용자가 `/compact`로 바로 압축할 수 있다.

## 압축할 때 남기는 것

가장 오래된 구간을 골라 요약 하나로 바꾸고, 최근 구간은 원문 그대로 둔다. 시스템 프롬프트는 고르지 않으며, tool 호출과 그 결과는 떼어 놓지 않는다. 요약은 고정된 Markdown 절(요청과 의도, 기술 개념, 파일과 코드, 오류와 해결, 남은 일, 현재 작업, 다음 단계, 중요한 맥락)로 쓰고, 이전 요약이 있으면 새 내용과 합친다. 요약이 원래 구간보다 줄지 않으면 쓰지 않는다.

```text
[system] [<compacted-summary> 오래된 구간의 요약] [최근 구간 원문 ...]
```

## tool 출력 상한

두 단계로 나뉜다.

| 단계 | 언제 | 하는 일 |
| --- | --- | --- |
| spill | tool 결과가 들어올 때 | 12,500 token을 넘으면 전체를 파일로 저장하고, 앞·뒤와 파일 경로, 그 파일을 `read`나 `grep`으로 보라는 안내만 보낸다([spill-policy](https://github.com/deepseek-ai/deepseek-harness/blob/5badb15009ae1756c3afe0ae0cef1faafc290ccc/packages/spill/spill-policy/README.md)) |
| pruning | 압축 기준을 넘었을 때만 | 8,192자를 넘는 이전 tool 결과를 앞 4,096자 + `[... tool result middle pruned ...]` + 뒤 1,024자로 바꾼다. 원본은 session 기록에 남는다. 줄인 결과가 기준 아래면 요약을 하지 않는다([tool-result-pruner](https://github.com/deepseek-ai/deepseek-harness/blob/5badb15009ae1756c3afe0ae0cef1faafc290ccc/packages/compaction/compaction-tool-result-pruner/README.md)) |

## cache

- 요약 요청을 보낼 때 시스템 프롬프트, 사용 가능한 tool 목록, 요약할 구간을 원래 대화와 byte 단위로 같게 보낸다. 그래서 요약 요청 자체가 기존 cache hit에 성공하고, 마지막 요약 지시와 요약 출력만 새로 계산된다.
- 오래된 구간만 바꾸므로 그 앞의 시스템 프롬프트 부분은 cache가 유지된다. 바뀐 지점부터는 cache를 다시 만든다.
- pruning은 압축 기준을 넘었을 때만 실행해서, 기준 아래에서는 이전 기록을 고치지 않는다.
- 각 plugin 문서에 "KV Cache effect" 절을 두어 그 기능이 cache를 어디서부터 깨는지 적는다.
