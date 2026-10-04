# Harness Profile — Claude Code

> 상태: **active** ([SPEC.md](../../SPEC.md) §7.9)  
> 비교 등급: `same-model` (DeepSeek의 Anthropic 호환 endpoint로 실험 model `deepseek-flash` 사용)

## 1. 기본 정보

| 항목 | 내용 |
| --- | --- |
| 기준 버전 | 2.1.288 |
| 바이너리 | `~/.local/bin/claude` (Lab 정의의 `settings.binary`로 바꿀 수 있음) |
| headless 실행 | `claude -p "<instruction>" --output-format stream-json --verbose` |
| model 연결 | Anthropic Messages 형식. `ANTHROPIC_BASE_URL` + 인증 변수 + model 지정 변수 |
| 기본 tool | Read, Write, Edit, Bash, Grep, Glob, WebFetch 등. `--tools`로 제한 가능 |
| system prompt | 기본값 사용. 수정하지 않는다 |

## 2. Model 연결 (DeepSeek)

DeepSeek 공식 안내([Integrate with Claude Code](https://api-docs.deepseek.com/quick_start/agent_integrations/claude_code))를 따른다. driver는 다음 값을 실행할 때만 환경 변수로 넣는다.

```text
ANTHROPIC_BASE_URL=https://api.deepseek.com/anthropic
ANTHROPIC_API_KEY=$DEEPSEEK_API_KEY
ANTHROPIC_MODEL=deepseek-flash
ANTHROPIC_DEFAULT_OPUS_MODEL=deepseek-flash
ANTHROPIC_DEFAULT_SONNET_MODEL=deepseek-flash
ANTHROPIC_DEFAULT_HAIKU_MODEL=deepseek-flash
CLAUDE_CODE_SUBAGENT_MODEL=deepseek-flash
```

- DeepSeek는 Claude model 이름을 자동으로 매핑한다(opus → `deepseek-v4-pro`, sonnet/haiku → `deepseek-flash`). 다른 model이 섞이지 않도록 모든 model 지정 변수를 고정한다.
- `--bare` 모드는 인증 정보로 `ANTHROPIC_API_KEY`(`x-api-key` header)만 읽는다. DeepSeek의 Anthropic 호환 endpoint는 이 header를 받는다.
- 지원되지 않는 기능: MCP tool, document 입력, code execution. Web Search는 추가 요청 비용이 발생한다.

## 3. 격리 (SPEC §7.3)

| 수단 | 목적 |
| --- | --- |
| 환경 변수를 비우고(`env_clear`) 필요한 값만 전달 | 실행하는 셸의 설정과 세션 변수가 섞이지 않게 함. Claude Code 안에서 `evals`를 실행해도 중첩 실행이 되지 않음 |
| run별 임시 `HOME`, `CLAUDE_CONFIG_DIR` | 설정, 인증, session 기록, memory를 run별로 분리. 사용자 `~/.claude`를 읽지 않음 |
| `CLAUDE_CODE_PROJECT_DIR_NAME=<run-id>` | session 기록 위치를 `<CLAUDE_CONFIG_DIR>/projects/<run-id>/`로 고정 |
| `--session-id <uuid>` | session 파일 이름 고정 |
| `--bare` | hook, plugin, CLAUDE.md 탐색, auto-memory, keychain 읽기를 건너뜀 |
| `--strict-mcp-config` | 지정하지 않은 MCP server 무시 |
| `--tools` / `--allowedTools` | Lab 정의의 tool 구성으로 제한 (H0: `Read`) |
| `--permission-prompts none` | 권한 확인이 필요한 동작은 묻지 않고 거부 |
| `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1` | telemetry, 오류 보고, 자동 업데이트 차단 |
| fixture 임시 복사본을 cwd로 실행 | 작업 디렉터리 격리 |

권한 확인을 건너뛰는 모드(`--dangerously-skip-permissions`)는 쓰지 않는다.

## 4. Session 기록

- 주 소스: `-p --output-format stream-json`의 stdout → `raw/stream.jsonl`. collector는 이것을 읽는다.
- session 기록: `<CLAUDE_CONFIG_DIR>/projects/<run-id>/<session-id>.jsonl` → `raw/session.jsonl`로 복사한다.
- 필드별 추출 위치는 [availability.md](../../schema/availability.md)에 있다.

## 5. Tool → category 매핑

| Claude Code tool | category |
| --- | --- |
| Read | read |
| Grep, Glob | search |
| Edit, Write, NotebookEdit | edit |
| Bash | exec |
| 그 외 | other |

## 6. Budget 대응

| budget | Claude Code 설정 | 상태 |
| --- | --- | --- |
| `max_turns` | 대응하는 옵션 없음 | unsupported. runner timeout으로만 제한 |
| `timeout_seconds` | runner가 프로세스 단위로 강제 | supported |
| `max_output_tokens` | `CLAUDE_CODE_MAX_OUTPUT_TOKENS` 환경 변수 | supported |

## 7. 알려진 제한

- Claude Code가 DeepSeek에 보내는 reasoning(thinking) 설정은 통제하거나 확인할 수 없다. 요청 본문이 기록에 남지 않는다.
- `max_turns`를 맞출 수 없다.
- system prompt와 tool 설명은 Claude model에 맞춰져 있다. 측정 결과는 "Claude Code + DeepSeek model" 조합의 결과이며, Claude model로 실행한 Claude Code의 성능이 아니다.

## 8. 사용 조건

- Anthropic은 Claude Code를 Claude가 아닌 model과 함께 쓸 때 생기는 문제에 대해 동작이나 지원을 보장하지 않는다([LLM gateway](https://code.claude.com/docs/en/llm-gateway)).
- 바이너리를 수정하거나 분석하지 않는다. 실행 결과로 남는 출력과 session 기록만 읽는다.
- Anthropic 계정 인증 정보를 쓰지 않는다. DeepSeek key로 DeepSeek에 요청한다.
- 결과를 공개할 때는 "수정하지 않은 Claude Code (버전) + DeepSeek `deepseek-flash` backend"처럼 실행 조건을 적고, Anthropic의 보증이나 협력을 암시하는 표현을 쓰지 않는다.
