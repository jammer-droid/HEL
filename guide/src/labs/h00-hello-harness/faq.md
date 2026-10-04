# FAQ

## Claude Code의 줄 번호와 cache

Claude Code의 읽기 tool은 줄마다 번호와 탭을 붙여 돌려준다. 그대로 출력하는 task에는 방해가 되지만, model이 "12번째 줄"처럼 위치를 말하거나 편집할 범위를 정할 때는 사용자 입장에서 도움이 될 수 있다.

두 번째 호출에서는 Claude Code가 앞부분 512 token, `hel`이 256 token을 provider cache에서 읽었다. `hel`은 cache를 따로 요청하지 않았다. DeepSeek API가 앞부분이 같은 요청을 자동으로 cache한 것이다.

## reasoning도 출력 token에 포함된다

baseline을 시작하는 시점에서 출력 token의 한도를 2048로 제한했다. 그리고 이 상황에서 model에게 요청을 보냈다.

model은 자신이 가진 tool이 없는 상태였기에 '파일에 접근할 수 없는데, 이를 어떻게 하지. 내용을 지어내야 하나?(추측)'와 같은 reasoning을 약 9천여 자 동안 고민하다가 token 제한 한도를 모두 reasoning에 쓰고 빈 답으로 끝났다.(`finish_reason: length`) 이를 통해 thinking이 켜진 model은 reasoning도 출력 token에 포함됨을 확인할 수 있었다.

출력 제한에 답변이 잘리지 않도록 한도를 8192로 늘리고, 같은 요청을 6번 더 보냈다. 그러나 이번에 출력 token은 368~721 수준에서 끝났다. API로 호출하는 model이 내부에서 이 데이터를 확인한건지 아닌지 모르겠으나 같은 지시에서도 매번 다른 출력을 하는 것을 확인할 수 있다.

reasoning 설정은 harness가 결정한다. 이 설정이 task 결과와 비용에 어떤 영향을 주는지는 아직 확인하지 않았다. Harness Evaluation에서 확인해볼만한 내용이다.

## `hel`은 지시 하나만 받는다

지금의 `hel`은 `--instruction`으로 지시 하나를 받아 끝까지 실행하고 끝난다. 사람과 주고받으며 쓰는 대화형 CLI는 아직 없다. 측정은 이 방식으로 충분하지만, 직접 써 보면서 harness를 키우려면 대화형 틀이 필요하다. H1 전에 [대화형 기본 틀](../interactive-cli/README.md)에서 만든다.

## 전용 tool이 아니라 범용 `bash`를 쓴다면?

Mini-SWE-Agent는 bash 하나로 모든 일을 한다. 파일 읽기도 `cat`이면 된다. 전용 tool은 할 수 있는 일이 좁아서 경로 검사 같은 제한을 걸기 쉽고, bash는 무엇이든 할 수 있는 대신 제한하기 어렵다.
