# FAQ

## 추론 내용도 context를 차지한다

tool을 쓰는 요청에서는 이전 응답의 `reasoning_content`를 돌려보내야 하고, DeepSeek는 이것을 context에 넣는다. `hel`은 응답을 그대로 돌려보내므로 model이 추론에 쓴 token이 다음 요청부터 input으로 계속 실려 간다. 요약할 구간을 고를 때 이 부분을 어떻게 다룰지는 H6 Compaction에서 정한다.

## 요청을 보내기 전에는 크기를 모른다

`hel`은 응답을 받은 뒤에야 그 요청의 `prompt_tokens`를 알게 된다. 압축 기준을 넘었는지는 요청을 보내기 전에 판단해야 하므로, 마지막 응답의 token 수에 그 뒤 추가된 내용의 추정치를 더하는 방법이 필요하다. Codex는 bytes를 4로 나누고, DeepSeek Harness는 글자 수를 4로 나눈다. 어느 쪽을 쓸지는 H6 Compaction에서 정한다.
