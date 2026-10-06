# FAQ

## Aider의 symbol 지도는 어떤 방식인가

논문의 [§9.7](https://arxiv.org/html/2609.00006v1#S9.SS7)은 Aider가 tree-sitter로 코드의 symbol을 추출하고, 대화와의 관련도로 순위를 매겨 token 예산 안에 repository map을 제공한다고 설명한다. 조사한 시스템 중 순위를 매긴 repository map을 제공하는 사례는 Aider 하나다.

[Recommendation 8](https://arxiv.org/html/2609.00006v1#S16.SS5)은 코드에 embedding 기반 RAG를 만드는 것을 피하라고 권하며, 대안에는 tree-sitter symbol 추출도 포함한다.

H4에서는 bash 검색과 전용 파일명·본문 검색 tool을 비교한다. symbol 추출과 repository map은 구현하거나 측정하지 않고 참고로 남긴다.

## 응답에 한도를 두면 전체 token도 고정되는가

한도는 전용 검색 tool 한 번이 돌려주는 내용에 적용된다. model이 검색을 몇 번 반복할지, bash로 다른 파일을 얼마나 읽을지, 최종 답을 얼마나 길게 쓸지는 이 한도로 정해지지 않는다. 사용 가능한 tool 목록도 매 요청에 포함되고, 앞서 받은 결과가 대화 기록에 남으면 다음 요청에서 다시 input이 된다.

H4에서는 개별 검색 응답을 제한했다. 누적된 대화 전체의 크기를 다루는 방법은 H5 Context Budget과 H6 Compaction에서 이어서 본다.
