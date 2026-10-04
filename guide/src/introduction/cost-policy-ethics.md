# 비용, 정책, 윤리

harness를 만들고 실험하는 과정에서 알아 둬야 할 내용을 미리 정리했다.

## API 비용

- model 호출은 token 단위로 과금된다. 실험은 같은 task를 여러 번 반복하므로, 반복 횟수 × 조건 수 × task 수만큼 호출이 쌓인다.
- [DeepSeek의 가격 안내](https://api-docs.deepseek.com/quick_start/pricing) 페이지를 통해 사용할 model의 호출 비용을 확인할 수 있다.
- 이 가이드는 비용을 token 수로 기록하고, 달러 환산은 그 시점의 가격으로 계산한다.

## API key와 기록 관리

- key는 환경 변수로만 주입하고 저장소에 commit하지 않는다.
- **원본 기록(raw)에 key가 남지 않게 한다.** harness가 HTTP 요청을 기록할 때 `Authorization` 같은 인증 header는 저장하지 않는다.

## 외부 API로 보내는 데이터

model API를 호출하면 instruction, 파일 내용, tool 결과가 모두 provider의 서버로 전송된다.

## Claude Code를 다른 model로 실행하는 것에 대해

이 가이드는 외부 비교를 위해 Claude Code를 DeepSeek model로 실행한다. 이에 대해 확인한 내용은 다음과 같다(2026-10 기준).

- DeepSeek는 Claude Code를 연결하는 방법을 [공식 문서](https://api-docs.deepseek.com/quick_start/agent_integrations/claude_code)로 안내한다.
- 연결해서 사용할 수는 있지만, Anthropic은 Claude Code를 Claude가 아닌 model과 함께 쓸 때 생기는 문제에 대해 동작이나 지원을 보장하지 않는다. 자세한 내용은 [Claude Code의 LLM gateway 문서](https://code.claude.com/docs/en/llm-gateway)에서 확인할 수 있다.
- 이 가이드는 Claude Code 바이너리를 **수정하거나 분석하지 않는다.** 실행 결과로 로컬에 남는 session 기록만 읽는다.

## Agent 실행 환경

agent는 파일을 읽고, 명령을 실행하고, 파일을 고칠 수 있다. 직접 만든 harness와 외부 harness 모두 해당한다.

- 실험은 task fixture를 복사한 임시 디렉터리에서 실행한다.
- 외부 harness는 사용자 설정, plugin, memory를 읽지 않도록 설정을 격리해서 실행한다.
- 설정을 격리해도 프로세스는 사용자 권한으로 파일 시스템과 네트워크에 접근할 수 있다. 이를 막으려면 container 같은 sandbox가 필요하다.
- sandbox 밖에서는 권한 확인을 건너뛰는 실행 모드(예: Claude Code의 `--dangerously-skip-permissions`)를 쓰지 않는다.

permission과 sandbox의 차이는 H7(Permissions)과 H8(Sandbox)에서 다룬다.
