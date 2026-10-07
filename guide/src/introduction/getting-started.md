# 시작하기

## 준비물

| 도구 | 용도 | 비고 |
| --- | --- | --- |
| Rust toolchain (`rustup`, `cargo`) | harness와 측정 도구 빌드 | stable 최신 버전 |
| git | 소스 받기, Lab checkpoint 이동 | |
| ripgrep (`rg`, H4부터) | 파일명·본문 검색 tool과 검색 측정 | PATH에서 실행 가능해야 함. `rg --version`으로 확인 |
| DeepSeek API key | 실험 model 호출 | [비용, 정책, 윤리](cost-policy-ethics.md) 참고 |
| Claude Code (선택) | 외부 harness와 비교할 때 | 없어도 우리 harness의 실험은 모두 진행할 수 있다 |

### Rust 설치

Rust는 공식 설치 도구인 `rustup`으로 설치한다. 설치 방법은 [rustup.rs](https://rustup.rs/)를 따른다. 설치한 뒤 다음 명령이 버전을 출력하면 된다.

```bash
cargo --version
```

### DeepSeek API key

이 가이드의 실험은 DeepSeek의 `deepseek-flash` model을 쓴다. 이 model을 고른 이유와 그 한계는 [측정 방법](measurement.md)에서 다룬다.

1. [DeepSeek Platform](https://platform.deepseek.com/)에서 계정을 만들고 API key를 발급한다.
2. key를 저장소 루트의 `.env` 파일에 적는다. `evals`는 환경 변수 `DEEPSEEK_API_KEY`가 없으면 이 파일을 읽는다.

```bash
echo 'DEEPSEEK_API_KEY=<발급받은 key>' > .env
```

`hel`을 직접 실행할 때는 환경 변수로 넣는다.

```bash
export DEEPSEEK_API_KEY="<발급받은 key>"
```

> [!WARNING]
> `.env`는 저장소의 `.gitignore`에 들어 있어 commit되지 않는다. key를 코드나 다른 설정 파일에 적어 commit하지 않는다.

## 소스 받기

소스 코드와 이 가이드의 원본은 [jammer-droid/HEL](https://github.com/jammer-droid/HEL)에 있다.

```bash
git clone https://github.com/jammer-droid/HEL.git
```

## 소스 트리

저장소의 구조는 다음과 같다.

| 경로 | 내용 |
| --- | --- |
| `crates/hel/` | 이 가이드에서 만드는 harness. 이름은 **hel**(Harness Engineering Lab)이다. H0에서 만든다(`h00`에는 없다) |
| `crates/record/` | 실행 기록(record)의 데이터 타입. harness와 측정 도구가 함께 쓴다 |
| `crates/evals/` | 측정 도구 `evals`: task 실행, 기록 수집, 판정, 집계 |
| `evals/labs/` | Lab별 test set, 기본 model과 budget, 채점 기준 |
| `evals/tasks/` | 실험에 쓰는 task와 그 입력 파일(fixture) |
| `evals/harnesses/` | 측정 대상 harness별 특성과 실행 방법(profile) |
| `evals/schema/` | 실행 기록의 형식 정의 |
| `results/` | `evals` 실행 결과가 생기는 곳. git으로 관리하지 않는다 |
| `guide/` | 지금 읽고 있는 이 가이드의 원본 |

## Lab checkpoint 사용하기

각 Lab의 시작 상태는 [`hXX` tag](https://github.com/jammer-droid/HEL/tags)로 제공한다. Lab의 시작 상태는 이전 Lab을 완성한 상태다.

| tag | 내용 |
| --- | --- |
| [`h00`](https://github.com/jammer-droid/HEL/tree/h00) | 기록 형식(`record`)과 측정 도구. harness는 없음 |
| [`h01`](https://github.com/jammer-droid/HEL/tree/h01) | H0 완성 상태. 대화형 `hel` 포함 |
| [`h02`](https://github.com/jammer-droid/HEL/tree/h02) | H1 완성 상태. `hel`에 bash tool 포함 |
| [`h03`](https://github.com/jammer-droid/HEL/tree/h03) | H2 완성 상태. `hel`에 편집 tool 포함 |
| [`h04`](https://github.com/jammer-droid/HEL/tree/h04) | H3 완성 상태. `hel`이 OS·shell·작업 경로와 `HEL.md`를 시스템 프롬프트로 보냄 |
| [`h05`](https://github.com/jammer-droid/HEL/tree/h05) | H4 완성 상태 |
| [`h06`](https://github.com/jammer-droid/HEL/tree/h06) | H5 완성 상태 |
| [`h07`](https://github.com/jammer-droid/HEL/tree/h07) | H6 완성 상태. `hel`이 context를 기본으로 압축함 |
| [`h08`](https://github.com/jammer-droid/HEL/tree/h08) | H7 완성 상태. 접근 레벨과 호출별 승인 |
| [`h09`](https://github.com/jammer-droid/HEL/tree/h09) | H8 완성 상태. macOS sandbox와 인스턴스별 저장 공간 |
| [`h10`](https://github.com/jammer-droid/HEL/tree/h10) | H9 완성 상태. 세션 snapshot·재개·세션별 spill |
| [`h11`](https://github.com/jammer-droid/HEL/tree/h11) | H10 완성 상태. Skills 목록·Hooks·지연 로딩 MCP tool |
| [`h12`](https://github.com/jammer-droid/HEL/tree/h12) | H11 완성 상태. subagent에게 작업을 순차 위임하는 `delegate_task` |
| [`h13`](https://github.com/jammer-droid/HEL/tree/h13) | H12 완성 상태. 읽기 전용 tool과 subagent를 동시에 실행하는 `--parallel` |
| [`h14`](https://github.com/jammer-droid/HEL/tree/h14) | H13 완성 상태. tool 호출의 exit code·실패 결과 기록, `evals failures`와 pass@k·pass^k |
| `hXX` | H(XX-1) 완성 상태 |

Lab을 시작할 때는 해당 tag에서 branch를 만든다. 예를 들어 H1은 `h01`에서 시작한다.

```bash
git checkout -b my-h01 h01
```

한 Lab에서 무엇이 바뀌었는지는 다음 tag와 비교해 본다. 예를 들어 H0에서 바뀐 것은 이렇게 본다.

```bash
git diff h00..h01
```

tag는 각 Lab이 끝날 때 만든다.

## 명령 설치

이 가이드에서 쓰는 명령은 두 가지다.

| 명령 | 역할 |
| --- | --- |
| `hel` | 직접 만드는 harness. 실행한 폴더를 작업 디렉터리로 쓴다 |
| `evals` | task를 실행하고 채점하는 측정 도구. 저장소 안에서 실행한다 |

저장소 루트에서 설치한다.

```bash
cargo install --path crates/evals --locked --target-dir target
```

`hel`은 각 Lab에서 직접 만들고, 고칠 때마다 다시 설치한다.

```bash
cargo install --path crates/hel --locked --target-dir target
```

`--locked`는 저장소의 `Cargo.lock` 버전 그대로 설치하고, `--target-dir target`은 저장소의 빌드 폴더(`target/`)를 재사용해 다시 설치할 때 바뀐 crate만 컴파일한다.

`evals`는 설치된 `hel`을 사용한다. 설치되어 있지 않으면 `crates/hel`을 빌드해서 사용한다. 설치하지 않고 작업 중인 코드를 바로 실행하려면 `--build`를 붙인다. 설치된 `hel`이 소스보다 오래되었으면 경고가 나온다.

## task 실행과 채점

Lab을 진행하면서 task 하나를 바로 실행해 볼 때는 `evals try`를 쓴다.

```bash
evals try read-echo-01
```

입력, tool 호출, 출력, 채점 결과, token 사용량이 터미널에 나온다. 결과는 `results/try/`에 남는다. 지시문만 바꿔서 시험하려면 `--instruction`을 붙인다.

```bash
evals try read-echo-01 --instruction "Read hello.txt and print it in uppercase."
```

Lab을 마칠 때는 그 Lab의 test set 전체를 실행하고 채점한다. baseline은 Lab을 시작할 때, 코드를 고치기 전에 먼저 실행한다.

```bash
evals run h00 --conditions baseline
```

```bash
evals run h00 --conditions variant
```

이미 실행한 결과를 다시 채점하고 report만 볼 때는 `evals report h00`을 쓴다. Lab ID를 생략하면 가장 최근 Lab을 쓴다. report는 `results/<lab>/report.md`에 저장된다.

## 이 가이드를 로컬에서 보기

이 가이드는 [mdBook](https://rust-lang.github.io/mdBook/)으로 만들고, 한국어 검색은 [Pagefind](https://pagefind.app/)로 제공한다. 로컬에서 빌드하는 방법은 저장소의 `guide/README.md`에 있다.
