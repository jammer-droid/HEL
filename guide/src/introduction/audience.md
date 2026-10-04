# 대상 독자와 전제

## 누구를 위한 가이드인가

이 가이드는 다음과 같은 사람을 위해 쓴다.

- **프로그래밍 경험이 있다.** 명령행, git, HTTP와 JSON 같은 기본 도구가 익숙하다.
- **LLM을 써 본 적이 있다.** ChatGPT, Claude 같은 서비스나 Claude Code, Codex 같은 coding agent를 사용해 봤다.
- **그 도구들이 안에서 어떻게 동작하는지 직접 만들어 보며 이해하고 싶다.**

## Harness란

> [!NOTE]
> **Harness**(하네스)는 model(LLM)을 agent(에이전트)로 동작하게 만들어 주는 프로그램을 말한다. model은 텍스트를 입력하면 텍스트를 생성하는 역할만 담당한다. 파일을 읽고, 명령을 실행하고, 결과를 다시 model에 보여주고, 언제 멈출지 정하는 것은 모두 harness가 한다.

이 프로젝트는 harness를 다음 7개 영역으로 나눠 본다. 이 분류는 production coding agent 11개의 소스 코드를 분석한 논문에서 가져왔다([참고 자료](../appendix/references.md)).

| 영역 | harness가 하는 일 |
| --- | --- |
| Agent loop | model 호출 → tool 실행 → 결과 전달을 반복. 중단 관리 |
| LLM integration | model API를 호출. 응답 형식과 오류 관리 |
| Tools & actions | 파일 읽기, 명령 실행, 편집 같은 tool을 정의하고 실행 |
| Memory & context | model에게 보여줄 기록과 맥락 |
| Safety & permissions | 위험 동작의 허용 여부와 권한 위임. 샌드박스 격리 |
| Orchestration | 작업 분배 및 Agent 병렬 실행 |
| Extensibility | core loop 변경 없는 기능 추가 |

## 구현 언어: Rust

이 가이드의 코드는 **Rust**로 작성한다. harness가 하는 일을 감추는 계층 없이 전부 눈으로 보기 위해 외부 프레임워크나 SDK 없이 HTTP로 직접 호출하는 것을 지향한다.

**Rust**가 아닌 익숙한 프로그래밍 언어로 작성해도 상관없다. 각 단계의 핵심은 언어보다 **무엇을 왜 바꾸고, 어떻게 측정하는가**에 있다. 다만 측정 도구와 기록 형식은 Rust 코드와 함께 제공되므로, 다른 언어를 쓴다면 결과를 같은 형식으로 남기는 부분을 직접 맞춰야 한다([측정 방법](measurement.md)).

## 다루지 않는 것

- **Model 학습과 fine-tuning.** model은 주어진 것으로 보고, 바꾸는 것은 harness뿐이다.
- **완성된 제품.** Claude Code 같은 도구를 그대로 복제하는 것이 목표가 아니다.
- **Production 운영.** 배포, 과금, 다중 사용자 같은 서비스 운영 문제는 다루지 않는다.
