# Eval Specification

> eval-v0

이 문서는 harness를 평가하는 절차를 harness와 무관한 형태로 정의한다. 직접 만드는 harness(`hel`)와 외부 harness(Claude Code, OpenCode, Codex 등)는 같은 task를 같은 절차로 실행하고, 같은 형식의 결과를 만든다. **새 harness를 추가할 때 바뀌는 부분은 driver와 collector뿐이다.**

---

## 1. 원칙

1. **Task는 harness 밖에 있다.** task는 자연어 instruction, fixture, 판정 규칙으로 구성되며 특정 harness를 가정하지 않는다.
2. **수집과 판정을 분리한다.** collector는 일어난 일을 기록만 하고(`record.json`), 판정은 task의 check가 한다(`verdict.json`). 판정 기준이 바뀌어도 record는 다시 만들지 않는다.
3. **모르는 값은 모른다고 기록한다.** 수집할 수 없는 값을 0이나 빈 값으로 채우지 않는다(`status: unavailable`).
4. **정량 평가만 자동화한다.** 정성 평가가 필요하면 record에 남긴 원본 session 기록으로 나중에 수행한다.
5. **외부 harness 결과는 등급에 맞게 해석한다.** 같은 model이면 비교, 다른 model이면 참조점이다(§7.2).

---

## 2. Pipeline

```text
 harness마다 바뀌는 부분                 모든 harness 공통
 ─────────────────────────            ─────────────────────────────────────────
 [Driver] (선택)                        [Task spec]  evals/tasks/<task-id>/
  task를 harness에 전달하고 실행             │
        │                                  │
        ▼                                  │
 [Collector] (필수)                         │
  session 기록 → record.json   ───────▶  [Validate]  record.schema.json으로 검사
                                           ▼
                                        [Check]     task.yaml의 checks로 판정 → verdict.json
                                           ▼
                                        [Report]    run 집계 → report.md
```

`hel`은 record를 직접 출력하는 기준 구현이다. 따라서 `hel`의 collector는 출력된 record를 그대로 읽는다.

---

## 3. 디렉터리

```text
evals/
├── SPEC.md                         # 이 문서
├── labs/<lab>.yaml                 # Lab 정의: test set, 기본 model/budget, 채점 기준 (§8.1)
├── schema/
│   ├── record.md                   # record 필드 설명서
│   ├── record.schema.json          # record JSON Schema
│   ├── availability.md             # harness별 필드 수집 가능 여부
│   └── CHANGELOG.md
├── tasks/<task-id>/
│   ├── task.yaml                   # task spec
│   └── fixture/                    # run마다 새 임시 디렉터리로 복사되는 작업 디렉터리
└── harnesses/<harness>/
    ├── PROFILE.md                  # harness 특성, 격리 방법, 정책 검토 (§7.1)
    └── fixtures/                   # collector 적합성 test용 샘플 session 기록과 기대 record

crates/evals/                       # evals 명령: try, run, report (Rust)
```

---

## 4. Task spec

`evals/tasks/<task-id>/task.yaml`:

```yaml
id: read-echo-01
eval_version: eval-v0
description: 짧은 ASCII 한 줄 파일을 읽고 그대로 출력한다
instruction: >
  Read the file hello.txt and print its contents exactly as they are.
  Do not add anything else.
fixture: fixture/            # run마다 임시 디렉터리로 복사되어 harness의 작업 디렉터리가 된다
checks:
  - id: output-exact
    type: output_exact_match
    expected_file: fixture/hello.txt
  - id: single-read
    type: tool_calls
    category: read
    count: 1
    path: hello.txt          # 인자 경로를 작업 디렉터리 기준으로 resolve해 비교
    not_applicable: [baseline]
```

**세션 task (eval-v5)**: `instruction` 대신 `turns`에 지시 목록을 쓰면 harness가 한 세션에서 지시를 차례로 보낸다. 앞 지시의 대화는 다음 지시로 이어지고, record는 run당 하나이며 `outcome.final_output`은 마지막 지시의 답이다. `instruction`과 `turns` 중 정확히 하나만 쓴다. 지원하는 driver는 hel(`--turns-file`)뿐이며, claude-code driver는 세션 task를 실행하지 않고 오류를 낸다. `evals try --instruction`으로 세션 task의 지시를 바꿀 수 없다.

```yaml
turns:
  - Read services/alpha.toml and reply with only the value of port in its [service] section.
  - Without reading any file again, reply with only the owner of the alpha service.
```

### 4.1 Check 종류

| type | 판정 | 비고 |
| --- | --- | --- |
| `output_exact_match` | `outcome.final_output`이 `expected_file` 내용과 같다 | 양쪽 끝의 **줄바꿈 한 개**만 제거하고 비교한다. 그 외 공백 정리, code fence 제거 등은 하지 않는다 |
| `file_exact_match` | run이 끝난 뒤 작업 디렉터리의 `path` 파일이 `expected_file` 내용과 같다 (eval-v2) | runner가 run 종료 시 작업 디렉터리를 `workspace/`에 복사하고 그 사본을 읽는다. 줄바꿈 규칙은 `output_exact_match`와 같다. 파일이 없으면 fail |
| `tool_calls` | `events` 중 `category`가 일치하는 호출이 정확히 `count`회이고, `path`가 있으면 그 호출들의 대상 경로가 모두 `path`로 resolve된다 | 대상 경로는 `args.path` 또는 `args.file_path`. 상대/절대 모두 작업 디렉터리 기준으로 정규화해 비교한다 |
| `ini_value` | run이 끝난 뒤 작업 디렉터리의 `path` INI 파일에서 `[section]`의 `key` 값이 `value`와 같다 (eval-v3) | `workspace/` 사본을 읽는다. 키와 값의 앞뒤 공백, 빈 줄, `;`·`#` 주석 줄, 공백 뒤 inline 주석은 무시한다. `=` 주변 공백, 키 순서 같은 서식은 비교하지 않는다. 같은 섹션에 키가 여러 번 있으면 마지막 값. 파일이나 키가 없으면 fail |

- `not_applicable`에 적힌 condition에서는 해당 check 결과를 `n/a`로 기록한다.
- check 종류를 추가할 때는 이 표와 eval version을 갱신한다.

---

## 5. Run record

collector의 출력이다. 필드 정의는 [schema/record.md](schema/record.md), 기계 검증은 [schema/record.schema.json](schema/record.schema.json)을 따른다.

```text
results/<lab>/<run-id>/
├── record.json      # collector 출력
├── verdict.json     # checker 출력
├── workspace/       # run이 끝난 뒤 작업 디렉터리 사본 (eval-v2, symlink 제외)
└── raw/             # 원본 session 기록 사본
```

- record는 collector가 만든 뒤 수정하지 않는다.
- 수치 필드는 `{ "value": ..., "status": "measured" | "unavailable" | "derived" }` 형태다.
- record 필드를 추가하거나 의미를 바꿀 때는 `schema/record.md`, `schema/record.schema.json`, `schema/availability.md`, 각 collector와 test, `schema/CHANGELOG.md`를 함께 갱신하고 `schema_version`을 올린다.

---

## 6. Verdict

checker의 출력이다.

```json
{
  "schema_version": "verdict-v0",
  "run_id": "h00-read-echo-01-variant-01",
  "checks": [
    { "id": "output-exact", "result": "pass", "detail": "" },
    { "id": "single-read", "result": "fail", "detail": "read calls: 2" }
  ],
  "overall": "fail"
}
```

- `result`: `pass` | `fail` | `n/a`
- `overall`: `n/a`를 제외한 모든 check가 `pass`이면 `pass`
- `validity.valid: false`인 record는 판정하지 않고 `overall: invalid`로 기록한다.

---

## 7. Harness 추가 규칙

새 harness는 `evals/harnesses/<name>/`에 아래를 갖추고, 상태 단계를 거쳐 실험에 들어간다.

### 7.1 Profile (`PROFILE.md`)

- 버전, 설치 방법, headless 실행 방법
- model 연결 방법: 지원 API 형식, base URL과 model 설정 방식
- 기본 tool 구성, system prompt 수정 가능 여부
- session 기록의 위치와 형식
- tool 이름 → category 매핑표 (`read` / `search` / `edit` / `exec` / `other`)
- **정책 검토**: 약관상 제한, 다른 model 연결이 공식 지원되는지, 공개 시 표기 문구
- 현재 상태(§7.9)와 알려진 제한

### 7.2 비교 등급 (`comparison_class`)

| 값 | 의미 | 해석 |
| --- | --- | --- |
| `subject` | 직접 만들어 실험하는 harness (`hel`) | 실험 대상 |
| `same-model` | 실험 model로 실행 가능한 외부 harness | 비교 |
| `reference` | 다른 model만 쓸 수 있는 외부 harness | 참조점 |

### 7.3 최소 격리 요건

- **설정 격리**: 사용자 설정, plugin, MCP, memory를 읽지 않는다. run마다 별도 설정 디렉터리를 쓴다.
- **인증**: 인증 정보는 실행 시 환경 변수로 주입한다. `evals`는 `DEEPSEEK_API_KEY` 환경 변수가 없으면 저장소 루트의 `.env`(git 제외)에서 읽어 harness 실행 환경에 넣는다. 전역 설정에 저장하지 않는다. `raw/`에 남기는 원본 기록에는 API key나 `Authorization` 같은 인증 header를 포함하지 않는다.
- **외부 통신**: telemetry와 자동 업데이트를 가능하면 끈다. 실행한 버전을 record에 기록한다.
- **작업 디렉터리**: task fixture의 임시 복사본을 쓴다.
- **권한**: 사용자 환경에서 권한 확인을 우회하는 실행 모드를 쓰지 않는다. container 안에서는 허용한다.
- container 격리는 H8 또는 첫 Full 실험 때 도입한다.

### 7.4 Driver 규약 (선택)

- **입력**: task spec, run id, 작업 디렉터리, model 설정, budget
- **동작**: harness를 headless로 실행한다. timeout은 runner가 프로세스 단위로 강제한다.
- **출력**: 종료 상태와 session 기록 위치
- budget 항목마다 harness의 어떤 설정으로 맞췄는지 PROFILE에 기록한다. 맞출 수 없는 항목은 `unsupported`로 표시한다.
- driver가 없으면 사람이 instruction을 입력해 실행하고, hook 등으로 session 종료 시 collector를 실행한다.

### 7.5 Collector 규약 (필수)

- **입력**: 원본 session 기록 경로, run 정보(run id, task id, condition, harness 이름과 버전, 요청한 model)
- **출력**: record schema를 만족하는 `record.json` 1개, 원본 기록 사본(`raw/`)
- 필드마다 `measured` / `unavailable` / `derived`를 정확히 표시한다.
- tool 이름은 PROFILE의 매핑표로 category를 붙이고, 원래 이름(`name`)도 보존한다.
- session에서 실제 응답한 model 이름을 추출해 `model.actual`에 기록한다.

### 7.6 설정 일치 기록

reasoning/effort 수준, 허용한 tool 목록, system prompt 수정 여부를 Lab 정의의 condition별 `settings`에 기록한다. 완전히 같게 맞출 수 없는 항목은 차이를 기록하고 결론을 그만큼 제한한다.

### 7.7 실제 model 확인

`model.actual`이 Lab 정의의 model과 다르면 collector는 `validity.valid: false`와 사유를 기록한다. 해당 run은 집계에서 제외하지만 삭제하지 않는다.

### 7.8 적합성 판정

1. **결정적 test**: `fixtures/`의 샘플 session 기록을 넣으면 기대 record가 나온다.
2. **pipeline smoke**: H0 task(`read-echo-01`)로 Validate → Check → Report 전체 pipeline을 통과한다.

### 7.9 상태 단계

`draft`(PROFILE 작성) → `conformant`(§7.8 통과) → `active`(실험에 사용). `conformant` 이상만 Lab 정의의 condition에 넣을 수 있다.

---

## 8. Lab 정의, 명령, report

### 8.1 Lab 정의 파일

`evals/labs/<lab>.yaml`은 Lab의 유일한 측정 정의다. Lab ID(`h00`)는 파일 이름과 파일 안의 `lab:`이 같아야 한다. 측정을 시작하기 전에 확정해 commit하고, 결과를 본 뒤 바꾸면 그 이유를 기록한다.

| 필드 | 내용 |
| --- | --- |
| `lab`, `title`, `description` | Lab ID와 설명 |
| `revision` | 생략하면 1. 조건을 바꿔 다시 측정할 때 올린다. experiment ID가 `<lab>`(1) 또는 `<lab>-r<N>`(2 이상)이 되어 이전 run과 섞이지 않는다 |
| `model`, `budget` | 기본 model(provider, id, params)과 budget(max_turns, timeout_seconds, max_output_tokens) |
| `conditions` | 비교 조건. `optional: true`이면 harness가 설치되지 않았을 때 건너뛴다 |
| `tasks`, `repetitions` | test set(task ID 목록)과 반복 횟수 |
| `rubric` | 채점 기준 설명. 실제 판정은 각 `task.yaml`의 checks가 한다 |

결과는 `results/<lab>/`에 쌓인다.

### 8.2 명령

`evals`는 저장소 안 어느 폴더에서 실행해도 위로 올라가며 저장소 루트(`evals/SPEC.md`가 있는 곳)를 찾는다. Lab은 ID로 지정하고, 생략하면 `evals/labs/`에서 가장 큰 번호의 Lab을 쓴다.

| 명령 | 하는 일 | API 호출 | 결과 위치 |
| --- | --- | --- | --- |
| `evals try <task> [--lab] [--harness \| --condition] [--instruction] [--fixture] [--build]` | task 하나를 1회 실행하고 입력, tool 호출, 출력, check, usage를 터미널에 보여준다 | 1회 | `results/try/` |
| `evals run [lab] [--conditions a,b] [--force] [--build]` | condition × task × repetition 실행 후 판정, report | 있음 | `results/<lab>/<run-id>/`, `report.md` |
| `evals report [lab]` | 이미 있는 run을 다시 형식 검사, 판정하고 report를 쓴다 | 없음 | `report.md`, run별 `verdict.json` |

`try`의 기본값:

- harness는 `hel`. PATH에 설치된 `hel`이 있으면 그것을 쓰고, 없으면 `crates/hel`을 빌드해 쓴다(`run`도 같다). `--build`는 설치 여부와 관계없이 작업 폴더 코드를 빌드해 쓴다. 설치된 `hel`이 `crates/hel`·`crates/record` 소스보다 오래되었으면 경고하고, 기록되는 `harness.version`에 `-stale`을 붙인다.
- model과 budget은 Lab 정의(기본: 최신 Lab)에서 가져온다. task가 그 Lab의 test set에 없어도 된다.
- `--harness claude-code`이면 Lab 정의의 같은 harness 조건 설정을 쓴다.
- `--condition <name>`이면 Lab 정의에서 그 이름의 조건(harness와 settings)을 그대로 쓴다. 예: `evals try find-echo-01 --condition variant-bash`.
- `--instruction`, `--fixture`로 입력을 바꾸면 run에 `overridden`이 기록되고 판정은 참고용으로 표시한다.

`run`의 실행 순서:

1. condition × task × repetition 조합으로 run 목록을 만든다. 결과가 있는 run은 건너뛴다(`--force`면 다시 실행하고 덮어쓴다).
2. **baseline-first**: baseline은 Lab 시작 상태의 코드로 **코드를 고치기 전(W2.4)** 에 `--conditions baseline`으로 실행해 저장한다. variant와 external은 W3에서 실행한다. 예전 코드를 다시 빌드하지 않기 위한 순서다. 측정 도구는 이 순서를 강제하지 않으므로, 코드를 고친 뒤 baseline을 다시 실행하지 않는다.
3. run마다 fixture를 임시 디렉터리로 복사하고, driver로 실행하고, timeout을 강제하고, collector를 실행한다.
4. 실행이 끝나면 그 experiment의 모든 run(이전에 실행한 baseline 포함)을 판정하고 report를 쓴다.

### 8.3 Report

터미널에 condition × task 요약표를 출력하고, `results/<lab>/report.md`에 다음을 쓴다.

- 채점 기준(Lab 정의의 `rubric`)
- condition × task별 runs, valid, pass, check별 통과 수, 평균 token, model 호출 수, wall time. `*`는 derived 값
- run별 결과. 실패하거나 무효인 run은 이유를 함께 쓴다. `output_exact_match` 실패는 처음 다른 줄을 보여주고, 공백은 `·`, 탭은 `→`로 표시한다

---

## 9. 버전

| eval version | 내용 |
| --- | --- |
| eval-v0 | H0. task `read-echo-01`. check `output_exact_match`, `tool_calls`. record-v0, verdict-v0 |
| eval-v1 | H1. task `path-echo-01`, `find-echo-01` 추가. 조건 이름 `variant-<name>` 허용(record-v0 호환 확장), hel 조건의 `settings.tools` → `--tools`, `evals try --condition`, report에 tool 호출 수와 run별 tool 호출 |
| eval-v2 | H2. task `edit-line-01`, `edit-ambiguous-01`, `write-new-01`(보조 확인용, h02 Lab 정의 밖) 추가. check `file_exact_match`, run 폴더에 `workspace/`(작업 디렉터리 사본). record·verdict 형식 변경 없음 |
| eval-v3 | H3. task `env-checksum-01`, `rule-config-01` 추가. check `ini_value`. hel 조건의 `settings.env` → `--env`, `settings.context_file` → `--context-file`. hel 실행 PATH를 `/usr/bin:/bin:/usr/sbin:/sbin`으로 변경(Claude Code driver와 같게. 이전 `/usr/bin:/bin`). H3 Adopt 후 `settings.env: false` → `--no-env`, `settings.context_file: false` → `--no-context-file` 추가(hel 기본값이 켜짐으로 바뀜). record·verdict 형식 변경 없음 |
| eval-v4 | H4. task `search-config-01`, `trace-config-01` 추가, `find-echo-01` 재사용. 기존 `output_exact_match` 사용, record·verdict·check 형식 변경 없음. hel `settings.ripgrep: true`는 runner PATH의 rg를 run별 bin/에 복사해 고정 PATH 앞에 추가하고 raw/search-engine.json에 원본 경로·버전을 기록. 누락·false이면 기존 PATH 유지. 원본 없음·복사 실행 실패는 API 호출 전에 중단 |
| eval-v5 | H6. 세션 task(`turns`, §4) 형식과 task `session-recall-01` 추가. hel driver는 지시 목록을 `raw/turns.json`에 쓰고 `--turns-file`로 넘긴다. hel raw log 항목에 세션 지시 번호 `turn`, harness 보조 요청(예: compaction)에 `purpose`. hel 조건 `settings.compaction: {at_tokens, keep_recent_tokens}` → `--compact-at`/`--keep-recent`, `false` → `--no-compaction`. report에 요청 순서별 cache hit %·context Mermaid 그래프(`purpose` 항목은 그래프에서 빼고 따로 표시)와 run별 보조 요청. record·verdict·check 형식 변경 없음. `DEEPSEEK_API_KEY`가 환경에 없으면 저장소 루트 `.env`에서 읽음 |

### H7 permissions 연결 (eval-v6)

hel 조건의 `settings.access`는 `read-only` / `confirm` / `auto`이며 `--access`로 전달한다. 생략하면 인자를 추가하지 않아 이전 baseline을 실행할 수 있다. `approval_response`는 access=confirm에서만 사용하며 `approve` / `deny` / `unavailable`이다. approve/deny는 run별 `raw/approval-input.json`에 64개의 boolean 응답을 저장하고 `--approval-input`으로 전달한다. 각 ask마다 한 응답을 소비하며 파일을 다 쓰면 거절한다. unavailable은 입력 파일 없이 실행한다. 이 입력 대역은 eval용이고 실제 사용자 승인 측정이 아니다.

H7의 `raw/permissions.jsonl`은 hel 원본 진단 로그다. tool 호출마다 이름·인자, 접근 레벨, 작업 성격, 정책 판정, 승인 응답과 실행 진입 여부를 남긴다. `executed`는 tool 구현에 진입했다는 뜻이며 성공 여부는 record의 event.ok로 본다. record-v0 필드는 변경하지 않는다. 기존 tool_calls는 호출 시도를 세므로 실행 횟수로 해석하지 않는다.
