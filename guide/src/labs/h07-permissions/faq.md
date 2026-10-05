# FAQ

## 실행 격리가 필요한 이유

예시로 bash tool을 통해 `cargo test` 명령을 실행하도록 허용하면 테스트와 빌드 스크립트를 model이 실행할 수 있게 된다. 그러면 이 스크립트 내부에서 접근하는 파일이나 외부 서버를 향한 명령을 tool 접근 권한만으론 제어하기 어렵다.

H8에서는 이러한 문제를 해결하기 위해 harness에서 실행하는 프로세스를 Sandbox로 격리하고, Sandbox에서 허용한 범위의 작업만으로 프로세스 실행을 제어하는 방법에 대해 다룬다.

Sandbox로 실행 환경을 격리하더라도 같은 대상을 마운트하는 호스트 작업 환경에 영향을 줄 수 있기 때문에 실행 환경의 격리와 작업 권한 판단은 함께 사용해야 한다.

## Codex의 Full access

H7의 자동 실행은 tool 호출마다 사용자에게 확인하지 않는 레벨이다. OS sandbox 제한을 없애는 설정과 구분해야 한다. Codex의 [Full access](https://learn.chatgpt.com/docs/sandboxing#how-permissions-work)는 sandbox 제한 해제와 승인 요청 생략을 함께 선택하는 설정이므로 H7의 레벨과 그대로 대응하지 않는다.
