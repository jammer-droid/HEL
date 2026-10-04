# 대화형 기본 틀

> [!NOTE]
> - 이 페이지의 코드는 H0 완료 상태 [`h01`](https://github.com/jammer-droid/HEL/tree/h01)에 들어 있다.

H0의 `hel`은 `--instruction`으로 지시 하나를 받아 loop를 끝까지 돌리고 종료한다. 측정에는 이 방식으로 충분하다. 하지만 H1부터 harness를 키우면서 직접 써 보려면 사람과 주고받는 틀이 필요하다. 이 페이지에서는 그 틀만 만든다. 새 tool이나 loop 변경은 없다.

## 무엇을 바꾸나

H0의 agent loop(`run_loop`)는 대화 기록 `messages`를 빌려 받아, 그 안에 assistant 응답과 tool 결과를 덧붙인다. 대화형 모드는 이 loop를 한 번 더 감싸기만 하면 된다.

```text
사람 입력 한 줄
  → messages에 user message 추가
  → run_loop (model 호출 ↔ tool 실행, 최대 max_turns번)
  → 최종 답 출력
  → 다음 입력 대기 (messages는 그대로 유지)
```

model이 이전 호출을 기억하지 못하지만, 앞 대화를 기억하는 것처럼 보이는 이유는 `messages`에서 전체 내용을 매번 다시 보내는 것 덕분이다. 기존 loop 안에서 전체 내용을 누적해 전달하는 것과 같은 원리이다.

CLI의 사용 방법은 다음과 같다.

| 항목 | 동작 |
| --- | --- |
| 진입 | `--instruction` 없이 `hel` 실행 |
| 기존 실행 | `--instruction`, `--context`, `--record`는 그대로. `--context`는 `--instruction`과 함께일 때만 허용 |
| 입력 | 한 줄씩. 빈 줄은 무시 |
| 종료 | `/exit` 또는 입력 끝(Ctrl-D) |
| turn 한도 | 입력 하나마다 `max_turns`(기본 5)번까지 model 호출 |
| tool 호출 표시 | 입력 하나가 끝난 뒤 `[tool] 이름 인자` 형식으로 stderr에 출력 |
| 실행 기록 | 남기지 않음 |

## 만들기

### 1. `--instruction`을 선택 인자로

`Args.instruction`을 `Option<String>`으로 바꾸고, 인자 해석에서 필수 검사를 뺀다. 대화형 모드는 record를 쓰지 않으므로 `--context`만 있고 `--instruction`이 없는 조합은 막는다.

```rust
struct Args {
    instruction: Option<String>,
    context: Option<PathBuf>,
    record: Option<PathBuf>,
}
```

```rust
    if context.is_some() && instruction.is_none() {
        return Err("--context needs --instruction".to_string());
    }
```

`run()`에서는 client와 tool 정의를 만든 뒤 지시가 없으면 대화형 함수로 넘긴다.

```rust
    let Some(instruction) = &args.instruction else {
        return chat(&client, &workdir, &tool_defs, ctx.budget.max_turns);
    };
```

### 2. 입력 loop

`messages`는 loop 밖에서 한 번만 만들고, 실행 기록용 `RunLog`는 입력마다 새로 만든다. `RunLog`의 종료 사유와 최종 답이 입력 하나 단위로 맞게 된다.

```rust
fn chat(
    client: &api::Client,
    workdir: &Path,
    tool_defs: &Value,
    max_turns: u32,
) -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut messages = Vec::new();
    eprintln!("hel — type /exit or press Ctrl-D to quit");
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            println!();
            return Ok(());
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if input == "/exit" {
            return Ok(());
        }

        messages.push(json!({ "role": "user", "content": input }));
        let mut log = RunLog::new();
        run_loop(client, workdir, tool_defs, &mut messages, max_turns, &mut log);
        // 3. 결과 출력
    }
}
```

- `print!`은 줄바꿈이 없어서 바로 화면에 나오지 않는다. `flush()`로 내보내야 `> `가 입력 전에 보인다.
- `read_line`은 읽은 byte 수를 돌려준다. 0이면 입력이 끝난 것(Ctrl-D, 또는 pipe가 닫힘)이다.

### 3. 결과 출력

```rust
        for event in &log.events {
            let status = if event.ok == Some(false) { " (failed)" } else { "" };
            eprintln!("[tool] {} {}{status}", event.name, event.args);
        }
        if let Some(output) = &log.final_output {
            println!("{output}");
        }
        if let Some(error) = &log.error {
            eprintln!("hel: {error}");
        }
        if log.termination == Termination::MaxTurns {
            eprintln!("hel: stopped after {max_turns} model calls");
        }
```

답은 stdout, tool 호출과 오류는 stderr로 나눈다. H0의 단발 실행과 같은 규칙이다. 오류가 나도 프로그램을 끝내지 않고 다음 입력을 받는다.

## 써 보기

```bash
cargo run -q -p hel
```

```text
hel — type /exit or press Ctrl-D to quit
> hi
Hi! What can I help you with today?

If you have a file in the working directory you'd like me to read or look over, just let me know its path.
> 안녕
안녕하세요! 무엇을 도와드릴까요?

작업 디렉터리에 읽어볼 파일이 있다면 경로를 알려주세요.
> 너가 사용할 수 있는 도구가 뭔데
제가 사용할 수 있는 도구는 딱 하나입니다:

- **read_file** : 작업 디렉터리에 있는 UTF-8 텍스트 파일을 읽어서 전체 내용을 반환합니다.

**사용 방법**
- 파라미터: `path` (작업 디렉터리를 기준으로 한 상대 경로)
- 예: `read_file(path="notes.txt")`

**할 수 없는 것**
- 파일 쓰기/수정/삭제
- 디렉터리 목록 조회나 파일 검색
- 명령어 실행

그래서 파일 이름을 정확히 알려주시면 제가 내용을 읽어서 요약, 번역, 분석, 코드 리뷰 등을 도와드릴 수 있습니다. 어떤 파일을 열어볼까요?
>
```

- 세 입력 모두 tool을 호출하지 않고 답했다. 그래서 `[tool]` 줄이 없다.
- `hel`은 system prompt를 보내지 않는다. 그런데 "hi"에 대한 첫 답부터 파일 경로를 묻는다. model이 받은 것은 대화와 tool 정의뿐이고, 그것만 보고 자기 역할을 파일을 읽어 주는 쪽으로 잡았다.
- 도구 설명은 `read_file` 정의의 `description`("Read a UTF-8 text file in the working directory and return its full contents.")과 `path` 설명을 거의 그대로 옮겼다. "할 수 없는 것" 목록은 정의에 없는 내용이다. 다른 tool이 없다는 데서 model이 추론했다.
- 대화를 기억하는지는 이 기록으로 알 수 없다. 앞 대화를 참조하는 입력이 없었다.

## 돌아보기

- 대화가 길어질수록 매 호출에 보내는 `messages`가 커진다. 입력 token과 비용이 대화 길이에 따라 늘어나고, 현재 줄이는 장치는 없다.
- 입력 하나에서 `max_turns`를 다 쓰거나 API 오류가 나면, 대화 기록이 tool 결과나 user message로 끝난 상태에서 다음 입력이 붙는다. 이러한 오류와 예외는 실험을 진행하며 하나씩 고쳐 나간다.
- 대화형 실행은 record를 남기지 않아 테스트에는 `evals`를 그대로 사용한다.

### 다른 harness와 비교하면

Mini-SWE-Agent의 [`mini` CLI](https://mini-swe-agent.com/latest/usage/mini/)도 대화를 순서대로 쌓기만 한다(2026-10 확인). 다른 점은 사람이 끼어드는 위치다. 기본 모드에서는 model이 제안한 명령을 사람이 확인한 뒤에 실행하고, agent가 끝났다고 하면 새 task를 이어서 줄 수 있다. 지금의 `hel`은 tool 실행 전에 묻지 않는다. 읽기 전용 tool 하나뿐이라 아직은 필요가 없고, 이 문제는 H7 Permissions에서 다룬다.

Claude Code는 응답을 streaming으로 받아 tool 실행 중에도 진행 상황을 보여주고, 대화가 길어지면 요약해서 줄이며, 대화를 저장했다가 다시 이어 갈 수 있다. 대화 길이는 H5 Context Budget과 H6 Compaction, 저장과 재개는 H9 Sessions & Checkpoints에서 다룬다.
