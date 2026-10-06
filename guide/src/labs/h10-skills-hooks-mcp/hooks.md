# B — Hooks

## 들어가며

hook은 harness의 실행 흐름에서 특정 시점에 호출되는 일종의 함수다. model이 매번 실행을 선택하지 않아도, 그 시점에 도달하면 harness가 연결된 hook을 호출한다.

예를 들어 파일을 편집하기 전에 대상 경로를 검사하거나, tool 실행 뒤에 결과를 기록하는 hook을 붙일 수 있다. 지침에 적힌 규칙을 실제 실행 시점에서 확인하려는 경우에도 hook을 사용할 수 있다.

### agent loop에 연결되는 위치

[공통 loop 그림](./#현재-agent-loop와-확장-지점)의 B 단계에는 tool 실행 전후 두 지점이 있다.

| 이벤트 | 호출 위치 | hook의 역할 |
| --- | --- | --- |
| `PreToolUse` | tool 실행 전 | 호출 인자 검사·조건에 맞지 않는 실행 차단 |
| `PostToolUse` | tool 실행 후 | 결과 검사·기록·model에게 전달할 정보 추가 |

`PostToolUse`가 실행될 때는 tool이 이미 파일을 바꾸거나 외부 요청을 보냈을 수 있다. 변경을 막아야 하는 규칙은 `PreToolUse`에서 실행 전에 판단해야 한다.

Claude Code의 [Hooks 문서](https://code.claude.com/docs/en/hooks#hook-lifecycle)는 이 두 이벤트 외에도 세션 시작과 사용자 입력 등 여러 시점을 다룬다. 성공 뒤의 `PostToolUse`와 실패 뒤의 `PostToolUseFailure`도 구분한다(2026-10-06 확인). `hel`에서 실패한 호출을 다루는 hook을 따로 만들지 않고, `PostToolUse`만 사용한다.

## 이번에 해볼 것

tool 실행 전에 `PreToolUse` hook을, 실행 후에는 `PostToolUse` hook을 호출한다. harness는 호출 정보와 실행 결과를 hook에 전달하고, hook이 반환한 값을 loop에서 사용한다.

기존 `hel`의 권한 게이트는 접근 레벨에 따라 tool 실행을 허용하거나 승인을 요청한다. hook을 호출해 프로젝트별 규칙을 실행 시점에 검사할 수 있다.

## 확인할 내용

- hook을 지정하지 않았을 때 기존 tool 실행이 유지되는지
- 실행 전 검사가 거절한 호출이 실제 tool에 도달하지 않는지
- 정상 실행 뒤 `PostToolUse` hook을 호출하고 결과가 대화에 돌아오는지
- hook 자체의 실패를 tool 실패와 구분할 수 있는지

[H2 FAQ](../h02-editing/faq.md)에서 남긴 읽기 전 편집 방지도 검토할 수 있다. 다만 편집 tool만 검사하면 `bash`로 파일을 바꾸는 경로가 남는다.

<!-- 결과 확인·돌아보기는 실제 구현과 측정 뒤 작성한다. -->
