# C — MCP

## 들어가며

지금의 `hel`은 소스 코드에 등록된 tool을 실행한다. 외부 프로그램이 제공하는 기능을 연결하려면 그 프로그램과 요청·결과를 주고받을 방법이 필요하다.

MCP(Model Context Protocol)는 이 통신에 사용하는 규약이다. harness 안의 MCP client가 서버와 연결하고, 서버는 tool과 자원을 제공한다. 서버는 같은 컴퓨터의 별도 프로세스일 수도 있고 원격 서비스일 수도 있다. [MCP 구조](https://modelcontextprotocol.io/docs/2026-07-28/learn/architecture)

### agent loop에 연결되는 위치

MCP를 통한 tool 사용에는 두 연결이 필요하다.

| 연결 | 흐름 |
| --- | --- |
| tool 발견 | 서버의 tool 이름·설명·인자 형식을 받아 model에게 보낼 사용 가능한 tool 목록에 반영 |
| tool 실행 | model의 호출 요청을 MCP client가 서버로 전달하고, 받은 결과를 대화에 추가 |

MCP에는 이를 위한 `tools/list`와 `tools/call`이 있다. model은 제공받은 목록에서 tool을 고르고, 실제 통신은 harness가 맡는다. [tool 발견과 호출](https://modelcontextprotocol.io/docs/2026-07-28/learn/architecture#understanding-the-tool-discovery-response)

[공통 loop 그림](./#현재-agent-loop와-확장-지점)의 C 단계는 tool 실행부와 외부 서버 사이의 호출·결과 반환을 보여준다. 서버에서 얻은 tool 목록을 model 입력 준비에 반영하는 과정도 함께 필요하다.

## 이번에 해볼 것

외부 tool을 기존 loop에서 사용하는 흐름을 다룬다.

MCP는 로컬 프로세스 사이의 표준 입출력을 사용하는 stdio와, 원격 서버에도 연결할 수 있는 Streamable HTTP를 제공한다. [전송 계층](https://modelcontextprotocol.io/docs/2026-07-28/learn/architecture#transport-layer)

외부 tool 호출에도 권한 판단이 필요하고, 반환된 결과는 model이 읽을 수 있는 형태로 대화에 포함해야 한다. 원격 서버가 일으키는 변경은 로컬 파일 접근 격리만으로 제한할 수 없으므로, 연결 대상의 실행 범위를 함께 다뤄야 한다.

## 확인할 내용

- 서버가 제공한 tool이 model에게 전달되는지
- 요청한 인자가 서버에 전달되고 결과가 대화로 돌아오는지
- 권한 판단에서 거절한 호출이 서버에 전달되지 않는지
- 서버 오류나 연결 중단 뒤 loop가 어떻게 동작하는지


<!-- 결과 확인·돌아보기는 실제 구현과 측정 뒤 작성한다. -->
