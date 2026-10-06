# FAQ

## 읽지 않은 파일을 고치지 못하게 하려면

논문 §7은 Claude Code와 Mistral Vibe가 prompt로 "읽지 않은 코드는 고치지 말라"고 지시하고, OpenCode는 tool 설명에 "읽지 않고 편집하면 오류"라고 적었지만 실제로 막는 코드는 없다고 지적한다. prompt에 쓴 문장은 model에게 바라는 것일 뿐 harness가 강제하는 장치가 아니다. H2는 편집 tool의 형태만 비교하고 이 정책은 다루지 않는다. 읽기 기록을 남겨 편집을 막는 방식은 H7 Permissions나 H10 Skills / Hooks / MCP에서 다시 본다.

## 엄격한 매칭에 실패하면

`search_replace`는 `search`가 글자 그대로 한 번 나와야 바꾼다. 이번 task에서는 실패하지 않았지만, 들여쓰기가 깊은 코드나 공백이 섞인 파일에서는 model이 옮겨 적은 문자열이 한 글자 달라 실패할 수 있다. 논문 §8.4의 다른 harness는 이런 경우 공백을 정규화하거나(Pi), 여러 단계로 느슨하게 맞춰 보거나(OpenCode, Hermes), LLM을 한 번 더 불러 치환 문자열을 고친다(Gemini CLI). model마다 이 실패가 얼마나 나오는지는 H13 Harness Evaluation에서 여러 model로 비교할 때 다시 본다.

## 확인 단계가 출력 token을 쓴다

model은 파일을 고친 뒤 거의 매번 결과를 다시 확인했다. 새 파일을 만들 때는 기대 내용 전체를 한 번 더 써서 `diff`로 비교해, 같은 내용을 두 번 출력했다. 편집 tool이 결과(예: 바뀐 줄 주변)를 함께 돌려주면 이 확인을 줄일 수 있다. tool 결과와 대화 길이가 비용에 미치는 영향은 H5 Context Budget에서 다룬다.

## 이전 실행의 작업 디렉터리가 보였다

한 실행이 `find /`로 디스크 전체를 검색했을 때, 결과에 이전 실행들이 남긴 작업 디렉터리의 같은 파일이 섞여 있었다. 측정 도구는 실행마다 새 작업 디렉터리를 만들지만, bash는 그 밖을 볼 수 있다. 실행끼리 서로 보지 못하게 가두는 문제는 H8 Sandbox에서 다룬다.

## macOS와 Linux의 명령 차이

macOS의 `sed -i`, `cat -A`는 Linux와 달라서, `md5sum`은 측정 환경의 PATH에 `/sbin`이 없어서 bash 명령이 여러 번 실패했다. model은 오류를 보고 다른 명령으로 바꿨지만 호출이 늘었다. Mini-SWE-Agent는 시스템 프롬프트에 OS 정보와 macOS용 안내를 넣는다. OS·작업 경로를 시스템 프롬프트에 어떻게 담을지는 H3 Repository Context에서 다룬다.
