# Harness Profile — hel

> 상태: 기준 구현 · 비교 등급: `subject`

`hel`은 각 Lab에서 직접 만드는 harness다(`crates/hel`). record를 직접 출력하므로 별도의 session 변환이 필요 없다. collector는 `hel`이 출력한 `record.json`을 그대로 읽는다.

| 항목 | 내용 |
| --- | --- |
| 버전 | `harness.version` (아래 실행 규약 참고) |
| model 연결 | DeepSeek 공식 API, OpenAI 호환 Chat Completions (`https://api.deepseek.com`), `Authorization: Bearer $DEEPSEEK_API_KEY` |

## 실행 규약

```text
hel (--instruction <TEXT> | --turns-file <turns.json>) [--compact-at N --keep-recent M | --no-compaction] [--access read-only|confirm|auto] [--tools <a,b>] [--env | --no-env] [--context-file <name> | --no-context-file] [--context <run-context.json> --record <record.json>]
```

- context 관리(H6, Adopt 이후 기본으로 켜짐): 요청 전 추정 context가 `min(W × 0.8, W − O − 65,536)`(W=1,000,000, O=출력 한도)을 넘으면 8,192자 넘는 이전 tool 결과를 줄이고, 그래도 넘으면 system 다음의 오래된 구간을 model 요약으로 바꾼다. 최근 `(W − O) × 0.16`은 원문 유지. 12,500 token 넘는 tool 결과는 H9부터 `<cwd>/.hel/sessions/<session-id>/spill/`(0700)에 UUID 이름의 0600 파일로 저장하고 앞·뒤·경로만 보낸다. 세션 삭제 때 함께 지우며 24시간 정리 대상에서 제외한다. 이전 H8의 임시 저장소에는 기존 비활성 24시간 정리를 유지한다. H6~H7의 기존 `hel-spill/`은 H8 정리 대상에 포함하지 않는다. `--compact-at N --keep-recent M`으로 기준을 바꾸고 `--no-compaction`으로 끈다. Lab 정의의 `settings.compaction: {at_tokens, keep_recent_tokens}` / `false`가 이 인자가 된다. 요약 요청은 raw log에 `purpose: compaction`으로 남는다(`h06`의 `hel`에는 없다).
- `--turns-file <turns.json>`: JSON 문자열 목록의 지시를 한 세션에서 차례로 보낸다(H6, eval-v5 세션 task). `max_turns`는 지시마다 적용한다. record는 하나이고 최종 출력은 마지막 지시의 답이다. 한 지시가 오류·한도로 끝나면 남은 지시는 보내지 않는다. raw log 항목에 `turn`(1부터)을 남긴다.

- `--tools`: model에게 줄 tool 목록(쉼표 구분). 없으면 `hel`의 기본 tool 구성을 쓴다(H0·`h01`: `read_file`, H1부터: `bash`). 측정 조건은 기본값에 기대지 않고 `settings.tools`로 명시한다. Lab 정의의 hel 조건에 `settings.tools`가 있으면 runner가 이 인자로 넘긴다. H1에서 추가(`h01`의 `hel`에는 없다. baseline 조건에는 `settings.tools`를 두지 않는다).
- `--env` / `--no-env`: 실행 환경 정보(OS, shell, 작업 디렉터리)를 system message로 보낼지. H3에서 추가했고 H3 Adopt 이후 기본으로 보낸다(`h03`의 `hel`에는 없고 system message도 없다). Lab 정의의 `settings.env: true`/`false`가 이 플래그가 된다.
- `--context-file <name>` / `--no-context-file`: 작업 디렉터리의 context 파일(기본 `HEL.md`)이 있으면 그 내용을 system message에 붙인다. 상위 디렉터리는 찾지 않는다. H3에서 추가했고 H3 Adopt 이후 기본으로 `HEL.md`를 읽는다. Lab 정의의 `settings.context_file: <name>`/`false`가 이 인자가 된다.
- H4부터 baseline 조건(설정 없음)은 환경 정보와 `HEL.md`를 보내는 `hel`이다. `h03` 코드와 같게 측정하려면 `settings: {env: false, context_file: false}`를 준다.

- 작업 디렉터리(cwd)는 runner가 만든 task fixture 복사본이다. eval-v7의 `fixture_workdir`가 있으면 복사본의 해당 하위 디렉터리를 cwd로 사용하고, 그 형제 경로에 시험용 보호 파일을 둘 수 있다. 파일을 읽는 tool은 이 디렉터리 밖을 읽지 않는다.
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

## H7 권한

`--access` 기본값은 `confirm`이다. `read-only`는 읽기·검색만 허용, `confirm`은 변경·범용 실행마다 1회 확인, `auto`는 등록된 tool 호출을 확인 없이 허용한다. 기존 파일 경계와 tool 인자 검사는 그대로 적용한다. bash는 명령 내용과 관계없이 범용 실행으로 분류한다.

TTY가 있으면 호출 이름·JSON 인자를 표시하고 y/yes에만 승인한다. 빈 입력·그 밖의 답은 거절, EOF/입력 오류와 비대화형 입력은 unavailable로 거절한다. `--approval-input`은 eval의 `--context`·`--record`·confirm 조건에서만 허용하는 JSON boolean 응답 파일이며, ask 한 번마다 하나씩 소비한다. 고갈되면 unavailable이다. 승인 캐시나 세션 전체 허용은 없다.

`raw/permissions.jsonl`은 각 호출의 access/action/decision/approval/executed를 남긴다. seq는 record.events와 대응한다. executed는 tool 구현 진입 여부이며, 파일 변경 성공 여부는 별도다. record-v0은 유지한다. H7 이전 baseline에는 이 로그가 없다.

이 버전에서 이전 Lab을 재실행할 때는 예전 승인 없는 동작이 필요하면 조건에 `access: auto`를 명시해야 한다. 이전 결과를 새 코드로 덮어쓰지 않는다. H7에서는 코드를 바꾸기 전에 baseline을 저장했다.

## H8 파일 접근 경계

- 외부 bash와 rg는 macOS sandbox-exec를 통해 실행한다. 다른 OS 또는 launcher 사용 불가 시 외부 프로그램 호출은 오류로 반환한다. 해제 옵션이나 자동 비격리 재실행은 없다. H7 access=auto도 이 정책을 유지한다.
- 프로젝트 읽기·쓰기, 현재 hel 인스턴스 tmp 읽기·쓰기, 현재 spill 읽기, 필요한 시스템 런타임과 선택한 실행 파일 읽기 허용. 시스템 경로는 sandbox.rs의 고정 정책을 사용한다. 네트워크는 H8 범위 밖으로 허용 상태다.
- 각 hel 인스턴스의 `<임시 폴더>/hel-runs/<UUID>/` 아래 tmp·spill 분리. tmp는 정상 종료 시 제거, 살아 있는 자식이 잠금을 보유하면 정리 유예. 비정상 종료 뒤 잠금이 해제된 tmp는 후속 시작에서 정리한다. spill은 파일 수정 시점부터 24시간 보관하며, 실행 중이면 나이에 관계없이 유지한다.
- 외부 명령에는 TMPDIR/TMP/TEMP를 현재 tmp로 지정하며 DEEPSEEK_API_KEY는 전달하지 않는다. 나머지 환경 변수는 기존 동작을 유지한다. 환경 변수 설정 자체는 접근 제한 수단이 아니다.
- 내부 파일 tool은 공통 Runtime의 경로 검사를 사용한다. canonical 경로를 검사하고 허용 루트의 디렉터리 descriptor를 기준으로 O_NOFOLLOW openat을 사용한다. 편집은 프로젝트 안에서만 허용하며, 프로젝트가 run 저장소의 상위 경로여도 저장소를 쓰기 범위에서 제외한다.
- read_file(path, start_line?, max_lines?)는 1부터 시작하는 줄 범위를 읽는다. 기본은 처음부터 EOF 방향이며 응답은 안내를 포함해 10,000 UTF-8 바이트 이내다. 잘린 범위는 read_file(cursor)로 이어 읽는다. cursor는 같은 hel 인스턴스에서만 유효하고 파일 변경 시 거절한다. 최근 128개 cursor를 보관하며 오래된 cursor는 새 범위 읽기로 대체해야 한다.
- H8 baseline은 구현 전의 소스로 저장했다. 구현 후 소스로 이전 조건의 baseline을 다시 실행하지 않는다. 측정 당시 소스와 측정 후 시스템 경로 보완 소스는 결과 폴더에 별도로 보존한다.


## H9 측정 driver (eval-v8)

`settings.session_mode`의 continuous/restart/resume 실행은 [SPEC §H9](../../SPEC.md#h9-프로세스-재시작-측정-eval-v8)를 따른다. resume는 hel의 `--resume <id>`를 사용한다. H9에서 구현 전 baseline·연속 각3회와 구현 후 resume3회를 실행했다. 이전소스로측정한결과는그대로보존한다.

H9 요청 params의 `tools: []`, `tool_choice: none`은 도구를 비활성화한다. CLI의 유일한 bash는 access=read-only로도 실행을 차단한다. DeepSeek의 `tool_choice: none` 계약은 [공식 API 문서](https://api-docs.deepseek.com/api/create-chat-completion/)에서 확인했다(2026-10-05). H9의 실제18요청에서도 tools=[]·none과도구호출0회를확인했다.

재시작 run만 collector가 두 stage의 record를 합친다. 원본은 stage 디렉터리에 그대로 보존하며, 합산 지표는 derived·전체 시간은 measured로 구분한다. 이외 조건에서는 기존 record를 그대로 읽는다.


## H9 세션 실행 규약

- 일반 실행·--instruction·--turns-file은 기본 새 세션을 시작한다. --resume <UUID>는 같은 작업 디렉터리의 세션을 재개한다. 시작 시 stderr에 session: ID를 표시한다. `hel sessions`와 `hel sessions delete <UUID>`는 model 호출·API key 없이 목록·삭제를 수행한다.
- cwd를 프로젝트 경계로 삼으며 상위 Git root를 찾지 않는다. `.hel/sessions/<id>/snapshot.json`에 schema_version=1, session_id, cwd, 시각, request_config, messages, meter, reader를 저장한다. `.hel/.gitignore`는 내부 상태를 제외하며, 검색의 마지막 제외 glob과 Runtime/native sandbox도 내부 저장소를 보호한다.
- 완료 답변 뒤 임시 파일 쓰기·sync·rename·디렉터리sync. 저장실패는경고후메모리계속,다음완료에전체snapshot재시도. API오류/출력한도/미완료답변에는저장하지않는다. 초기저장소·점유실패는시작거절이며, 완료된snapshot이없는세션은재개할수없다.
- 재개 시 system은 현재 env/HEL.md로 교체, 나머지메시지·요약·tool결과는보존. 현재model/tools/params/output/compaction/access를사용한다. 동일input조건이면Meter관측값복원,그외초기화;출력한도·압축정책만변경하면관측값유지. Reader는cursor와위치·파일identity를복원하며128개한도·변경파일거절·경로검사유지.
- .hel의store작업잠금은acquire/delete경합을직렬화하고각session의.active는인스턴스수명동안점유한다. 자식에인스턴스·세션FD를상속하므로본체종료후자식이보유해도재개/삭제거절. tmp는인스턴스소유유지,spill은세션소유로정상종료후보관한다.
- 현재session spill읽기외에.hel데이터는tool읽기/쓰기에서제외한다. snapshot형식·ID/cwd·tool연결·cursor형식이잘못되면재개거절. H8의기존spill을새세션으로이관하지않으며프로젝트이동·파일되돌리기·mid-tool복구는지원하지않는다.
