# H1 — Tool Loop

> [!NOTE]
> - 시작 상태: [`h01`](https://github.com/jammer-droid/HEL/tree/h01) · 완료 상태: [`h02`](https://github.com/jammer-droid/HEL/tree/h02)
> - 논문: [§8 Tool and Action Systems](https://arxiv.org/html/2609.00006v1#S8), [§16.3 Tool Design](https://arxiv.org/html/2609.00006v1#S16.SS3)

```bash
git checkout -b my-h01 h01
```

bash 하나만 주면 model은 어디까지 스스로 해낼 수 있을까?

## 들어가며

### 전용 tool과 bash tool

H0의 `hel`에는 `read_file` tool 하나만 있다. model이 파일 경로를 넘기면 harness가 그 파일을 읽어 내용을 돌려준다. 이렇게 한 가지 일만 하도록 만든 tool을 이 글에서는 **전용 tool**이라고 부른다.

**bash tool**은 명령 문자열 하나를 받는다. harness는 그 문자열을 shell에서 실행하고 종료 코드와 출력을 돌려준다. 파일을 읽을지(`cat`), 목록을 볼지(`ls`), 파일을 찾을지(`find`)는 model이 명령을 쓰면서 정한다.

| | 전용 tool (`read_file`) | bash tool |
| --- | --- | --- |
| model이 넘기는 것 | 파일 경로 | 명령 문자열 |
| 할 수 있는 일 | tool을 만들 때 정한 일 하나 | shell에서 되는 모든 일 |
| harness가 아는 것 | 하려는 일(읽기)과 대상 경로 | 문자열뿐. 하려는 일은 명령을 해석해야 앎 |
| 결과 형식 | harness가 정함 | 명령마다 다름. 종료 코드, stdout, stderr |
| 실패했을 때 | harness가 정한 오류 메시지 | 명령이 낸 오류와 종료 코드 |

bash tool 하나를 주면 tool을 더 만들지 않아도 할 수 있는 일이 늘어난다. 그 대신 harness는 명령이 무엇을 하는지 모르고, 결과의 크기와 형식도 정하지 못한다.

### 논문은 tool을 어떻게 보나

[Harness Engineering 논문](https://arxiv.org/html/2609.00006v1)의 §8은 tool을 agent가 실제로 할 수 있는 일을 정하는 부분으로 보고, 성숙한 harness일수록 여기에 engineering을 많이 쓴다고 했다.

- harness마다 tool 수 차이가 크다.

| Harness | 기본 tool 수 |
| --- | --- |
| Mini-SWE-Agent | 1 (bash) |
| Pi | 7 (기본 노출 4) |
| OpenCode | 17 |
| Claude Code | 43 |
| Hermes | 69 |

- tool을 감싸는 정도도 다르다. Mini-SWE-Agent는 모든 action을 shell 호출 하나로 보내고, 시간이 넘으면 프로세스를 정리하는 것만 감싼다. Claude Code는 tool마다 입력 검증, 권한 확인, 동시 실행 가능 여부, 화면 표시를 따로 정의한다. 논문은 감싸는 범위가 넓을수록 안전 장치는 늘지만 만드는 비용도 커진다고 정리했다.
- model에게 주는 지시도 갈린다([§7.3](https://arxiv.org/html/2609.00006v1#S7.SS3)). Claude Code는 전용 tool이 있으면 Bash를 쓰지 말라고 지시한다. 사용자가 작업을 이해하고 검토하기 쉽다는 이유다. Mini-SWE-Agent는 응답마다 bash 명령 하나만 쓰게 한다.

§16.3의 권고는 bash tool 하나로 시작하고, 실패를 관찰했을 때만 tool을 더하라는 것이다. 근거로 Mini-SWE-Agent가 bash 하나로 SWE-bench Verified에서 74% 이상을 보고한 것을 든다(Mini-SWE-Agent 측의 자체 보고 수치). tool을 더하는 순서도 적었다.

> [!NOTE]
> `h01` 이후에는 논문의 권고 사항을 반영하여 `hel`의 기본 사용 도구를 `bash`로 설정할 것이다.

- bash 출력이 잘리는 것이 문제가 되면 `read_file`, `write_file`
- `find`나 `rg`로 찾기가 불편하면 grep, glob
- 파일 전체를 다시 쓰느라 token이 낭비되면 `search_replace`

[§15.5](https://arxiv.org/html/2609.00006v1#S15.SS5)는 harness 구조를 더할수록 task 성공률이 처음에는 빠르게 오르다가 평평해진다는 직관을 제시한다. 빠르게 오르는 구간의 예로 bash tool과 읽기·쓰기 tool을 든다. 이번 Lab에서는 이 직관을 간단하게 테스트해볼 것이다.

### 참고 자료

- [Mini-SWE-Agent](https://github.com/SWE-agent/mini-swe-agent/tree/04d809ceab9df28f9adaed044884180159172930)의 기본 설정은 tool calling API로 `bash` tool 하나(인자 `command`)를 보낸다. (텍스트에서 명령을 찾는 방식은 선택 설정으로 남아 있다. 2026-10 기준.)
  - 명령은 호출마다 새 shell에서 실행한다. 그래서 `cd`가 다음 호출로 이어지지 않는다.
  - 종료 코드와 출력을 함께 돌려준다. 출력이 10,000자를 넘으면 앞뒤 5,000자만 보내고 `head`, `tail`, `sed`로 일부만 보라고 안내한다.
  - 기본 timeout은 30초이고, 넘으면 명령이 만든 프로세스를 모두 종료한다.
- Claude Code([Tools reference](https://code.claude.com/docs/en/tools-reference), 2026-10 기준)는 Bash와 전용 Read를 함께 둔다.
  - Read는 줄 번호를 붙여 돌려주고, 큰 파일은 일부만 보여 주며 이어 읽는 방법을 알려 준다. 디렉터리는 읽지 못해서 `ls`는 Bash로 한다.
  - macOS와 Linux에서는 파일 찾기(Glob)와 내용 검색(Grep) 전용 tool을 기본 목록에서 빼고 Bash의 `find`, `grep`을 쓰게 했다.
  - Bash 결과는 약 30,000자까지 그대로 보내고, 넘으면 파일로 저장한 뒤 앞부분만 보여 준다.

## 이번에 해볼 것

출발 상태는 `h01`의 `hel`이다. tool은 `read_file` 하나이고, 대화형 모드와 `--instruction` 실행을 모두 지원한다.

여기에 bash tool을 만들고, model에게 주는 tool 구성을 실행할 때 고를 수 있게 한다(`--tools`). 같은 task를 세 가지 구성으로 실행해 비교한다.

| 구성 | 주는 tool | 확인할 것 |
| --- | --- | --- |
| 시작 상태 | `read_file` | 비교 기준 |
| bash만 | `bash` | bash로 읽기와 찾기를 해내는가 |
| bash + 읽기 tool | `bash`, `read_file` | 둘 중 무엇을 고르는가 |

세 구성 모두 비용(token, model 호출 수, 시간)을 함께 기록하고, 결과를 보고 전용 읽기 tool이 무엇을 더해 주는지 판단한다.

task는 두 개다. 각 구성에서 task마다 3번씩 실행한다.

| task | 지시 | 확인할 것 |
| --- | --- | --- |
| `path-echo-01` | `memo.txt`를 읽고 내용을 그대로 출력 | 경로를 알 때의 읽기 |
| `find-echo-01` | 이름이 `target.txt`인 파일을 찾아 내용을 그대로 출력. 위치는 알려 주지 않음 | 파일 찾기. 실제 위치는 `archive/2024/q3/kv9/target.txt` |

채점은 H0와 같이 최종 출력이 파일 내용과 정확히 같은지만 본다. 어떤 tool을 어떤 명령으로 썼는지는 채점하지 않고 실행 기록으로 확인한다.

## 결과 확인

### 시작 상태로 실행

먼저 `h01`의 `hel`(tool은 `read_file` 하나)로 두 task를 3번씩 실행했다. `evals`에서는 이 구성을 `baseline` 조건이라고 부른다.

```bash
evals run h01 --conditions baseline
```

| task | 통과 | input / output token (평균) | model 호출 (평균) | tool 호출 (평균) |
| --- | --- | --- | --- | --- |
| `path-echo-01` | 3/3 | 772 / 95 | 2 | `read_file` 1.0 |
| `find-echo-01` | 0/3 | 3520 / 1342 | 4 | `read_file` 27.3 |

(`hel` `696916f`, model `deepseek-flash`, 최대 model 호출 5번.)

`find-echo-01`의 tool 호출을 보면 경로를 추측하고 있다. 첫 번째 실행은 이렇게 시작했다.

```text
read_file target.txt         (error)
read_file ./target.txt       (error)
read_file data/target.txt    (error)
read_file files/target.txt   (error)
read_file docs/target.txt    (error)
read_file src/target.txt     (error)
read_file tmp/target.txt     (error)
```

- 세 번 모두 파일을 찾지 못했다. 시도한 경로는 7개, 53개, 22개였다. model은 한 번의 응답에 `read_file` 호출 여러 개를 한꺼번에 보냈다.
- 두 번째 실행은 `archive`, `docs`, `src` 같은 이름을 파일처럼 읽으려 했다. 실제로 있는 폴더라 "Is a directory" 오류를 받았지만, 그 안으로 내려가기 전에 model 호출 5번을 모두 써서 멈췄다.
- 내용을 지어내지는 않았다. 세 번 모두 목록을 보거나 검색할 수단이 없어 찾지 못했다고 답했다. 세 번째 실행은 "28개 경로를 시도했다"고 답했지만 기록된 호출은 22개였다.
- 경로를 알려 준 `path-echo-01`은 `read_file`을 한 번 호출하고 끝났다.

### bash tool 만들기

bash tool은 명령 문자열을 `bash -c`로 작업 디렉터리에서 실행하고, 종료 코드와 출력을 문자열 하나로 돌려준다.

```rust
fn bash(workdir: &Path, args: &Value) -> Result<String, String> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or("missing string argument: command")?;
    let output = Command::new("bash")
        .args(["-c", command])
        .current_dir(workdir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not start bash: {e}"))?;
    let code = output
        .status
        .code()
        .map_or("killed".to_string(), |c| c.to_string());
    let text = format!(
        "exit={code}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() { Ok(text) } else { Err(text) }
}
```

- 결과의 첫 줄은 `exit=<종료 코드>`이고, 그 뒤에 stdout과 stderr를 이어 붙인다.
- stdin을 닫아서, 인자 없는 `cat`처럼 입력을 기다리는 명령도 바로 끝난다. 명령마다 시간 제한을 따로 두지 않고, run 전체 시간 제한(120초)에 맡긴다.
- 출력은 자르지 않는다. 이번 task는 출력이 짧다.
- 종료 코드가 0이 아니면 오류로 돌려준다. model은 어느 쪽이든 종료 코드와 출력을 모두 받는다.

model에게 줄 tool은 `--tools`로 고른다. 주지 않은 tool을 model이 호출하면 실행하지 않고 `unknown tool`로 답한다. `--tools`를 생략하면 bash 하나만 준다.

```bash
hel --tools bash,read_file
```

### bash만 줬을 때, bash와 읽기 tool을 함께 줬을 때

```bash
evals run h01 --conditions variant-bash,variant-bash-read
```

`variant-bash`는 bash만 준 구성, `variant-bash-read`는 bash와 `read_file`을 함께 준 구성이다. 시작 상태의 결과와 함께 적었다.

| task | 구성 | 통과 | input / output token (평균) | model 호출 | tool 호출 (평균) |
| --- | --- | --- | --- | --- | --- |
| `path-echo-01` | 시작 상태 (`read_file`) | 3/3 | 772 / 95 | 2 | `read_file` 1.0 |
| | bash만 | 3/3 | 810 / 143 | 2 | `bash` 1.0 |
| | bash + `read_file` | 3/3 | 949 / 121 | 2 | `read_file` 1.0 |
| `find-echo-01` | 시작 상태 (`read_file`) | 0/3 | 3520 / 1342 | 4 | `read_file` 27.3 |
| | bash만 | 3/3 | 1447 / 179 | 3 | `bash` 2.0 |
| | bash + `read_file` | 2/3 | 1604 / 290 | 3 | `bash` 1.7, `read_file` 0.3 |

(`hel` `091ee2c`, model `deepseek-flash`, 구성마다 task당 3번.)

bash만 준 `find-echo-01`은 세 번 모두 같은 순서로 풀었다.

```text
bash find . -name target.txt -type f 2>/dev/null
bash cat ./archive/2024/q3/kv9/target.txt
```

- 경로를 알 때는 `cat memo.txt` 한 번, 모를 때는 `find`로 찾은 뒤 `cat`으로 읽었다. 실행마다 `find`에 붙인 옵션(`-type f`, `2>/dev/null`)만 조금 달랐다.
- 시작 상태와 비교하면 `find-echo-01`의 tool 호출은 평균 27.3번에서 2번, input token은 3520에서 1447, 걸린 시간은 7.1초에서 2.5초로 줄었다.
- 경로를 알 때는 bash 쪽이 조금 더 비쌌다. input token이 772에서 810으로 늘었다. bash tool 설명이 더 길고, 결과 앞에 `exit=0` 줄이 붙는다.

bash와 `read_file`을 함께 주면 model은 상황에 따라 나눠 썼다.

- 경로를 알려 준 `path-echo-01`은 세 번 모두 `read_file`을 골랐다.
- `find-echo-01`은 세 번 모두 bash `find`로 위치를 찾았다. 찾은 파일은 두 번은 bash `cat`, 한 번은 `read_file`로 읽었다.
- 실패한 한 번은 `read_file`로 정확한 내용을 읽었지만, 최종 답을 code block(```` ``` ````)으로 감싸서 출력이 파일 내용과 달라졌다.
- tool 정의가 두 개라 input token이 가장 많았다(`path-echo-01` 949).

## 돌아보기

### 변경 사항

- `hel`이 bash tool을 갖게 됐다. model은 tool 하나로 파일 읽기(`cat`)와 찾기(`find`)를 스스로 골라 썼다. 읽기 전용 도구만 가지고 있는 상태에서 파일 이름만 전달하면 한 번도 찾지 못했지만, bash를 주자 세 번 모두 찾았다.
- 두 tool을 함께 주면 model은 경로를 알 때 `read_file`, 찾을 때 bash를 골랐다. 찾은 파일을 읽을 때는 세 번 중 두 번 bash를 이어 썼다.
- model에게 전달할 tool을 실행할 때 `--tools`로 고른다. 아무것도 주지 않으면 bash 하나로 시작한다.

### 트레이드오프

- 경로를 아는 읽기는 조금 비싸졌다. bash만 주면 input token이 772에서 810으로, 두 tool을 함께 주면 949로 늘었다. tool 정의가 길어지고 늘어나는 만큼 매 호출에 붙는다. 다만, 현재 수준으로는 유의미한 비용 차이가 발생했다고 보긴 어렵다.
- harness가 명령의 내용을 모른다. `read_file`은 작업 디렉터리 밖을 막았지만 bash에는 그런 검사가 없다. `bash`와 같은 범용 도구를 사용하는 경우 발생하는 작업 범위와 권한의 문제는 H7, 실행을 가두는 문제는 H8에서 다룬다.
- 출력 크기를 정하지 못한다. 큰 파일을 `cat`하면 출력 전체가 대화에 들어간다. 이번 task는 출력이 짧아서 차이가 없었다.

### 논문의 내용 또는 다른 harness와 비교하면

논문 §16.3의 권고대로 bash 하나로 시작해도 읽기와 찾기는 충분했다. 논문에서 말하는 "tool을 추가해야 하는 신호"(출력이 잘리는 문제, 찾기가 불편한 문제)는 이번 task에서 나타나지 않았다.

Mini-SWE-Agent는 이번 `hel`과 같은 구성(tool calling으로 bash 하나)이다. 다른 점은 출력이 10,000자를 넘으면 앞뒤만 보내고, 명령마다 30초 시간 제한을 둔다는 것이다. 지금의 `hel`은 둘 다 없다.

Claude Code는 macOS와 Linux에서 찾기(Glob, Grep)를 bash로 넘기고 읽기(Read)는 전용 tool로 남겼다. 이번에 두 tool을 함께 준 `hel`에서 model이 나눠 쓴 방식과 같다. Claude Code는 전용 tool을 쓰면 사용자가 작업을 이해하고 검토하기 쉽다고 설명하고, Read에는 큰 파일 일부 읽기와 이미지·PDF 처리 같은 기능을 따로 두었다.
