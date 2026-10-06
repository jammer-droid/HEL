# FAQ

## skill이 많아지면

목록을 먼저 전달해도 skill이 많아지면 이름·설명·경로가 차지하는 문맥이 늘어난다. 비슷한 설명 사이에서 잘못 고르거나, 필요한 skill을 놓칠 수도 있다. 목록을 좁히는 방식과 설명의 구체성을 함께 살펴볼 수 있다.

문맥을 얼마나 차지하는지는 H5 Context Budget·H6 Compaction과 이어진다. 여러 구성의 작업 성능과 비용 비교는 H13 Harness Evaluation에서 함께 살펴볼 수 있다.

## Skills와 MCP의 발견·호출 비용

Skills는 model에게 목록을 전달한 뒤 선택한 본문을 읽는다. MCP는 harness의 client가 `tools/list`로 외부 tool 목록을 받고, model이 선택한 tool을 `tools/call`로 호출한다. 목록을 한 번 가져왔는지 매번 갱신했는지에 따라 서버 통신량이 달라진다.

MCP의 서버 통신 횟수와 model API 요청 횟수는 따로 센다. harness가 model 요청 전에 목록을 가져올 수 있으므로, `tools/list` 자체가 model과 한 번 더 왕복한다는 뜻은 아니다. H13 Harness Evaluation에서는 입력 token과 model 요청·서버 통신 비용을 나눠 비교할 수 있다.

## 자식 agent에도 같은 확장을 전달하려면

Skills·Hooks·MCP를 부모가 사용한다고 해서 자식에게 같은 문맥과 접근 권한을 모두 넘겨야 하는 것은 아니다. 어떤 지침과 tool을 전달하고, 같은 hook을 호출할지는 H11 Subagents에서 자식의 실행 범위와 함께 다룬다.
