# Harness Engineering Lab

> **Build, break, measure, and evolve a coding harness from scratch.**

coding agent의 harness를 Rust로 직접 만들고, 각 설계 선택이 무엇을 바꾸는지 실험으로 확인하는 학습 프로젝트다. 작업 결과는 코드, 실험 기록, 학습 가이드로 남긴다.

학습 가이드: <https://jammer-droid.github.io/HEL/>

## 구성

| 경로 | 내용 |
| --- | --- |
| `crates/hel/` | 직접 만드는 harness `hel` |
| `crates/record/` | 실행 기록(record) 타입. `hel`과 `evals`가 함께 쓴다 |
| `crates/evals/` | 측정 도구 `evals`: task 실행, 채점, report |
| `evals/labs/` | Lab 정의: test set, 기본 model과 budget, 채점 기준 |
| `evals/tasks/` | task: 지시문, 입력 파일(fixture), 채점 규칙(checks) |
| `evals/schema/`, `evals/SPEC.md` | 실행 기록 형식과 측정 명세 |
| `evals/harnesses/` | harness별 특성과 실행 방법 (`hel`, `claude-code`) |
| `results/` | `evals` 실행 결과가 생기는 곳. git으로 관리하지 않는다 |
| `guide/` | 학습 가이드 원본 (mdBook). `main`에 올라가면 GitHub Pages로 자동 배포된다 |

## Lab 진행

각 Lab의 시작 상태는 `hXX` tag로 제공한다. Lab의 시작 상태는 이전 Lab의 완료 상태다.

## 준비

| 항목 | 용도 |
| --- | --- |
| Rust (`rustup`, `cargo`) | 빌드. 설치는 [rustup.rs](https://rustup.rs/) |
| DeepSeek API key | 실험 model(`deepseek-flash`) 호출 |
| Claude Code (선택) | 외부 harness 비교. 없으면 해당 조건은 건너뛴다 |

API key는 환경 변수로 넣는다. 코드나 파일에 적어 commit하지 않는다.

```bash
export DEEPSEEK_API_KEY="<발급받은 key>"
```

## 설치

```bash
cargo install --path crates/evals --locked --target-dir target
```

```bash
cargo install --path crates/hel --locked --target-dir target
```

`--locked`는 저장소의 `Cargo.lock` 버전 그대로 설치하고, `--target-dir target`은 저장소의 빌드 폴더(`target/`)를 재사용해 다시 설치할 때 바뀐 crate만 컴파일한다.

`hel`은 H0에서 직접 만든다. `h00`에는 `crates/hel`이 없으므로 H0를 마친 뒤 설치한다.

`evals`는 설치된 `hel`을 사용한다. `hel`이 설치되어 있지 않으면 `crates/hel`을 빌드해서 사용한다. `hel` 코드를 고친 뒤에는 다시 설치하거나 `--build` 옵션으로 작업 중인 코드를 실행한다. 설치된 `hel`이 소스보다 오래되었으면 `evals`가 경고한다.

## hel 사용

```bash
hel --instruction "Read the file hello.txt and print its contents exactly as they are."
```

- 실행한 폴더를 작업 디렉터리로 쓴다. `read_file` tool은 이 폴더 안의 파일만 읽는다.
- 출력과 오류 메시지는 영어다.

| 옵션 | 필수 | 설명 |
| --- | --- | --- |
| `--instruction "<text>"` | 예 | model에게 보낼 지시 |
| `--context <file>` | 아니요 | 실행 정보 파일(JSON). run ID, Lab, task, 조건, model, budget(최대 turn 수, timeout, 출력 token 한도)을 담는다. 없으면 기본값으로 실행한다 |
| `--record <file>` | 아니요 | 실행 기록(record)을 쓸 경로. 같은 폴더의 `raw/requests.jsonl`에 model 요청과 응답 원본도 남긴다(`Authorization` header 제외). `--context`와 함께 써야 한다 |

직접 실행할 때는 `--instruction`만 쓰면 된다. 답만 출력하고 기록은 남기지 않는다.

`--context`와 `--record`는 `evals`가 쓰는 옵션이다. `evals try`나 `evals run`은 run마다 이렇게 `hel`을 실행한다.

1. task의 `fixture/`를 임시 폴더에 복사해 작업 디렉터리로 쓴다.
2. Lab 정의의 model과 budget으로 `results/.../<run-id>/context.json`을 만든다.
3. `hel --instruction <task 지시문> --context <context.json> --record <record.json>`을 실행한다. 환경 변수는 `PATH`와 `DEEPSEEK_API_KEY`만 넘긴다.
4. budget의 timeout이 지나면 `hel`을 종료한다.
5. `hel`이 쓴 `record.json`을 읽어 채점한다.

기록 형식은 `evals/schema/record.md`, 실행 규약은 `evals/harnesses/hel/PROFILE.md`에 있다.

H0의 `hel`은 지시 하나를 받아 실행하고 끝나는 형태다. 대화형 실행은 H1 전에 따로 만든다.

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
