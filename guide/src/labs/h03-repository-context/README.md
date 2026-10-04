# H3 — Repository Context

> [!NOTE]
> - 시작 상태: [`h03`](https://github.com/jammer-droid/HEL/tree/h03) · 완료 상태: [`h04`](https://github.com/jammer-droid/HEL/tree/h04)
> - 논문: [§9.7 Repository Context](https://arxiv.org/html/2609.00006v1#S9.SS7), [§16.5 Memory and Context](https://arxiv.org/html/2609.00006v1#S16.SS5), [§7.2 Prompt Engineering Architectures](https://arxiv.org/html/2609.00006v1#S7.SS2)

```bash
git checkout -b my-h03 h03
```

실행 환경 정보를 system prompt로 주고, 작업 디렉터리의 `HEL.md`를 읽어 넣으면, model이 환경과 저장소 규칙을 알아내는 데 드는 호출과 실패는 어떻게 달라질까?

## 들어가며

[Part II 개요](../../parts/part2-repository-intelligence.md)에서 본 것처럼 지금의 `hel`은 system prompt 없이 사용자 지시와 tool 정의만 model에게 보낸다. 이번 Lab에서는 model이 저장소와 실행 환경을 미리 알도록 harness가 정보를 넣어 주는 방법을 다룬다.

### 논문이 본 repository context

논문 §9.7은 대화 기록과 별개로 각 시스템이 저장소 정보를 어떻게 model에게 주는지 정리한다. 대부분은 사람이 쓴 Markdown 파일을 harness가 찾아 prompt에 넣는다.

- Codex는 project root에서 작업 디렉터리까지 내려오며 찾은 `AGENTS.md`를 모두 이어 붙인다.
- Claude Code는 조직·사용자·프로젝트 범위의 `CLAUDE.md`를 찾아 넣고, 하위 디렉터리의 파일은 그 디렉터리의 파일을 읽을 때 넣는다.
- Aider는 tree-sitter로 symbol 지도를 만들어 대화와 관련된 순서로 넣는다. 이런 지도를 쓰는 시스템은 Aider 하나다.

§16.5의 Recommendation 6은 이 방식 사용을 권장한다. 프로젝트·사용자 범위의 Markdown context 파일을 계층적으로 자동 탐색하고, 다른 도구가 쓰는 파일 이름도 읽고, 최상위 내용은 system prompt 위쪽에 넣으라고 한다. 논문의 최소 harness 예시(§16.10)는 작업 디렉터리에서 루트까지 `AGENTS.md`를 찾아 이어 붙이는 함수 하나로 이를 구현한다.

실행 환경 정보는 §7.2의 prompt 구성에서 다룬다. OpenCode와 Pi는 작업 디렉터리와 날짜를, Mistral Vibe는 `git log`와 `git status` 출력을 system prompt에 넣는다. Mini-SWE-Agent는 `platform.uname()`으로 얻은 OS 정보를 넣고, macOS에서는 `sed -i ''`를 쓰라는 안내를 덧붙인다.

### 이번 Lab에서 다룰 repository context

`hel`에는 다음 두 가지를 추가할 예정이다.

- **system prompt**: harness가 시작할 때 OS, shell, 작업 디렉터리를 알아내 system message로 보낸다. "macOS에서는 이렇게 하라" 같은 지시는 넣지 않는다. model이 환경을 안 상태에서 명령을 스스로 맞춰 고르는지 보기 위해서다.
- **`HEL.md` 읽기**: 작업 디렉터리에 `HEL.md`가 있으면 그 내용을 system prompt에 붙인다. 파일 이름은 다른 도구가 쓰는 `AGENTS.md`, `CLAUDE.md`와 겹치지 않게 정했다. 이미 `AGENTS.md`를 쓰는 저장소에서 `hel`을 실행해도 다른 도구용 규칙이 섞이지 않는다. 상위 디렉터리로 올라가며 찾지 않고 작업 디렉터리 하나만 본다.

`HEL.md`에 쓴 규칙은 model에게 주는 정보일 뿐이고, harness는 model이 그 규칙을 지키는지 검사하지 않는다. Claude Code 문서도 `CLAUDE.md`를 강제되는 설정으로 보지 말고, 반드시 막아야 하는 동작은 hook으로 처리하라고 안내한다. 규칙을 강제하는 방법은 H7 Permissions와 H10 Skills / Hooks / MCP에서 다룬다.

### 참고 자료

- [Mini-SWE-Agent `mini.yaml`](https://github.com/SWE-agent/mini-swe-agent/blob/04d809ceab9df28f9adaed044884180159172930/src/minisweagent/config/mini.yaml): 첫 user message의 `<system_information>`에 OS 정보를 넣고, macOS면 `sed` 안내를 더한다. context 파일은 읽지 않는다.
- [Codex `agents_md.rs`](https://github.com/openai/codex/blob/8f82b8a31cfdf690479067a179eba2c761102643/codex-rs/core/src/agents_md.rs): project root(기본 `.git`이 있는 곳)에서 작업 디렉터리까지의 `AGENTS.md`를 이어 붙이고, 합계 32KiB에서 자른다. root를 못 찾으면 작업 디렉터리만 본다.
- [Claude Code 메모리 문서](https://code.claude.com/docs/en/memory): `CLAUDE.md`의 범위와 로드 순서, `AGENTS.md`를 읽는 조건, 파일당 200줄 이하 권장(2026-10 기준).

## 이번에 해볼 것

출발 상태는 `h03`의 `hel`이다. tool은 기본값 `bash` 하나이고(`--tools`로 `read_file`, `write_file`, `search_replace`를 고를 수 있다), model에게는 사용자 지시와 tool 정의만 보낸다.

여기에 두 가지 실행 옵션을 만든다. 옵션을 주지 않으면 지금과 같이 동작한다.

- `--env`: OS, shell, 작업 디렉터리를 알아내 system message로 보낸다.
- `--context-file HEL.md`: 작업 디렉터리에 `HEL.md`가 있으면 그 내용을 system message에 붙인다.

같은 task를 세 가지 구성으로 실행해 비교한다. 환경 정보를 먼저 만들어 확인하고, 그다음 `HEL.md` 읽기를 더한다.

| 구성 | 옵션 | 확인할 것 |
| --- | --- | --- |
| 시작 상태 | 없음 | 환경과 규칙을 model이 스스로 알아낼 때의 호출과 실패 |
| 환경 정보 | `--env` | 실패한 명령이 줄어드는가, system message만큼 비용이 얼마나 느는가 |
| 환경 정보 + `HEL.md` | `--env --context-file HEL.md` | 파일에만 적힌 규칙을 지키는가, 규칙을 찾는 호출이 줄어드는가 |

task는 세 개다. 각 구성에서 task마다 3번씩 실행하고, 실행마다 model 호출은 10번까지 허용한다.

| task | 지시 | 확인할 것 |
| --- | --- | --- |
| `edit-line-01` | H2에서 쓴 task. 237줄짜리 `settings.py`에서 값 하나를 바꿈 | system message가 더하는 비용. 정확성은 그대로인지 |
| `env-checksum-01` | `data.txt`의 MD5 값을 `checksum.txt`에 씀 | 실행 환경에 없는 명령의 실패와, 환경을 모를 때의 대비 명령. OS와 PATH에 따라 `md5sum`, `md5`, `openssl`, python 중 쓸 수 있는 것이 다르다 |
| `rule-config-01` | "cache_ttl을 300으로 설정하라". `config/default.ini`에 현재 값이 있고 `config/local.ini`도 있다 | 작업 디렉터리의 `HEL.md`에만 적힌 규칙("default.ini는 고치지 않고 local.ini에 쓴다")을 지키는가 |

`rule-config-01`의 `HEL.md`는 세 구성 모두 작업 디렉터리에 들어 있다. 달라지는 것은 harness가 그 내용을 미리 넣어 주는지다. 옵션이 없으면 model이 `ls`로 파일을 보고 직접 읽어야 한다.

채점은 실행이 끝난 뒤의 파일로 한다. `edit-line-01`과 `env-checksum-01`은 파일이 기대한 내용과 정확히 같은지 본다. `rule-config-01`은 `default.ini`가 그대로인지와, `local.ini`의 `[cache]` 섹션에 `cache_ttl = 300`이 있는지 본다. 새 키를 쓸 때 `=` 주변 공백이나 빈 줄은 model마다 다르게 쓰므로 비교하지 않는다. 실패한 bash 명령 수와 token은 실행 기록으로 확인한다.

## 결과 확인

### 1. 시작 상태

옵션 없이 `h03`의 `hel`로 세 task를 3번씩 실행했다.

```bash
evals run h03 --conditions baseline --build
```

`baseline`, `variant-env`, `variant-env-hel`은 Lab 정의(`evals/labs/h03.yaml`)에 적은 세 구성의 이름이다. 각각 시작 상태, 환경 정보, 환경 정보 + `HEL.md`에 해당한다.

(처음 측정에서는 `evals`가 `hel`을 `PATH=/usr/bin:/bin`으로 실행해서, macOS의 `/sbin`에 있는 `md5`와 `md5sum`을 찾지 못했다. 실제 사용 환경과 같도록 PATH에 `/usr/sbin:/sbin`을 더하고 다시 측정한 결과를 적었다.)

| task | 통과 | 평균 tool 호출 | 평균 input / output token | 실패한 bash 호출 |
| --- | --- | --- | --- | --- |
| `edit-line-01` | 3/3 | 5.7 | 5907 / 1032 | 2 |
| `env-checksum-01` | 3/3 | 4.3 | 3906 / 843 | 0 |
| `rule-config-01` | 3/3 | 5.7 | 5049 / 731 | 2 |

모두 통과했지만 model은 자신이 어떤 OS에서 실행되는지 모르는 채로 시작했다. 실행 기록에서 이렇게 드러났다.

- `env-checksum-01`은 세 실행 모두 값을 구하기 전에 명령이 있는지부터 확인했다.

```text
#1  command -v md5sum; command -v md5; md5 -q data.txt 2>/dev/null; md5sum data.txt 2>/dev/null
#2  md5sum data.txt 2>&1 || echo "md5sum failed/absent"
#3  which md5sum md5 openssl python3; echo $SHELL
```

- 두 실행은 reasoning에서 OS를 추측했다. #1은 `ls -la` 출력의 group 이름을 보고 "The environment seems macOS (staff group)"라고 판단했고, #3은 "macOS-ish ... GNU md5sum at /sbin/md5sum (busybox?)"처럼 OS를 정하지 못했다.
- `edit-line-01`과 `rule-config-01`에서는 Linux 기준 명령이 실패했다. macOS의 `cat`에는 `-A` 옵션이 없어 세 번 실패했고, `sed -i '35s/.../'`는 macOS `sed`가 `-i` 뒤의 값을 백업 확장자로 읽어 실패했다. model은 오류를 보고 "this is BSD sed"라고 판단한 뒤 python으로 다시 고쳤다.
- `rule-config-01`은 세 실행 모두 `ls`나 `grep`으로 파일을 훑다가 `HEL.md`를 발견하고 스스로 읽었다. 규칙은 모두 지켰다.

### 2. 환경 정보 추가

`--env`를 주면 harness가 시작할 때 다음 system message를 만든다.

```rust
pub fn environment(workdir: &Path) -> String {
    let kernel = output("uname", &["-sr"]).unwrap_or_else(|| "unknown".to_string());
    let shell = output("bash", &["-c", "echo $BASH_VERSION"])
        .map(|version| format!("bash {version}"))
        .unwrap_or_else(|| "bash (version unknown)".to_string());
    format!(
        "Environment:\n- OS: {} ({kernel}, {})\n- Shell: {shell}\n- Working directory: {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        workdir.display()
    )
}
```

실제로 보낸 내용은 이렇다(작업 디렉터리 경로는 `/path/to/tmp`로 바꿔 적었다).

```text
Environment:
- OS: macos (Darwin 24.1.0, aarch64)
- Shell: bash 3.2.57(1)-release
- Working directory: /path/to/tmp/hel-lab/<run-id>
```

```bash
evals run h03 --conditions variant-env --build
```

| task | 통과 | 평균 tool 호출 | 평균 input / output token | 실패한 bash 호출 |
| --- | --- | --- | --- | --- |
| `edit-line-01` | 3/3 | 3.0 (시작 상태 5.7) | 8475 / 713 | 0 (2) |
| `env-checksum-01` | 3/3 | 3.0 (4.3) | 3110 / 660 | 0 (0) |
| `rule-config-01` | 3/3 | 4.7 (5.7) | 4515 / 596 | 0 (2) |

`env-checksum-01`의 명령은 이렇게 바뀌었다.

```text
#1  ls -la && md5 -q data.txt 2>/dev/null || md5sum data.txt
#2  md5 -q data.txt 2>/dev/null || md5sum data.txt
#3  ls -la; cat data.txt; md5 -q data.txt 2>/dev/null || md5sum data.txt
```

- 명령이 있는지 확인하는 호출(`which`, `command -v`)이 사라졌고, 세 실행 모두 macOS의 `md5 -q`를 먼저 썼다. reasoning에서 OS를 추측하는 문장도 없어졌다.
- `|| md5sum`을 붙이는 fallback은 남았다. 이 형태는 환경 정보가 없던 처음 측정에서도 세 실행 중 두 번 나왔다. 환경 정보와 상관없는 model의 습관으로 보이고, 같은 명령 안에 붙어서 tool 호출을 늘리지 않는다.
- `edit-line-01`은 세 실행 중 두 번 처음부터 macOS 형식의 `sed -i ''`를 썼고 실패하지 않았다. `cat -A`도 더 이상 쓰지 않았다.
- `rule-config-01`은 여전히 세 실행 모두 `HEL.md`를 직접 찾아 읽었다. 환경 정보에는 저장소 규칙이 없으니 예상한 결과다.
- system message는 첫 호출 input을 364에서 454 token으로 약 90 token 늘렸다. `edit-line-01`의 평균 input이 늘어난 것은 호출이 줄었는데도 세 실행 모두 첫 호출에서 237줄 파일 전체를 `cat`했기 때문이다. 실행마다 차이가 커서 지금 규모로는 의미 있는 차이로 보기 어렵다.

### 3. `HEL.md` 추가

`--context-file HEL.md`를 주면 작업 디렉터리의 `HEL.md`를 읽어 환경 정보 뒤에 붙인다. 작업 디렉터리 밖은 찾지 않고, 파일이 없으면 아무것도 더하지 않는다.

```rust
if let Some(name) = context_file
    && let Ok(text) = fs::read_to_string(workdir.join(name))
    && !text.trim().is_empty()
{
    parts.push(format!(
        "Contents of {name} in the working directory:\n\n{}",
        text.trim_end()
    ));
}
```

```bash
evals run h03 --conditions variant-env-hel --build
```

| task | 통과 | 평균 tool 호출 | 평균 input / output token | 실패한 bash 호출 |
| --- | --- | --- | --- | --- |
| `edit-line-01` | 3/3 | 3.0 (환경 정보 3.0, 시작 상태 5.7) | 7797 / 647 (8475 / 713, 5907 / 1032) | 1 |
| `env-checksum-01` | 3/3 | 3.0 (3.0, 4.3) | 2727 / 500 (3110 / 660, 3906 / 843) | 0 |
| `rule-config-01` | 3/3 | 3.7 (4.7, 5.7) | 4481 / 1134 (4515 / 596, 5049 / 731) | 0 |

(`edit-line-01`의 실패 1은 `diff`가 차이를 찾아 exit 1을 돌려준 것이다. 실행 기록은 exit code만 보므로 실패로 남았다.)

`rule-config-01`을 더 자세히 보면 이렇다.

| `rule-config-01` | 시작 상태 | 환경 정보 | 환경 정보 + `HEL.md` |
| --- | --- | --- | --- |
| 실행별 tool 호출 | 6, 7, 4 | 4, 4, 6 | 2, 3, 6 |
| `HEL.md`를 직접 읽은 실행 | 3/3 | 3/3 | 1/3 |
| 첫 호출 input token | 346 | 434 | 492 |

```text
#1  ls -la; cat config/default.ini; cat config/local.ini
    config/local.ini에 [cache] 섹션과 cache_ttl = 300을 추가
#2  ls -la && cat HEL.md && cat config/default.ini && ...
    grep -rn "cache_ttl" .
    config/local.ini에 [cache] 섹션과 cache_ttl = 300을 추가
#3  ls -la; ls -la config
    cat config/default.ini; cat config/local.ini
    local.ini 백업 후 [cache] 섹션 추가, diff로 확인, 백업 삭제 (3번)
```

- 두 실행은 `HEL.md`를 찾지 않고 바로 설정 파일을 열었다. 한 실행(#2)은 내용을 이미 받았는데도 첫 명령에 `cat HEL.md`를 넣었다.
- 세 실행 모두 규칙을 지켰고, 최종 답에 "per the project rules", "Per HEL.md"처럼 규칙을 근거로 적었다.
- #3은 `HEL.md`를 찾지 않았지만 백업을 만들고 diff로 확인한 뒤 백업을 지우느라 호출이 6번이 됐다. 호출 수에는 파일을 찾는 비용과 결과를 확인하는 습관이 함께 들어 있다.
- `HEL.md`(4줄)는 첫 호출 input을 약 58 token 늘렸다. 평균 input은 호출이 줄어 환경 정보만 있을 때와 비슷했다(4515 → 4481). 평균 output이 1134로 늘어난 것은 #3 한 실행이 2374 token을 쓴 영향이다.

`edit-line-01`과 `env-checksum-01`에는 `HEL.md`가 없어서 이 구성의 system message는 환경 정보만 있을 때와 같다. 같은 내용으로 다시 실행한 셈인데, `env-checksum-01`의 `|| md5sum` fallback은 세 실행 중 세 번에서 한 번으로 줄었다. 실행마다 나타났다 말았다 하는 습관이다.

## 돌아보기

### 변경 사항

- model이 환경과 규칙을 알아내려고 harness와 주고받던 요청-응답이 사라졌다. 명령이 있는지 확인하는 호출(`which`, `command -v`), OS에 맞지 않아 실패한 뒤 다시 시도한 호출(`cat -A`, `sed -i`), reasoning에서 OS를 추측하는 문장이 환경 정보를 준 뒤 모두 없어졌다.
- `HEL.md`를 넣어 주자 규칙 파일을 찾아 읽은 실행이 세 번에서 한 번으로 줄었다.
- 결과의 정확성은 그대로였다. 세 구성 모두 테스트를 통과했다. 규칙 task도 시작 상태에서 model이 `HEL.md`를 스스로 찾아 지켰다. 이번에 달라진 것은 결과보다 그 결과에 이르기까지의 탐색이다.
- 이제 `hel`은 실행 환경 정보와 작업 디렉터리의 `HEL.md`를 기본으로 보낸다. 보내지 않으려면 `--no-env`, `--no-context-file`을 준다.

### 트레이드오프

- system message는 매 호출에 붙는다. 환경 정보는 약 90 token, 4줄짜리 `HEL.md`는 약 58 token이었다. `HEL.md`가 길어질수록 모든 호출의 input이 그만큼 늘어난다. Claude Code가 파일당 200줄 이하를 권하는 이유이기도 하다.
- 평균 tool 호출과 token은 줄어든 task도 있고 늘어난 task도 있었다. 파일 전체를 `cat`할지, 결과를 백업하고 diff로 다시 확인할지 같은 model의 작업 습관이 실행마다 달라서, 세 번 실행으로는 정보를 넣은 효과와 구분하기 어렵다. 이번에는 이 값을 참고로만 두었다.
- `HEL.md`는 작업 디렉터리에서만 읽는다. 하위 디렉터리에서 `hel`을 실행하면 저장소 루트의 `HEL.md`를 받지 못한다. 상위 디렉터리를 찾는 방식은 [FAQ](faq.md)에 남겼다.
- 넣어 준 규칙은 model에게 주는 정보일 뿐이다. 이번에는 세 번 모두 지켰지만 harness가 막지는 않는다. 규칙을 강제하는 방법은 H7 Permissions와 H10 Skills / Hooks / MCP에서 다룬다.

### 논문의 내용 또는 다른 harness와 비교하면

논문 §16.5의 Recommendation 6은 context 파일을 자동으로 찾아 prompt 위쪽에 넣으라고 권한다. 이번 task에서는 그 효과가 정확성보다 탐색에서 드러났다. 작은 작업 디렉터리에서는 model이 규칙 파일을 스스로 찾았고, 미리 넣어 주자 그 탐색이 사라졌다.

Codex는 `.git`이 있는 project root에서 작업 디렉터리까지 내려오며 `AGENTS.md`를 모두 이어 붙이고, 합계 32KiB에서 자른다. 내용은 user message로 보낸다. 환경 정보도 `<environment_context>`라는 user message로 보내고, 작업 디렉터리, shell, 날짜, 시간대를 넣는다. `hel`은 둘 다 system message 하나에 넣고, 작업 디렉터리 하나만 본다.

Claude Code는 `CLAUDE.md`를 조직, 사용자, 프로젝트 범위로 나눠 찾고, 하위 디렉터리의 파일은 그 디렉터리의 파일을 읽을 때 넣는다. `hel`은 범위를 나누지 않고 하위 디렉터리 파일도 읽지 않는다. 이번 task처럼 디렉터리 하나짜리 작업에서는 이런 구분이 필요 없었다.

Mini-SWE-Agent는 OS 정보와 함께 macOS에서는 `sed -i ''`를 쓰라는 안내를 넣는다. `hel`은 안내 없이 OS 이름과 버전만 주었는데, model은 세 번 중 두 번 처음부터 `sed -i ''`를 썼다.
