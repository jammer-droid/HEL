# Harness Engineering Lab

> **Build, break, measure, and evolve a coding harness from scratch.**

이 가이드는 coding agent의 **harness**를 처음부터 직접 만들어 보는 과정이다. harness는 LLM 바깥에서 model을 반복 호출하고, tool을 실행하고, context를 관리하고, 위험한 동작을 막는 프로그램이다.

Lab에서는 아주 작은 harness에서 시작해 **실패를 만들고, 그 실패를 설명하는 가설을 세우고, 최소한의 구현을 진행한 뒤, 같은 조건에서 다시 측정**하는 작업을 반복하면서 harness의 내부 구조와 작동 방식을 이해하고, 더 나아가 개인의 워크플로우에 적합한 harness를 효과적으로 구현할 수 있는 기초 체력을 얻는 것을 목표로 한다.

## Lab의 진행 방식

- Rust로 작은 harness를 직접 구현한다. model API를 직접 호출하는 loop에서 시작해 tool, repository 탐색, context 관리, 권한, sandbox, session, 확장, subagent로 넓혀 간다.
- 매 단계마다 변경 전과 후를 **같은 task, 같은 model, 같은 budget**으로 비교한다.
- 같은 측정 절차로 Claude Code 같은 외부 harness도 함께 측정해, 우리가 만든 구조와 production harness의 결과를 함께 비교한다.
- 각 단계 끝에서 production harness는 같은 문제를 어떻게 다루는지, 그리고 그 복잡성이 **어떤 engineering pressure 때문에** 생겼는지 살펴본다.

## 읽는 순서

1. **Introduction**을 먼저 읽는다. 이 가이드의 방식, 준비물, 측정 방법, 비용과 정책을 다룬다.
2. **Lab**은 H0부터 순서대로 진행한다. 각 Lab은 이전 Lab의 결과물을 시작 상태로 삼는다.
3. **Appendix**는 필요할 때 찾아본다.

각 Lab은 글 한 편과 FAQ로 구성된다. 글은 같은 흐름을 따른다.

| 절 | 내용 |
| --- | --- |
| 들어가며 | 필요한 개념, 논문과 참고 자료가 다루는 내용 |
| 이번에 해볼 것 | 그중 직접 만들어 확인할 부분 |
| 결과 확인 | 핵심 코드와 실제로 돌려서 확인한 결과 |
| 돌아보기 | 달라진 것, 대가, 논문 또는 다른 harness와 비교 |

FAQ에는 본문에 넣지 않았지만 이후 Lab과 이어지는 이야기를 남긴다. 각 글 맨 위에 시작 tag와 다루는 논문 절을 적는다.

## 목차

> [!NOTE]
> 목차에서 흐리게 보이는 항목은 아직 작성되지 않은 페이지다.

| Part | Lab | 핵심 질문 |
| --- | --- | --- |
| I. Foundations | H0 Hello Harness | LLM을 agent로 만드는 최소 구조는 무엇인가? |
| | H1 Tool Loop | bash 하나만으로 어디까지 가능한가? |
| | H2 Editing | coding agent의 edit는 왜 실패하는가? |
| II. Repository Intelligence | H3 Repository Context | agent가 매번 repository를 다시 발견하지 않게 하려면? |
| | H4 Repository Search | 전용 파일명·본문 검색 tool은 bash 검색의 탐색 비용을 어떻게 바꾸는가? |
| III. Context | H5 Context Budget | Codex와 DeepSeek Harness는 context budget을 어떻게 측정·배분하고, 한도에 가까워지면 무엇을 남기고 버리는가? |
| | H6 Compaction | 기존 작업과 같은 환경에서 compaction을 일찍 일으킨 뒤 이어서 작업하면, 작업은 계속되고 호출마다 cache hit는 다시 올라가는가? |
| IV. Safety and Runtime Reliability | [H7 Permissions](labs/h07-permissions/README.md) · [FAQ](labs/h07-permissions/faq.md) | 접근 레벨에 따라 tool 호출을 어떻게 제어하는가? |
| | H8 Sandbox | permission과 isolation은 왜 다른 문제인가? |
| | H9 Sessions & Checkpoints | crash, resume, reproducibility를 어떻게 다룰 것인가? |
| V. Extensibility and Orchestration | H10 Skills / Hooks / MCP | core loop를 바꾸지 않고 capability를 어떻게 확장할 것인가? |
| | H11 Subagents | delegation과 context isolation은 언제 이득인가? |
| | H12 Parallelism | concurrency가 실제로 task completion에 도움이 되는가? |
| VI. Evaluation | H13 Harness Evaluation | 추가 scaffold가 실제 capability를 얼마나 개선하는가? |

Lab의 이름과 순서는 실험을 진행하며 바뀔 수 있다.
