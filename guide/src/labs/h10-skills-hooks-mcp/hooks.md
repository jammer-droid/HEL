# B — Hooks

> [!NOTE]
> 관련 논문: [§10.1 Claude Code의 권한 계층](https://arxiv.org/html/2609.00006v1#S10.SS1), [§10.2 Codex의 권한 계층](https://arxiv.org/html/2609.00006v1#S10.SS2), [§14.1 Hooks와 이벤트 기반 확장](https://arxiv.org/html/2609.00006v1#S14.SS1)

## 들어가며

hook은 harness의 실행 흐름에서 특정 시점에 호출되는 일종의 함수다. model이 매번 실행을 선택하지 않아도, 그 시점에 도달하면 harness가 연결된 hook을 호출한다.

예를 들어 파일을 편집하기 전에 대상 경로를 검사하거나, tool 실행 뒤에 결과를 기록하는 hook을 붙일 수 있다. 지침에 적힌 규칙을 실제 실행 시점에서 확인하려는 경우에도 hook을 사용할 수 있다.

hook의 호출 시점과 적용 대상은 별개다. 이번에 다뤄볼 `PreToolUse`, `PostToolUse`를 예시로 들면 다음과 같이 구분할 수 있다.

이 hook들은 tool의 호출 전후에 실행될 수 있다. 호출 전이라면 `PreToolUse`를, 호출 후라면 `PostToolUse`를 사용할 수 있다. 그리고 이 hook들은 harness에서 제공하는 모든 tool에 적용하거나, 일부에만 적용할 수 있다.

| hook | 대상 tool | 하는 일 |
|---|---|---|
| `PostToolUse` | `read_file` | 읽기에 성공한 파일 기록 |
| `PreToolUse` | `search_replace`, `write_file` | 편집 대상의 읽기 기록 확인 |

### agent loop에 연결되는 위치

[공통 loop 그림](./#현재-agent-loop와-확장-지점)의 B 단계에는 tool 실행 전후 두 지점이 있다.

| 이벤트 | 호출 위치 | hook의 역할 |
| --- | --- | --- |
| `PreToolUse` | tool 실행 전 | 호출 인자 검사·조건에 맞지 않는 실행 차단 |
| `PostToolUse` | tool 실행 후 | 결과 검사·기록·model에게 전달할 정보 추가 |

등록된 hook이 있으면 `PreToolUse → 기존 권한 검사·필요 시 사용자 승인 → tool 실행 → PostToolUse` 순서로 진행한다. 각 이벤트에서 대상 tool에 일치하는 hook이 없으면 그 단계를 건너뛴다. hook이 없을 때는 기존 권한 검사와 tool 실행 흐름을 유지한다.

| tool 호출의 상태 | Post 호출 |
| --- | --- |
| Pre에서 차단 | 생략 |
| 권한 검사에서 거절되거나 사용자 승인을 받지 못함 | 생략 |
| tool 실행 성공 | 호출 |
| tool 실행을 시작했지만 파일 없음·치환 실패 등의 오류 발생 | 생략하고 오류를 model에게 바로 전달 |

`PostToolUse`가 실행될 때는 tool이 이미 파일을 바꾸거나 외부 요청을 보냈을 수 있다. 변경을 막아야 하는 규칙은 `PreToolUse`에서 실행 전에 판단해야 한다.

Claude Code의 [Hooks 문서](https://code.claude.com/docs/en/hooks#hook-lifecycle)는 이 두 이벤트 외에도 세션 시작과 사용자 입력 등 여러 시점을 다룬다. 성공 뒤의 `PostToolUse`와 실패 뒤의 `PostToolUseFailure`도 구분한다(2026-10-06 확인). `hel`은 tool 실행이 성공했을 때만 `PostToolUse`를 호출하고, 실패한 호출에는 별도 hook을 호출하지 않는다. 현재 `bash` tool은 명령의 종료 코드가 0이 아니면 실패로 처리하므로 이 경우에도 Post를 생략한다.

## 이번에 해볼 것

[H2에서 남긴 질문](../h02-editing/faq.md#읽지-않은-파일을-고치지-못하게-하려면)을 이어서, 읽지 않은 파일의 편집을 hook으로 막아 본다. `PostToolUse`에서 파일 읽기 성공을 기록하고, `PreToolUse`에서 편집 대상의 읽기 기록을 확인하는 흐름이다. 기록이 없으면 편집을 실행하지 않고 이유를 model에게 돌려준다.

harness는 이벤트와 대상 tool에 맞춰 등록된 command를 실행한다. 호출 정보를 JSON으로 표준 입력에 전달하고, 명령의 출력과 종료 코드를 받아 실행 흐름에 반영한다. command에는 외부 스크립트를 실행하는 명령도 지정할 수 있다.

읽기 기록과 편집 전 검사는 테스트용 스크립트에 둔다. harness는 읽기 이력의 의미를 알 필요 없이 같은 호출 방식으로 다른 검사나 작업을 연결할 수 있다. 스크립트는 호출마다 새 프로세스로 실행되므로, 호출 사이에 필요한 기록은 스크립트가 별도로 관리해야 한다.

확인할 동작은 차단 뒤에도 이어진다. model이 거절 사유를 보고 파일을 읽은 다음 다시 편집할 수 있어야 한다.

기존 `hel`의 권한 게이트는 접근 레벨에 따라 tool 실행을 허용하거나 승인을 요청한다. hook을 호출해 프로젝트별 규칙을 실행 시점에 검사할 수 있다.

### 프로젝트 hook 등록

프로젝트의 `.hel/hooks.json`에 hook을 등록한다. `hooks` 아래에 이벤트를 두고, `matcher`로 대상 tool을 고른 뒤 실행할 handler를 나열한다. 다음은 테스트 스크립트를 연결하는 설정 예시다.

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "^(search_replace|write_file)$",
        "hooks": [
          {
            "type": "command",
            "command": "python3 hooks/read_guard.py",
            "timeout": 10
          }
        ]
      }
    ],
    "PostToolUse": [
      {
        "matcher": "^read_file$",
        "hooks": [
          {
            "type": "command",
            "command": "python3 hooks/read_guard.py",
            "timeout": 10
          }
        ]
      }
    ]
  }
}
```

`matcher`는 tool 이름에 적용하는 정규식이며 생략하면 전체 tool을 대상으로 한다. `type: "command"`는 명령 실행을 뜻하고 `timeout`은 초 단위 제한이다. 예시는 각 호출에 10초를 지정했다.

같은 이벤트와 matcher에 여러 handler를 등록할 수 있다. 동일한 command를 중복 등록해도 각각 실행 대상으로 남긴다. 같은 이벤트에서 여러 matcher 그룹이 일치하면 그룹 배열 순서와 각 `hooks` 배열 순서대로 하나씩 실행한다.

`PreToolUse`에서 명시적으로 tool 차단을 반환하면 남은 handler를 실행하지 않고 해당 tool도 차단한다. 이때 `PostToolUse`도 호출하지 않고 차단 사유를 model에게 바로 전달한다. hook 자체의 오류는 보고한 뒤 다음 handler로 진행한다. 명시적 차단 없이 검사를 마치면 기존 tool 실행 흐름을 계속한다.

이 형식은 [Codex의 설정 형식](https://learn.chatgpt.com/docs/hooks#config-shape)을 참고한다. `hel`은 프로젝트의 `.hel/hooks.json`을 사용하며 사용자 홈의 설정이나 플러그인 설정을 합치지 않는다.

### hook 명령의 신뢰와 실행 권한

hook command는 사용자가 신뢰한 외부 명령으로 실행한다. hel을 시작할 때 `.hel/hooks.json`의 이벤트·matcher·command를 보여주고 프로젝트 설정 전체에 대해 `[y/N]`으로 확인한다.

승인하면 `.hel/hooks-trust.json`에 프로젝트의 실제 경로와 설정 파일 내용의 SHA-256 해시를 저장한다. 다음 실행에서 경로와 해시가 같으면 다시 묻지 않는다. 설정 파일 내용이 바뀌면 재확인한다. 승인한 설정은 현재 실행 동안 메모리에 유지한다.

| 상황 | 처리 |
| --- | --- |
| 처음 발견하거나 변경된 설정 | 실행할 정의를 표시하고 신뢰 확인 |
| 기존 승인과 프로젝트 경로·설정 해시 일치 | 재확인 없이 hook 사용 |
| 사용자가 거절 | hook 생략을 알리고 hel 계속 실행 |
| 입력을 받을 수 없고 기존 승인도 없음 | hook 생략을 알리고 hel 계속 실행 |

신뢰 대상은 등록된 명령과 설정이다. 명령이 참조하는 스크립트나 의존 파일의 내용은 해시에 포함하지 않으므로, 같은 command에서 스크립트 내용만 바꾸면 다시 확인하지 않는다.

hook 명령에는 model이 호출하는 tool의 sandbox를 적용하지 않는다. 따라서 외부 스크립트나 도구를 연결할 수 있고, 파일·네트워크 접근 범위는 hel 프로세스가 가진 OS 권한을 따른다. `cwd`는 작업 디렉터리이며 접근 범위를 제한하는 경계가 아니다.

[Codex의 hook 신뢰 절차](https://learn.chatgpt.com/docs/hooks#review-and-trust-hooks)를 참고한 방식이다. hook을 신뢰해도 model이 요청한 tool의 권한 검사와 sandbox 제한은 그대로 적용한다.

### hook 실행 결과와 실행 흐름 관리

`hel`에서는 hook의 실행 결과와 이에 따른 실행 흐름을 다음과 같이 처리한다. (Codex의 hook 처리 방식을 참고한다.)

| hook 결과 | 처리 |
| --- | --- |
| Pre의 명시적 `deny` 또는 종료 코드 `2` | 해당 tool 실행 차단, 이유를 model에게 전달 |
| 정상 종료, 차단 없음 | 기존 실행 흐름 계속 |
| 명령 실행 실패·일반 오류·시간 초과·잘못된 응답 | hook 오류를 보고하고 기존 실행 흐름 계속 |
| Post의 차단 피드백 또는 종료 코드 `2` | 이미 끝난 실행은 유지하고 model에게 피드백 전달 |

`PreToolUse` hook은 명시적인 거절을 반환해 해당 tool의 실행을 막는다. JSON으로 `deny`를 반환하거나 종료 코드 `2`와 함께 표준 오류로 차단 이유를 출력한다. harness는 실행하지 않은 tool 호출의 결과로 이 이유를 model에게 전달한다. model은 tool 실행 차단이 됐더라도 다음 행동을 할 수 있다.

스크립트가 규칙 위반으로 판단한 경우와 스크립트 자체의 오류를 나눠 확인한다. model이 요청한 tool의 기존 권한 게이트와 sandbox 제한은 hook 결과와 별도로 유지한다.

### 스크립트와 주고받는 정보

harness는 hook 스크립트를 실행하면서 표준 입력(`stdin`)으로 JSON 객체 하나를 전달한다. 스크립트는 표준 출력(`stdout`)의 JSON과 종료 코드로 판단을 반환한다. 종료 코드 `2`를 사용하면 표준 오류(`stderr`)에 차단 사유나 피드백을 쓴다.

#### harness → 스크립트: 입력

| 필드 | 형식 | 내용 |
| --- | --- | --- |
| `hook_event_name` | 문자열 | `PreToolUse` 또는 `PostToolUse` |
| `session_id` | 문자열 | 현재 hel 세션의 식별자 |
| `cwd` | 문자열 | hel의 작업 디렉터리 절대 경로 |
| `tool_use_id` | 문자열 | 해당 tool 호출의 식별자. 같은 호출의 Pre/Post는 같은 값 |
| `tool_name` | 문자열 | 호출할 tool 이름 |
| `tool_input` | JSON 객체 | model이 전달한 tool 인자 |
| `tool_response` | JSON 값 | Post에 추가하는 tool 실행 결과 |

다음은 `search_replace` 실행 전 Pre hook이 받는 입력 예시다. 식별자와 경로는 설명을 위한 값이다.

```json
{
  "hook_event_name": "PreToolUse",
  "session_id": "session-123",
  "cwd": "/project",
  "tool_use_id": "call-7",
  "tool_name": "search_replace",
  "tool_input": {
    "path": "a.txt",
    "search": "old",
    "replace": "new"
  }
}
```

같은 호출이 편집을 마치면 Post 입력에 결과가 추가된다. 아래는 이 tool에도 Post hook을 등록했을 때 받는 예시다.

```json
{
  "hook_event_name": "PostToolUse",
  "session_id": "session-123",
  "cwd": "/project",
  "tool_use_id": "call-7",
  "tool_name": "search_replace",
  "tool_input": {
    "path": "a.txt",
    "search": "old",
    "replace": "new"
  },
  "tool_response": "replaced 1 occurrence in a.txt"
}
```

읽기 기록을 남기는 스크립트에는 `read_file`의 Post 입력이 전달된다. `a.txt`의 내용이 `old`와 줄바꿈이라면 다음과 같다.

```json
{
  "hook_event_name": "PostToolUse",
  "session_id": "session-123",
  "cwd": "/project",
  "tool_use_id": "call-6",
  "tool_name": "read_file",
  "tool_input": {
    "path": "a.txt"
  },
  "tool_response": "old\n"
}
```

`tool_response`는 tool별 결과를 담는다. 위 예시는 문자열을 반환하는 현재 hel의 읽기·편집 tool을 기준으로 한다. Pre에는 실행 결과가 없으므로 이 필드를 넣지 않는다.

#### 스크립트 → harness: 출력

계속 진행하려면 아무것도 출력하지 않고 종료 코드 `0`으로 끝낸다. 이는 hook이 차단하지 않는다는 뜻이며, 기존 권한 검사까지 통과했다는 뜻은 아니다.

Pre에서 tool 호출을 차단하려면 다음 JSON을 `stdout`에 출력하고 종료 코드 `0`으로 끝낸다.

```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "deny",
    "permissionDecisionReason": "먼저 a.txt를 읽어 주세요."
  }
}
```

| 필드 | 내용 |
| --- | --- |
| `hookSpecificOutput.hookEventName` | 판단을 반환하는 이벤트, 여기서는 `PreToolUse` |
| `hookSpecificOutput.permissionDecision` | `deny`이면 해당 tool 실행 차단 |
| `hookSpecificOutput.permissionDecisionReason` | model에게 전달할 차단 이유 |

이 응답을 받으면 남은 Pre hook과 tool 실행을 중단하고 Post hook도 호출하지 않는다. model에게는 먼저 파일을 읽으라는 이유가 돌아간다.

Post에서 원본 tool 결과 대신 피드백을 전달하려면 다음 JSON을 `stdout`에 출력하고 종료 코드 `0`으로 끝낸다.

```json
{
  "decision": "block",
  "reason": "편집 결과에서 규칙 위반을 발견했습니다. 결과를 다시 확인해 주세요."
}
```

`decision: "block"`은 원본 tool 결과 전달을 막고, `reason`을 model에게 전달할 결과로 사용한다. 이미 실행된 편집의 효과는 되돌리지 않는다.

Post hook이 여러 개이면 각각 동일한 원본 `tool_response`를 받는다. 앞선 hook의 피드백을 다음 hook의 입력으로 넘기지 않는다. Post는 등록 순서대로 끝까지 실행하고, `block` 또는 종료 코드 `2`로 반환한 피드백을 순서대로 모아 model에게 전달할 원본 결과를 대체한다.

중간 hook이 정상 종료하거나 자체 오류로 실패해도 앞서 수집한 피드백은 유지한다. 대체할 피드백이 하나도 없으면 원본 tool 결과를 그대로 전달한다.

JSON 대신 종료 코드 `2`를 사용할 수도 있다. Python 스크립트에서는 다음과 같이 사유를 `stderr`에 쓰고 종료한다.

```python
import sys

print("먼저 a.txt를 읽어 주세요.", file=sys.stderr)
sys.exit(2)
```

같은 종료 코드라도 Pre에서는 tool 실행을 차단하고, Post에서는 원본 결과를 피드백으로 대체한다.

| 스크립트 종료 방식 | harness 처리 |
| --- | --- |
| 종료 코드 `0`, 출력 없음 | 다음 hook 또는 기존 실행 흐름으로 진행 |
| 종료 코드 `0`, Pre의 `deny` JSON | 남은 Pre hook·tool 실행·Post 호출 생략 |
| 종료 코드 `0`, Post의 `block` JSON | model에게 전달할 원본 결과를 `reason`으로 대체 |
| 종료 코드 `2`, `stderr`에 사유 | Pre는 실행 차단, Post는 결과 대체 |
| 그 밖의 실패 코드·실행 오류·시간 초과·잘못된 응답 | hook 오류를 보고하고 다음 hook으로 진행 |

이 입출력 구조는 [Codex의 공통 입력](https://learn.chatgpt.com/docs/hooks#common-input-fields), [PreToolUse](https://learn.chatgpt.com/docs/hooks#pretooluse), [PostToolUse](https://learn.chatgpt.com/docs/hooks#posttooluse)를 참고한다.

- pre hook은 tool을 실행할지 말지, post hook은 실행 결과를 model에게 전달할지말지에 영향을 준다.

### 실행 시간과 피드백 길이

`timeout`은 hook 명령 하나에 적용하는 초 단위 실행 제한이다. 생략하면 600초를 사용한다. 위의 테스트 스크립트 설정은 10초를 지정한다. 시간이 초과되면 hook 오류를 보고하고 다음 handler로 진행한다.

model에게 전달할 최종 hook 피드백은 약 2,500 추정 token을 기준으로 제한한다. 여러 Post hook의 피드백은 등록 순서대로 합친 뒤 이 기준을 적용한다. 긴 내용은 현재 세션의 spill 파일에 전체를 저장하고, model에게 앞뒤 미리보기와 파일 경로를 전달한다. 파일 저장에 실패해도 전체 내용을 그대로 넣지 않고 미리보기로 줄인다.

`deny`나 `block` 판단은 JSON을 해석할 때 먼저 처리한다. 길이 제한은 그 뒤 model에게 보여 줄 피드백에 적용하므로, 내용을 줄여도 차단 판단은 유지된다.

## 확인할 내용

- hook을 지정하지 않았을 때 기존 tool 실행이 유지되는지
- 실행 전 검사가 거절한 호출이 실제 tool에 도달하지 않는지
- 정상 실행 뒤 `PostToolUse` hook을 호출하고 결과가 대화에 돌아오는지
- hook 자체의 실패를 tool 실패와 구분할 수 있는지

`bash`는 읽기, 쓰기 등의 다양한 작업을 할 수 있는  통로 역할을 한다. 그래서 `bash`라는 tool 호출 전후에 hook을 배치할 순 있지만, 이번 실험에서 확인하고자 하는 편집 전용 tool에 연결한 hook에서는 `bash`를 통해 수정한 내용은 감지할 수 없다. 따라서 이번 실험에서는 hook을 연결한 tool로 한정해서 살펴본다.

### model의 차단 메시지 처리 확인

`config.ini`에 `port=7000`을 넣고, model에게 다음과 같이 요청한다.

```text
config.ini의 port=7000을 port=8000으로 바꿔줘. 다른 내용은 바꾸지 마.
첫 tool 호출은 파일을 읽지 말고 search_replace로 해줘.
그 뒤에는 tool이 돌려준 결과를 확인하고 작업을 완료해줘.
```

사용할 tool은 `read_file`과 `search_replace`다. 같은 요청을 hook이 없는 경우와 읽기 검사 hook을 적용한 경우에 전달한다. hook은 읽기 기록이 없는 편집을 차단하면서 다음 메시지를 돌려준다.

```text
config.ini를 먼저 read_file로 읽은 뒤 편집을 다시 요청해 주세요.
```

hook을 적용한 경우 확인할 흐름은 `편집 요청 → 차단 사유 전달 → 파일 읽기 성공 → 편집 재요청 성공`이다. 최종 파일이 `port=8000`으로 바뀌었는지도 함께 확인한다. model이 답변에 “읽었다”고 쓰는 것만으로 판단하지 않고 실제 tool 호출과 반환 결과를 본다.

처음부터 파일을 읽었다면 정상 작업이지만 차단 뒤 회복을 확인한 경우에는 포함하지 않는다. 첫 편집 요청을 유도한 횟수와 실제 차단 뒤 회복한 횟수를 나눠 기록한다. hook 없이 바로 편집에 성공하는 경우도 정상 결과다. 이 비교에서는 작업을 끝냈는지와 읽기 규칙을 거쳐 끝냈는지를 구분한다.

## 구현과 동작 확인

`crates/hel/src/hooks.rs`에서는 `.hel/hooks.json`을 읽고 사용자가 이 hook 실행을 승인했는지 판단하고, 승인이 됐으면 `.hel/hooks-trust.json`에 프로젝트 경로와 해시를 저장한다.

이후 실행에서는 호출할 tool에 맞는 hook 명령을 실행하고, 그 응답의 결과를 받아 tool 실행 여부를 결정한다.

`tools.rs`의 공통 호출 경로는 Pre와 기존 권한 검사를 거쳐 tool을 실행한다. 성공한 tool에만 Post를 호출한다.

```rust
let mut execution = permissions::execute(tool, name, args, runtime, access, approval);
if let Ok(original) = &execution.result
    && let Some(feedback) = runtime.hooks.after(runtime, &call, original)
{
    // The tool itself succeeded; only the model-visible result is replaced.
    execution.result = Ok(feedback);
}
execution
```

Pre handler가 명시적으로 거절하면 `before`가 즉시 사유를 반환한다. 호출 경로에서는 남은 hook과 tool 실행을 생략하고 그 사유를 model에게 전달한다.

```rust
for handler in self.handlers.iter().filter(|h| h.matches(Event::Pre, call.name)) {
    if let Decision::Feedback(reason) = self.invoke(runtime, call, handler, None) {
        return Some(model_feedback(
            runtime,
            &format!("PreToolUse hook blocked {}: {reason}", call.name),
        ));
    }
}
None
```

읽기 기록은 `evals/tasks/hook-read-before-edit-01/fixture/hooks/read_guard.py`가 관리한다. 성공한 `read_file`의 Post에서 경로를 기록하고, `search_replace`의 Pre에서 기록이 없으면 종료 코드 `2`로 거절한다. 스크립트의 상태 파일은 실행마다 새로 복사한 fixture의 `.hel` 안에 둔다. 설정과 스크립트는 같아도 각 실행의 읽기 기록은 섞이지 않는다.

### 결정적 테스트

| 확인한 동작 | 결과 |
| --- | --- |
| 같은 설정·프로젝트의 신뢰 재사용, 설정 변경·다른 프로젝트의 재확인 | 통과 |
| 미신뢰·잘못된 설정 생략, hook 없을 때 기존 실행 유지 | 통과 |
| 일치하는 handler의 중복·작성 순서 유지 | 통과 |
| Pre 차단 뒤 나머지 hook·tool·Post 생략 | 통과 |
| 일반 hook 오류 뒤 다음 handler 진행 | 통과 |
| Pre 뒤 권한 검사, tool 성공 시에만 Post 호출 | 통과 |
| 모든 Post에 같은 원본 전달, 피드백 순서 합산·결과 대체 | 통과 |
| hook의 외부 경로 접근과 기존 tool 경계 구분 | 통과 |
| 명령 시간 초과·출력 폭주 처리, 긴 피드백 저장과 미리보기 | 통과 |
| 실제 CLI에서 차단 사유 전달 → 읽기 → 편집 재시도 | 통과 |

Hooks 관련 hel 테스트 16개와 측정 준비 테스트 1개를 추가했다. 프로세스의 표준 출력과 표준 오류는 각각 1 MiB까지 수집한다. 설정 파일도 1 MiB, handler 수는 최대 128개로 제한하며, 이 한도는 model에게 전달하는 2,500 추정 token과 별개다.

### model 행동 테스트 결과

저장소 루트에서 다음 명령으로 실행했다. `baseline`은 Hooks 구현 전의 코드로, `variant`는 구현 후의 코드로 각각 실행한 조건 이름이다. `--build`는 현재 작업 디렉터리의 hel을 빌드하므로, 비교할 코드 상태를 맞춘 뒤 실행한다.

```bash
# Hooks 구현 전: hook 없는 조건
cargo run --quiet -p evals -- run h10 --conditions baseline --build

# Hooks 구현 후: hook 적용 조건
cargo run --quiet -p evals -- run h10 --conditions variant --build

# 저장된 결과로 비교 보고서 생성
cargo run --quiet -p evals -- report h10
```

실행 횟수와 model 설정은 `evals/labs/h10.yaml`을 따른다. 결과는 `results/h10/`에 저장한다.

두 실행 방식에서 각각 세 번씩 요청했다. 여섯 번 모두 첫 호출이 편집 요청이어서 의도한 상황이 만들어졌다.

| 관찰 | hook 없음 | hook 적용 |
| --- | --- | --- |
| 첫 편집 요청 | 3/3 | 3/3 |
| 읽기 전에 편집이 실제 실행됨 | 3/3 | 0/3 |
| 차단 메시지를 받은 뒤 읽고 다시 편집 | 차단 없음 | 3/3 |
| 최종 파일이 `port=8000` | 3/3 | 3/3 |

hook이 없을 때는 `편집 성공 → 파일 읽기로 확인` 순서였다. hook을 적용하면 `편집 차단 → 파일 읽기 → 편집 성공 → 파일 읽기로 확인`으로 바뀌었다. 차단 사유가 다음 model 입력에 포함된 것과, 그 뒤 읽기·편집이 성공한 것을 호출 기록에서 확인했다.

| 실행당 평균 | hook 없음 | hook 적용 |
| --- | --- | --- |
| model 호출 | 3회 | 5회 |
| tool 호출 | 2회 | 4회 |
| 입력 token | 3,140.3 | 5,749.3 |
| 출력 token | 487.3 | 619.3 |

두 방식 모두 파일 수정에는 성공했다. hook을 적용한 경우 읽기 전에 편집되는 것을 막았고, model은 전달받은 사유에 따라 읽은 뒤 다시 편집했다. 이 경로를 거치면서 model과 tool 호출이 각각 두 번 늘었다.

## 돌아보기

명령 실행과 판단 반영은 harness에 두고, 읽기 기록과 편집 검사는 외부 스크립트로 작성했다. 다른 규칙을 연결할 때도 같은 이벤트·입출력 형식을 사용할 수 있다.

검사 스크립트 자체의 오류는 보고 후 계속 진행하도록 구현했다. hook이 실행을 막는 규칙으로 쓰이더라도, 검사기가 실패하면 그 규칙이 적용되지 않을 수 있다. 또한 설정의 신뢰는 참조 스크립트 내용까지 검사하지 않는다. 프로젝트의 hook을 관리할 때 이 두 동작을 함께 고려해야 한다.
