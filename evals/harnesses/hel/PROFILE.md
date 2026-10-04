# Harness Profile — hel

> 상태: 기준 구현 · 비교 등급: `subject`

`hel`은 각 Lab에서 직접 만드는 harness다(`crates/hel`). record를 직접 출력하므로 별도의 session 변환이 필요 없다. collector는 `hel`이 출력한 `record.json`을 그대로 읽는다.

| 항목 | 내용 |
| --- | --- |
| 버전 | `harness.version` (아래 실행 규약 참고) |
| model 연결 | DeepSeek 공식 API, OpenAI 호환 Chat Completions (`https://api.deepseek.com`), `Authorization: Bearer $DEEPSEEK_API_KEY` |

## 실행 규약

```text
hel --instruction <TEXT> [--context <run-context.json> --record <record.json>]
```

- 작업 디렉터리(cwd)는 runner가 만든 task fixture 복사본이다. 파일을 읽는 tool은 이 디렉터리 밖을 읽지 않는다.
- `--context`: runner가 쓰는 `record::RunContext`(run 정보, harness 버전, model, budget).
- `--record`: record-v0을 쓰고, 같은 디렉터리의 `raw/requests.jsonl`에 요청과 응답 본문을 남긴다(인증 header 제외).
- `harness.version`: `crates/hel`, `crates/record`를 마지막으로 바꾼 commit. 수정 중이면 `-dirty`, 설치된 `hel`이 소스보다 오래되었으면 `-stale`.
- `evals`는 PATH에 설치된 `hel`을 우선 사용하고, 없거나 `--build`이면 작업 폴더를 빌드해 쓴다.

## Tool → category 매핑

| hel tool | category |
| --- | --- |
| `read_file` | read |

Lab이 진행되며 tool이 추가되면 이 표를 갱신한다.
