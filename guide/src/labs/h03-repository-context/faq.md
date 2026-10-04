# FAQ

## 환경 정보를 받은 model이 사실과 다른 내용을 보고하면

`--env`로 실행한 `env-checksum-01`의 한 실행은 최종 답에 "macOS/Darwin has no `md5sum`"이라고 적었다. 이 Mac에는 `/sbin/md5sum`이 있다. model은 OS 이름을 받은 뒤 명령이 있는지 확인하지 않고, macOS에 대해 알고 있던 내용을 보고에 섞었다. 작업에 쓴 `md5 -q`가 성공해서 결과에는 영향이 없었다. 확인하지 않은 일반 지식이 작업 선택까지 바꾸는 경우(실제로 있는 명령을 피하는 등)는 이번 task에서 나타나지 않았다. model의 보고를 실행 기록과 대조하는 방법은 H13 Harness Evaluation에서 다룬다.

## 상위 디렉터리의 context 파일도 읽으려면

`hel`은 작업 디렉터리의 `HEL.md`만 읽는다. Codex는 `.git`이 있는 project root까지 올라가 그 사이의 `AGENTS.md`를 모두 이어 붙이고, Claude Code는 작업 디렉터리 위의 `CLAUDE.md`를 모두 읽는다. 저장소의 하위 디렉터리에서 실행할 일이 많아지면 이런 탐색이 필요하다. 하위 디렉터리의 규칙을 그 디렉터리 파일을 읽을 때 붙이는 방식(Claude Code)은 tool 결과에 내용을 더하는 구조라, 저장소 탐색을 다루는 H4 Repository Search나 확장 지점을 다루는 H10 Skills / Hooks / MCP에서 다시 본다.

## 실패한 명령을 세는 방법

실행 기록은 bash 명령의 exit code가 0이 아니면 실패로 센다. 그래서 `md5 -q data.txt || md5sum data.txt`처럼 앞 명령이 실패해도 뒤 명령이 성공하면 실패로 남지 않고, 반대로 `diff`가 차이를 찾아 exit 1을 돌려주면 실패로 남는다. 이번 Lab에서는 명령 내용을 직접 읽어 구분했다. 실행 기록에서 이런 차이를 자동으로 구분하는 방법은 H13 Harness Evaluation에서 다룬다.

## 같은 명령에 fallback을 붙이는 습관

`env-checksum-01`에서 model은 `md5 -q data.txt 2>/dev/null || md5sum data.txt`처럼 두 OS에 모두 대비한 명령을 자주 썼다. 환경 정보가 없던 첫 측정에서도, 환경 정보를 준 뒤에도 나타났고, 같은 system message로 다시 실행하면 세 번 중 세 번에서 한 번으로 바뀌기도 했다. 환경 정보와 상관없이 나타나는 model의 습관으로 보인다. 같은 명령 안에 붙어서 tool 호출을 늘리지 않으므로 이번 Lab에서는 비용으로 보지 않았다.
