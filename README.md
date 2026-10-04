# Harness Engineering Lab

> **Build, break, measure, and evolve a coding harness from scratch.**

coding agent의 harness를 Rust로 직접 만들고, 각 설계 선택이 무엇을 바꾸는지 실험으로 확인하는 학습 프로젝트다. 작업 결과는 코드, 실험 기록, 학습 가이드로 남긴다.

harness 구현은 바이브 코딩으로 진행한다. 코드를 작성하는 것보다 harness가 어떻게 작동하는지 확인하는 것이 더 중요하기 때문이다.

학습 가이드: <https://jammer-droid.github.io/HEL/>

## 구성

| 경로 | 내용 |
| --- | --- |
| `crates/hel/` | 직접 만드는 harness `hel`. H0에서 만든다(`h00`에는 없다) |
| `crates/record/` | 실행 기록(record) 타입. `hel`과 `evals`가 함께 쓴다 |
| `crates/evals/` | 측정 도구 `evals`: task 실행, 채점, report |
| `evals/labs/` | Lab 정의: test set, 기본 model과 budget, 채점 기준 |
| `evals/tasks/` | task: 지시문, 입력 파일(fixture), 채점 규칙(checks) |
| `evals/schema/`, `evals/SPEC.md` | 실행 기록 형식과 측정 명세 |
| `evals/harnesses/` | harness별 특성과 실행 방법 (`hel`, `claude-code`) |
| `results/` | `evals` 실행 결과가 생기는 곳. git으로 관리하지 않는다 |
| `guide/` | 학습 가이드 원본 (mdBook). `main`에 올라가면 GitHub Pages로 자동 배포된다 |

## Lab 진행

각 Lab의 시작 상태는 [`hXX` tag](https://github.com/jammer-droid/HEL/tags)로 제공한다. Lab의 시작 상태는 이전 Lab의 완료 상태다.

| tag | 내용 |
| --- | --- |
| [`h00`](https://github.com/jammer-droid/HEL/tree/h00) | 기록 형식(`record`), 측정 도구(`evals`), H0 가이드. `hel`은 없다 |
| [`h01`](https://github.com/jammer-droid/HEL/tree/h01) | H0 완료 상태. `hel`(agent loop, `read_file`, 대화형 모드) |
| [`h02`](https://github.com/jammer-droid/HEL/tree/h02) | H1 완료 상태. `hel`에 bash tool, `--tools`로 tool 구성 선택(기본 bash) |
| [`h03`](https://github.com/jammer-droid/HEL/tree/h03) | H2 완료 상태. `hel`에 `write_file`, `search_replace` tool(`--tools`로 선택) |
| [`h04`](https://github.com/jammer-droid/HEL/tree/h04) | H3 완료 상태. `hel`이 실행 환경 정보와 작업 디렉터리의 `HEL.md`를 system message로 보냄(`--no-env`, `--no-context-file`로 끔) |
| [`h05`](https://github.com/jammer-droid/HEL/tree/h05) | H4 완료 상태 |
| [`h06`](https://github.com/jammer-droid/HEL/tree/h06) | H5 완료 상태 |
| `h07` *(예정)* | H6 완료 상태 |

```bash
git clone https://github.com/jammer-droid/HEL.git
```

Lab을 시작할 때는 그 Lab의 시작 tag에서 branch를 만든다. 예를 들어 H1은 `h01`에서 시작한다.

```bash
git checkout -b my-h01 h01
```

## 준비

| 항목 | 용도 |
| --- | --- |
| Rust (`rustup`, `cargo`) | 빌드. 설치는 [rustup.rs](https://rustup.rs/) |
| DeepSeek API key | 실험 model(`deepseek-flash`) 호출 |
| ripgrep (`rg`, H4부터) | 전용 검색 tool과 검색 측정. PATH에서 `rg --version`으로 확인 |
| Claude Code (선택) | 외부 harness 비교. 없으면 해당 조건은 건너뛴다 |

API key는 환경 변수로 넣는다. 코드나 파일에 적어 commit하지 않는다.

```bash
export DEEPSEEK_API_KEY="<발급받은 key>"
```

## 설치

```bash
cargo install --path crates/evals --locked --target-dir target
```

`--locked`는 저장소의 `Cargo.lock` 버전 그대로 설치하고, `--target-dir target`은 저장소의 빌드 폴더(`target/`)를 재사용해 다시 설치할 때 바뀐 crate만 컴파일한다.

`hel`은 H0에서 직접 만든다. `crates/hel`을 만든 뒤 설치한다(`h01`부터는 포함되어 있다).

```bash
cargo install --path crates/hel --target-dir target
```

`evals`는 설치된 `hel`을 사용한다. `hel`이 설치되어 있지 않으면 `crates/hel`을 빌드해서 사용한다. `hel` 코드를 고친 뒤에는 다시 설치하거나 `--build` 옵션으로 작업 중인 코드를 실행한다. 설치된 `hel`이 소스보다 오래되었으면 `evals`가 경고한다.

## hel 사용

```bash
hel
```

옵션 없이 실행하면 대화형으로 실행된다. 한 줄씩 입력하면 앞의 대화와 함께 model에게 보내고, `/exit`나 Ctrl-D로 끝낸다. tool 호출은 `[tool]` 줄로, 입력 하나가 끝날 때의 context 크기와 cache hit는 `[context: N tokens · cache hit M (P%)]` 줄로 표시된다.

```bash
hel --instruction "Read the file hello.txt and print its contents exactly as they are."
```

`--instruction`을 주면 지시 하나를 실행하고 끝난다.

- 실행한 폴더를 작업 디렉터리로 쓴다. `read_file`, `write_file`, `search_replace` tool은 이 폴더 안의 파일만 다룬다.
- `--tools`를 생략하면 `bash` tool 하나만 준다. model이 쓴 명령을 이 폴더에서 그대로 실행한다.
- H4 검색 tool은 `--tools bash,glob,grep`로 제공한다. PATH에 ripgrep(`rg`)이 필요하다. `glob`은 파일명, `grep`은 본문을 검색하고 결과가 100건·10KB를 넘으면 잘림을 표시한다.
- 실행 환경 정보(OS, shell, 작업 디렉터리)와 이 폴더의 `HEL.md`를 system message로 함께 보낸다. `HEL.md`가 없으면 환경 정보만 보낸다. 상위 폴더의 `HEL.md`는 읽지 않는다.
- 출력과 오류 메시지는 영어다.

> [!WARNING]
> `bash` tool은 명령을 확인 없이 실행하고, 작업 디렉터리 밖의 파일도 읽거나 바꿀 수 있다. 중요한 파일이 없는 폴더에서 실행한다. 읽기만 허용하려면 `--tools read_file`로 실행한다.

| 옵션 | 필수 | 설명 |
| --- | --- | --- |
| `--instruction "<text>"` | 아니요 | model에게 보낼 지시. 없으면 대화형으로 실행한다 |
| `--tools <a,b>` | 아니요 | model에게 줄 tool(`bash`, `read_file`, `write_file`, `search_replace`, `glob`, `grep`)을 쉼표로 나열한다. 없으면 `bash` |
| `--no-env` | 아니요 | 실행 환경 정보를 보내지 않는다 |
| `--context-file <name>` | 아니요 | `HEL.md` 대신 작업 디렉터리의 `<name>` 파일을 보낸다 |
| `--no-context-file` | 아니요 | context 파일을 보내지 않는다 |
| `--context <file>` | 아니요 | 실행 정보 파일(JSON). run ID, Lab, task, 조건, model, budget(최대 turn 수, timeout, 출력 token 한도)을 담는다. 없으면 기본값으로 실행한다. `--instruction`과 함께 써야 한다 |
| `--record <file>` | 아니요 | 실행 기록(record)을 쓸 경로. 같은 폴더의 `raw/requests.jsonl`에 model 요청과 응답 원본도 남긴다(`Authorization` header 제외). `--context`와 함께 써야 한다 |

직접 실행할 때는 대화형이나 `--instruction`만 쓰면 된다. 답만 출력하고 기록은 남기지 않는다.

`--context`와 `--record`는 `evals`가 쓰는 옵션이다. `evals try`나 `evals run`은 run마다 이렇게 `hel`을 실행한다.

1. task의 `fixture/`를 임시 폴더에 복사해 작업 디렉터리로 쓴다.
2. Lab 정의의 model과 budget으로 `results/.../<run-id>/context.json`을 만든다.
3. `hel --instruction <task 지시문> --context <context.json> --record <record.json>`을 실행한다. Lab 정의의 조건에 `settings.tools`가 있으면 `--tools`로, `settings.env`는 `--env`/`--no-env`로, `settings.context_file`은 `--context-file <name>`/`--no-context-file`로 넘긴다. 환경 변수는 고정 `PATH`와 `DEEPSEEK_API_KEY`만 넘긴다. 기본 PATH는 `/usr/bin:/bin:/usr/sbin:/sbin`이며, `settings.ripgrep: true`이면 run별 `bin/rg`를 준비해 그 폴더를 앞에 추가한다.
4. budget의 timeout이 지나면 `hel`을 종료한다.
5. `hel`이 쓴 `record.json`을 읽어 채점한다.

기록 형식은 `evals/schema/record.md`, 실행 규약은 `evals/harnesses/hel/PROFILE.md`에 있다.

## evals 사용

저장소 안 어느 폴더에서 실행해도 된다. Lab과 task는 ID로 지정한다.

| ID | 가리키는 파일 |
| --- | --- |
| Lab `h00` | `evals/labs/h00.yaml` |
| task `read-echo-01` | `evals/tasks/read-echo-01/` |

### task 하나 시험 실행

```bash
evals try read-echo-01
```

입력, tool 호출, 출력, 채점 결과, token 사용량을 터미널에 보여준다. 결과는 `results/try/`에 남는다.

| 옵션 | 기본값 | 설명 |
| --- | --- | --- |
| `--lab <lab>` | 최신 Lab | model과 budget을 가져올 Lab |
| `--harness <name>` | `hel` | `hel` 또는 `claude-code` |
| `--condition <name>` | 없음 | Lab 정의의 조건 하나(harness와 설정)로 실행한다. 예: `evals try find-echo-01 --condition variant-bash` |
| `--instruction "<text>"` | task의 지시문 | 이번 실행만 지시문을 바꾼다. 채점은 참고용으로 표시된다 |
| `--fixture <dir>` | task의 `fixture/` | 이번 실행만 입력 파일 폴더를 바꾼다. 채점은 참고용으로 표시된다 |

채점에 실패하면 처음 다른 줄을 보여준다. 공백은 `·`, 탭은 `→`로 표시된다.

### Lab 전체 측정

```bash
evals run h00 --conditions baseline
```

```bash
evals run h00 --conditions variant
```

- Lab의 test set을 조건별로 반복 실행하고 채점한 뒤 report를 만든다. 결과는 `results/<lab>/`에 남는다.
- baseline은 Lab을 시작할 때, 코드를 고치기 전에 먼저 실행한다. 코드를 고친 뒤 baseline을 다시 실행하면 바뀐 코드가 baseline으로 기록된다.
- 결과가 있는 run은 건너뛴다. 다시 실행하려면 `--force`를 붙인다(기존 결과를 덮어쓴다).
- `--conditions`를 생략하면 Lab 정의의 모든 조건을 실행한다.

### 결과 다시 보기

```bash
evals report h00
```

API를 호출하지 않고 다시 채점해 터미널 요약과 `results/<lab>/report.md`를 만든다.
