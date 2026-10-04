# H4 — Repository Search

> [!NOTE]
> - 시작 상태: [`h04`](https://github.com/jammer-droid/HEL/tree/h04) · 완료 상태: [`h05`](https://github.com/jammer-droid/HEL/tree/h05)
> - 논문: [§16.3 Tool Design](https://arxiv.org/html/2609.00006v1#S16.SS3), [§13.2 The Twin Absences](https://arxiv.org/html/2609.00006v1#S13.SS2), [§16.5 Memory and Context](https://arxiv.org/html/2609.00006v1#S16.SS5)

```bash
git checkout -b my-h04 h04
```

파일명·본문 검색을 전용 tool로 제공하면, bash로 검색할 때보다 필요한 코드를 찾는 과정의 재시도와 context 사용량이 줄어드는가?

## 들어가며

### 논문이 본 코드 검색

[Part II 개요](../../parts/part2-repository-intelligence.md)에서는 저장소의 정보를 미리 주는 방법과 필요할 때 찾는 방법을 나누어 보았다. H3에서 실행 환경과 `HEL.md`를 미리 주었고, 이번에는 작업에 필요한 파일과 코드를 찾는 과정을 살펴본다.

논문의 [Table 13](https://arxiv.org/html/2609.00006v1#S13.SS2)은 shell 검색과 전용 검색 tool을 함께 다룬다. Mini-SWE-Agent는 shell에서 grep을 실행한다. Claude Code는 ripgrep과 파일 읽기를 사용하고, Gemini CLI는 ripgrep과 glob 도구를 제공한다. [Recommendation 3](https://arxiv.org/html/2609.00006v1#S16.SS3)은 bash에서 시작해 실제 문제가 관찰되면 도구를 더하라고 권한다. 검색에서는 bash와 find/rg를 함께 쓰는 것이 불편해질 때 grep/glob을 추가하는 예를 든다.

이 권고가 어떤 차이를 만드는지 확인하려면 실제 탐색 과정을 봐야 한다. model은 검색 범위를 정하고, 검색어를 고르고, 결과에서 읽을 파일을 선택한다. 전용 tool이 명령 구성을 줄여 줄 수 있지만, 잘못 고른 검색어나 불필요한 파일 읽기는 그대로 남을 수 있다.

### 이번 Lab에서 다룰 검색 tool

파일명 검색은 이름이나 경로 패턴에 맞는 파일을 찾는다. 본문 검색은 파일 안에서 문자열이나 정규식에 맞는 내용을 찾는다. 이 두 작업을 전용 tool로 제공하고, bash로 검색할 때와 필요한 코드를 찾아가는 과정을 비교한다.

두 방식에서 같은 검색 엔진을 사용할 수 있게 둔다. bash로 검색과 읽기를 한 번에 조합하는 방법도 유지한다. 확인할 것은 잘못된 검색을 고치는 호출, 필요 이상으로 반환된 내용, 필요한 코드에 도달할 때까지 사용한 context다. 검색 tool의 설명과 인자 정의가 매 요청에 더하는 token도 함께 본다.

결과를 적게 반환하면 읽을 내용이 줄지만, 필요한 항목이 잘려 다시 검색할 수도 있다. 전용 tool로 나눈 동작이 bash 명령 하나보다 많은 호출을 요구할 가능성도 있다. 그래서 작업 완료 결과와 함께 이런 복합적인 작동을 함께 고려해야 한다.

### 참고 자료

**Codex**의 [model prompt](https://github.com/openai/codex/blob/8f82b8a31cfdf690479067a179eba2c761102643/codex-rs/core/gpt_5_codex_prompt.md)는 본문에 `rg`, 파일 탐색에 `rg --files`를 우선 쓰도록 한다. 별도의 Rust [file-search](https://github.com/openai/codex/blob/8f82b8a31cfdf690479067a179eba2c761102643/codex-rs/file-search/README.md)는 파일 경로를 fuzzy matching하며, [TUI의 @파일 선택](https://github.com/openai/codex/blob/8f82b8a31cfdf690479067a179eba2c761102643/codex-rs/tui/src/file_search.rs)에서도 사용된다. 사용자 입력창의 파일 선택과 model이 수행하는 코드 검색을 구분해서 본다(확인한 commit `8f82b8a`).

**Claude Code**의 [현재 공식 문서](https://code.claude.com/docs/en/tools-reference#glob-tool-behavior)는 macOS·Linux·WSL에서 기본적으로 Bash 검색을 사용한다고 설명한다. Glob/Grep은 설정에 따라 제공되며, Grep은 파일 경로·일치 내용·개수 중 반환 형식을 선택할 수 있다. 논문의 관찰 시점과 현재 제품의 기본 도구 구성을 구분해야 한다(2026-10-04 확인).

## 이번에 해볼 것

세 가지 검색 작업을 해 본다. bash로 검색할 때와 파일명·본문 검색 tool을 더했을 때를 비교한다. 환경 정보와 `HEL.md`를 주는 동작은 H3와 같게 두고, 파일을 읽거나 여러 명령을 조합할 때는 두 방식 모두 bash를 사용할 수 있다.

| 작업 | 저장소에서 찾아야 할 것 | 확인할 차이 |
| --- | --- | --- |
| 파일명 검색 | 위치를 알려 주지 않은 `target.txt`의 내용 | 간단한 탐색에서 더해지는 tool 정의와 호출 비용 |
| 본문 검색 | 문서·테스트에도 같은 설정명이 있는 저장소에서 webhook 구현의 `RETRY_LIMIT` | 검색 범위 선택, 불필요한 결과량, 다시 검색하는 과정 |
| 단서 연결 검색 | HTTP route의 handler, handler가 가리키는 설정 파일, 그 안의 `storage_bucket` | 여러 파일의 단서를 잇는 검색·읽기 과정 |

파일명 검색에는 H1에서 쓴 파일을 그대로 사용한다. 본문 검색은 37개 파일, 단서 연결 검색은 43개 파일로 만든 작은 저장소를 쓴다. 본문 검색에는 과거 문서와 테스트용 설정값이 섞여 있고, 단서 연결 검색에는 여러 서비스의 경로와 설정이 있다. 특정 명령의 오류를 유발하는 특수한 경로나 문법은 넣지 않는다.

판정은 찾은 내용이 정답과 같은지로 한다. 새 작업의 답은 정수 하나나 짧은 문자열 하나다. 어떤 tool을 몇 번 썼는지는 정답 조건에 넣지 않는다. bash 명령 하나로 검색과 읽기를 마쳤다면 그 과정도 그대로 비교한다.

검색 결과가 없었던 경우와 명령 자체가 잘못된 경우는 실행 기록에서 구분한다. 필요한 정보를 얻기까지의 재검색과 반환된 내용을 보고, 전체 호출 수와 input/output token을 함께 확인한다. 검색 tool의 정의가 더하는 token도 총사용량에 포함된다.

## 결과 확인

### 1. 기본 실행 결과

환경 정보와 `HEL.md`를 제공하는 H3 코드로 먼저 실행했다. 아래 명령의 `baseline`은 전용 검색 tool을 추가하기 전 조건이다. 검색 엔진은 run마다 같은 ripgrep 15.2.0을 PATH에 두었다.

```bash
cargo run --quiet -p evals -- run h04 --conditions baseline --build
```

각 작업을 세 번 실행했고, 아홉 번 모두 정답을 냈다. tool 오류로 기록된 호출은 없었다. 표는 평균이며, 반환량은 model에게 전달한 tool 결과를 호출마다 한 번씩 센 UTF-8 bytes다. 같은 결과가 이후 대화에 다시 포함된 것은 중복해서 합산하지 않았다. input token에는 다시 보낸 대화 기록도 포함된다.

| 작업 | 정답 | Tool 호출 | Input / output token | 반환량 bytes |
| --- | --- | --- | --- | --- |
| 파일명 검색 | 3/3 | 2.0 | 1,823 / 252 | 199 |
| 본문 검색 | 3/3 | 3.7 | 14,984 / 382 | 16,287 |
| 단서 연결 검색 | 3/3 | 4.7 | 6,544 / 465 | 2,726 |

파일명 검색은 세 번 모두 `find`로 경로를 찾고 `cat`으로 내용을 읽었다. 한 번은 같은 명령 안에서 `xxd`로 파일 bytes까지 확인했다. 전용 검색을 더했을 때 줄일 수 있는 동작이 많지 않은 task라고 볼 수 있다.

### 2. 정답을 읽은 뒤에도 이어진 검색

본문 검색의 첫 실행은 파일 목록을 본 뒤 `src/delivery/webhook.py`를 읽었다. 그 안에 `RETRY_LIMIT = 7`이 있었다. 이후 저장소 전체를 다시 grep하고 webhook 경로도 재확인했다.

(아래 명령의 작업 디렉터리는 `/path/to/HEL`로 바꿨다.)

```bash
cat /path/to/HEL/src/delivery/webhook.py
```

이 호출이 돌려준 내용은 140 bytes였다. 뒤이어 실행한 명령은 다음과 같다.

```bash
grep -rn "RETRY_LIMIT" /path/to/HEL --include="*.py" --include="*.md" --include="*.json"
```

이 결과는 25,247 bytes였다. 과거 문서와 테스트의 값까지 포함했고, 결과마다 긴 절대 경로가 붙었다. 최종 답은 `7`로 맞았지만 전체 input은 29,366 token이었다.

다른 실행에서는 `grep -rn "RETRY_LIMIT" . 2>/dev/null | head -50`을 썼다. 잘린 결과에는 테스트와 문서만 있었고, 정답 소스는 빠졌다. model은 다음 호출에서 `src` 아래 파일들을 한꺼번에 읽은 뒤, 다시 `src`와 `docs`를 검색했다. 결과를 자르는 것만으로 탐색이 짧아지지는 않았다.

본문 검색 세 번의 반환량은 30,044 / 10,463 / 8,354 bytes, input은 29,366 / 9,462 / 6,125 token이었다. 모두 정답은 맞추었지만, 정답 파일을 이미 읽은 상태에서도 다시 저장소 전체를 검색했다. 전용 검색 tool을 제공했을 때, 이 비용을 줄일 수 있는지 확인하기 좋은 task라 할 수 있겠다.

### 3. 검색어와 파일 구조

단서 연결 검색은 route에서 handler, 설정 파일로 이어지는 경로를 모두 찾아냈다. 한 실행에서는 `POST /reports/export`라는 문자열로 검색했지만 JSON에는 method와 path가 서로 다른 필드에 있어 일치하지 않았다. 같은 bash 호출에 이어 붙인 `cat`으로 route를 읽어 다음 단계로 진행했다.

이 검색어는 전용 정규식 검색 tool에서도 그대로 일치하지 않는다. 검색 도구의 입력·출력 형식과 model이 검색어를 고르는 판단을 함께 보되, 어느 쪽에서 차이가 생겼는지 구분해야 한다.

아홉번의 실행에서 model은 제공된 `rg`를 직접 쓰지 않고 `find`와 `grep`을 골랐다. `rg`는 `bash`에서 호출하는 명령어의 개념으로 봐야하기 때문에 전용 tool을 추가했을 때의 행동 변화를 관찰하는 것이 더 유의미한 결과를 가져올 것으로 예상된다.

### 4. 전용 검색 tool 추가

`hel`에 `glob`과 `grep`을 더했다. `--tools bash,glob,grep`으로 model에게 제공한다. PATH에 있는 `rg`를 실행하므로 ripgrep이 필요하다.

| Tool | 입력 | 돌려주는 내용 |
| --- | --- | --- |
| glob | 파일명 pattern, 검색할 path(생략하면 작업 디렉터리) | 정렬된 상대 경로 목록 |
| grep | 정규식 pattern, path, 선택적 파일명 include | 상대 경로·줄 번호·일치한 줄 |

shell 명령 문자열을 조립하지 않고 각 인자를 분리해 `rg`에 전달한다. 본문 검색에서 쓰는 호출의 핵심은 다음과 같다.

```rust
command.args(["--json", "--regexp", pattern]);
if let Some(include) = include {
    command.args(["--glob", include]);
}
command.arg("--").arg(relative);
```

경로는 작업 디렉터리 안에 있는지 확인한다. 일치 결과가 없으면 정상 결과로 돌려주고, 잘못된 정규식이나 실행 실패는 오류로 돌려준다. 최대 100건·10,000 UTF-8 bytes까지 보여주며, 넘으면 범위를 좁히라는 안내를 붙인다. 한도는 model에게 보내는 응답에 적용한다. 현재 구현은 `rg`의 출력 전체를 받은 뒤 응답을 줄인다.

### 5. 검색 tool을 사용한 결과

같은 작업을 각각 세 번 더 실행했다. `variant-search`는 bash에 glob·grep을 추가한 조건이다.

```bash
cargo run --quiet -p evals -- run h04 --conditions variant-search --build
```

| 작업 | 정답 판정(추가 전 → 후) | 평균 tool 호출 | 평균 input token | 평균 반환 bytes |
| --- | --- | --- | --- | --- |
| 파일명 검색 | 3/3 → 3/3 | 2.0 → 2.0 | 1,823 → 2,776 | 199 → 89 |
| 본문 검색 | 3/3 → 3/3 | 3.7 → 3.7 | 14,984 → 7,140 | 16,287 → 6,971 |
| 단서 연결 검색 | 3/3 → 2/3 | 4.7 → 5.0 | 6,544 → 6,410 | 2,726 → 2,885 |

아홉 번 중 여덟 번에서 전용 검색 tool을 썼다. glob은 여섯 번, grep은 다섯 번 호출했다.

파일명 검색은 세 번 모두 glob으로 경로를 찾고 bash에서 cat을 실행했다. 호출 수는 두 번으로 같았다. 첫 요청 input은 459에서 830 token으로 늘었다. 반환 내용이 조금 줄어도 tool 정의를 더한 요청과 대화 기록을 보내는 비용 때문에 전체 input은 증가했다.

본문 검색은 비용이 줄었다. 세 번 중 한 번은 처음부터 `src`로 범위를 좁혔다.

```json
{"pattern":"RETRY_LIMIT","path":"src"}
```

이 grep은 1,132 bytes를 반환했고, 그 안에 다음 줄이 있었다.

```text
src/delivery/webhook.py:3:RETRY_LIMIT = 7
```

이후 glob으로 소스 목록을 확인하고 webhook.py를 읽었다. 이 실행의 전체 반환량은 1,478 bytes, input은 3,853 token이었다.

나머지 두 번은 처음에 작업 디렉터리 전체를 grep했다. 경로순으로 앞에 있는 문서에서 100건이 나와 결과가 잘렸다. 잘림 안내를 받은 뒤 `src`로 범위를 좁히거나 파일을 직접 읽었다. 전용 도구를 줘도 넓게 검색하거나 정답을 본 뒤 추가 확인하는 동작은 그대로 존재했다.

본문 검색의 반환량 중앙값은 10,463에서 8,986 bytes로 14.1%, input 중앙값은 9,462에서 8,555 token으로 9.6% 줄었다. 호출 수는 두 조건 모두 4 / 3 / 4번이었다. 전용 tool을 추가하면 model은 전용 tool을 더 우선적으로 선택하는 경향을 여기에서도 확인할 수 있었다.

- 실패한 실행은 올바른 값을 찾고도 설명과 code block을 붙여 형식이 맞지 않아 실패했다. 이 실행에서는 전용 검색 tool을 사용하지 않았다.

## 돌아보기

### 변경 사항

전용 검색 tool을 제공한 아홉 번 중 여덟 번에서 model이 이를 사용했다. 파일명 검색은 세 번 모두 glob을, 본문 검색은 세 번 모두 grep을 선택했다. 이번 task와 model에서는 검색을 전용 tool에 맡기는 성향을 확인할 수 있었다.

이 선택을 바탕으로 `glob`과 `grep`을 유지한다. model은 검색할 pattern과 path를 넘기고, harness가 명령 인자를 구성하고 결과를 정리한다. 경로 검사, 일치 없음과 오류의 구분, 응답 한도는 코드와 결정적 test로 확인할 수 있는 동작이 됐다. 사용하려면 `--tools bash,glob,grep`을 지정한다.

harness는 model이 요청한 작업을 실행하고 다음 단계로 이어 주는 역할을 한다. 전용 tool은 여기에 더해, model이 매번 명령으로 구성하던 동작의 일부를 harness가 통제할 수 있는 영역으로 옮긴다. model에게 남은 검색어와 범위의 선택을 보면서도, 선택한 작업을 어떤 규칙으로 실행하고 결과를 돌려줄지는 정할 수 있다.

### 트레이드오프

같은 task에서도 model의 탐색 순서와 최종 표현은 달라졌다. 한 번은 올바른 값을 찾고도 설명과 code block을 붙였다. 이 양식 불일치는 실행 기록에 남기되, 전용 검색 tool의 기능적 실패와 구분해서 보았다.

총 token도 매 실행에서 일정하게 만들 수 있는 값은 아니다. 본문 검색에서는 줄었고, 파일명 검색에서는 tool 정의를 더한 비용으로 늘었다. 그러나 이번 실험에서 확인한 수치만으로 비용의 증감이 항상 이런 경향으로 진행된다고 판단할 수는 없다.

harness가 직접 정한 것은 각 tool의 인자와 응답 형식, 반환 한도다. 여러 호출에 걸쳐 쌓이는 context는 [FAQ](faq.md#응답에-한도를-두면-전체-token도-고정되는가)에서 이어서 본다.

출력 한도 때문에 필요한 결과가 잘려 다시 검색하는 대가도 있다. 실제로 두 번은 넓게 검색한 결과가 잘린 뒤 `src`로 범위를 좁혔다. 현재 구현은 rg 출력을 모두 받은 뒤 반환량을 제한하므로, 이 한도가 검색 과정의 메모리나 실행 시간까지 제한하지는 않는다.

### 논문의 내용 또는 다른 harness와 비교하면

논문의 [Recommendation 3](https://arxiv.org/html/2609.00006v1#S16.SS3)은 bash로 시작해 관찰한 문제에 맞춰 도구를 더하라고 권한다. 이번에는 넓은 검색 결과와 추가 확인을 보고 전용 검색을 더했다. 이 경로를 model이 실제로 사용하면서, 검색 동작의 일부를 harness가 정한 규칙으로 처리할 수 있었다.

Codex의 `rg`·`rg --files` 사용 지침과 Claude Code의 플랫폼별 Bash·Grep·Glob 구성은 검색 기능을 model에게 제공하는 방식이 다양함을 보여준다.
