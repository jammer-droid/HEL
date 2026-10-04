# Harness Profile — hel

> 상태: 기준 구현 · 비교 등급: `subject`

`hel`은 각 Lab에서 직접 만드는 harness다(`crates/hel`). record를 직접 출력하므로 별도의 session 변환이 필요 없다. collector는 `hel`이 출력한 `record.json`을 그대로 읽는다.

| 항목 | 내용 |
| --- | --- |
| 버전 | `harness.version` (아래 실행 규약 참고) |
| model 연결 | DeepSeek 공식 API, OpenAI 호환 Chat Completions (`https://api.deepseek.com`), `Authorization: Bearer $DEEPSEEK_API_KEY` |

## 실행 규약

```text
hel (--instruction <TEXT> | --turns-file <turns.json>) [--compact-at N --keep-recent M | --no-compaction] [--tools <a,b>] [--env | --no-env] [--context-file <name> | --no-context-file] [--context <run-context.json> --record <record.json>]
```

- context 관리(H6, Adopt 이후 기본으로 켜짐): 요청 전 추정 context가 `min(W × 0.8, W − O − 65,536)`(W=1,000,000, O=출력 한도)을 넘으면 8,192자 넘는 이전 tool 결과를 줄이고, 그래도 넘으면 system 다음의 오래된 구간을 model 요약으로 바꾼다. 최근 `(W − O) × 0.16`은 원문 유지. 12,500 token 넘는 tool 결과는 임시 파일에 저장하고 앞·뒤·경로만 보낸다. `--compact-at N --keep-recent M`으로 기준을 바꾸고 `--no-compaction`으로 끈다. Lab 정의의 `settings.compaction: {at_tokens, keep_recent_tokens}` / `false`가 이 인자가 된다. 요약 요청은 raw log에 `purpose: compaction`으로 남는다(`h06`의 `hel`에는 없다).
- `--turns-file <turns.json>`: JSON 문자열 목록의 지시를 한 세션에서 차례로 보낸다(H6, eval-v5 세션 task). `max_turns`는 지시마다 적용한다. record는 하나이고 최종 출력은 마지막 지시의 답이다. 한 지시가 오류·한도로 끝나면 남은 지시는 보내지 않는다. raw log 항목에 `turn`(1부터)을 남긴다.

- `--tools`: model에게 줄 tool 목록(쉼표 구분). 없으면 `hel`의 기본 tool 구성을 쓴다(H0·`h01`: `read_file`, H1부터: `bash`). 측정 조건은 기본값에 기대지 않고 `settings.tools`로 명시한다. Lab 정의의 hel 조건에 `settings.tools`가 있으면 runner가 이 인자로 넘긴다. H1에서 추가(`h01`의 `hel`에는 없다. baseline 조건에는 `settings.tools`를 두지 않는다).
- `--env` / `--no-env`: 실행 환경 정보(OS, shell, 작업 디렉터리)를 system message로 보낼지. H3에서 추가했고 H3 Adopt 이후 기본으로 보낸다(`h03`의 `hel`에는 없고 system message도 없다). Lab 정의의 `settings.env: true`/`false`가 이 플래그가 된다.
- `--context-file <name>` / `--no-context-file`: 작업 디렉터리의 context 파일(기본 `HEL.md`)이 있으면 그 내용을 system message에 붙인다. 상위 디렉터리는 찾지 않는다. H3에서 추가했고 H3 Adopt 이후 기본으로 `HEL.md`를 읽는다. Lab 정의의 `settings.context_file: <name>`/`false`가 이 인자가 된다.
- H4부터 baseline 조건(설정 없음)은 환경 정보와 `HEL.md`를 보내는 `hel`이다. `h03` 코드와 같게 측정하려면 `settings: {env: false, context_file: false}`를 준다.

- 작업 디렉터리(cwd)는 runner가 만든 task fixture 복사본이다. 파일을 읽는 tool은 이 디렉터리 밖을 읽지 않는다.
- `--context`: runner가 쓰는 `record::RunContext`(run 정보, harness 버전, model, budget).
- `--record`: record-v0을 쓰고, 같은 디렉터리의 `raw/requests.jsonl`에 요청과 응답 본문을 남긴다(인증 header 제외).
- `harness.version`: `crates/hel`, `crates/record`를 마지막으로 바꾼 commit. 수정 중이면 `-dirty`, 설치된 `hel`이 소스보다 오래되었으면 `-stale`.
- `evals`는 `hel`을 환경 변수를 비우고 `PATH=/usr/bin:/bin:/usr/sbin:/sbin`과 `DEEPSEEK_API_KEY`만 주어 실행한다(Claude Code driver와 같은 PATH. eval-v3 이전에는 `/usr/bin:/bin`이라 macOS `/sbin`의 `md5`, `md5sum`을 찾지 못했다).
- `settings.ripgrep: true`(eval-v4)는 evals 프로세스의 PATH에서 rg를 찾아 run의 `bin/rg`로 복사한다. 복사본 `--version` 실행에 성공한 뒤 그 bin을 고정 PATH 앞에 추가한다. 설정이 없거나 false이면 위 기본 PATH를 유지한다. 원본이 없거나 복사본을 실행할 수 없으면 model API 호출 전에 중단한다. `raw/search-engine.json`에 원본의 canonical 경로, `bin/rg`, 버전 출력을 기록한다. 복사본은 fixture 밖에 있어 검색 대상으로 섞이지 않는다. 서로 비교할 조건에는 같은 설정을 주고, 비교 시 복사본 버전·내용이 같은지 확인한다.
- `evals`는 PATH에 설치된 `hel`을 우선 사용하고, 없거나 `--build`이면 작업 폴더를 빌드해 쓴다.

## Tool → category 매핑

| hel tool | category |
| --- | --- |
| `read_file` | read |
| `bash` | exec (H1. 명령 내용과 관계없이 exec. `cat`으로 읽어도 read로 세지 않는다) |
| `write_file` | edit (H2) |
| `search_replace` | edit (H2) |
| `glob`, `grep` | search (H4, rg 기반) |

Lab이 진행되며 tool이 추가되면 이 표를 갱신한다.

## H4 검색 tool

`--tools bash,glob,grep`로 제공한다. 실행 PATH에 ripgrep이 필요하며 evals는 `settings.ripgrep: true`로 준비할 수 있다. 기본 tool은 계속 bash다.

- `glob(pattern, path=".")`: `rg --files --glob`로 파일명 검색, 상대 경로 정렬 결과
- `grep(pattern, path=".", include?)`: `rg --json --regexp`로 한 줄 단위 정규식 검색. 상대 경로·줄 번호·일치 내용 반환
- shell 문자열을 만들지 않고 인자 배열로 실행. 검색 경로를 canonicalize해 cwd 밖 접근을 거절하고 디렉터리 순회에서 symlink를 따라가지 않음
- rg 기본 ignore 우선순위 사용. 명시적 glob/include는 `.gitignore`보다 우선할 수 있음(rg --glob 동작). UTF-8 파일 경로·내용 지원
- 최대 100 matching lines/paths와 10,000 UTF-8 bytes(잘림 안내 포함). 긴 한 줄은 UTF-8 경계에서 부분 반환 가능. 한도는 model에게 돌려주는 텍스트에 적용되며 rg의 스캔·프로세스 출력 버퍼를 제한하지 않음
- 일치 없음(exit 1)은 정상 결과, 잘못된 regex/glob·경로·실행 실패는 error. 오류 메시지도 10,000 bytes 안에 제한
