# H2 — Editing

> [!NOTE]
> - 시작 상태: [`h02`](https://github.com/jammer-droid/HEL/tree/h02) · 완료 상태: [`h03`](https://github.com/jammer-droid/HEL/tree/h03)
> - 논문: [§8.4 File Editing Strategies](https://arxiv.org/html/2609.00006v1#S8.SS4), [§16.4 File Editing](https://arxiv.org/html/2609.00006v1#S16.SS4)

```bash
git checkout -b my-h02 h02
```

bash만으로 파일을 고칠 때와 전용 편집 tool을 줄 때, 비용과 작업 범위는 어떻게 달라질까?

## 들어가며

### 논문이 본 편집 방식

[Harness Engineering 논문](https://arxiv.org/html/2609.00006v1)의 §8.4는 파일 편집을 SWE agent의 핵심 action으로 보고, 11개 harness가 8가지 방식으로 나뉜다고 정리했다. 논문은 편집 방식이 코드 수정 정확도를 가장 크게 좌우하는 요인 중 하나라고 본다.

| Harness | 편집 방식 | 바꾸지 못했을 때 |
| --- | --- | --- |
| Mini-SWE-Agent | bash의 `sed`, `awk` | shell 오류 |
| Claude Code | 정확한 문자열 치환, 파일 안에서 한 번만 나와야 함 | 오류와 맥락 |
| Mistral Vibe | 정확한 문자열 치환(전부 바꾸기 선택 가능), 새 파일 전용 `write_file` | 여러 곳 일치 오류 |
| OpenCode | model에 따라 patch 형식이나 문자열 치환. 9단계로 점점 느슨하게 맞춰 봄 | 일치 없음과 여러 곳 일치를 구분 |
| Hermes | 9단계로 느슨하게 맞춰 봄 | 가장 가까운 줄 제안, 3번 실패하면 `write_file`로 넘어감 |
| Gemini CLI | 문자열 치환, 정확히 맞지 않으면 공백·정규식·유사도 순으로 맞춰 봄 | LLM을 한 번 더 불러 치환 문자열을 고침 |

- 바꿀 부분을 정확하게 찾는 쪽과 느슨하게 찾는 쪽으로 갈린다. Mistral Vibe는 2026년 4월에서 7월 사이에 느슨하게 맞추는 SEARCH/REPLACE tool을 지우고 Claude Code처럼 정확히 맞추는 방식으로 옮겼다. 논문은 model이 강해질수록 tool이 오차를 흡수하기보다 엄격하게 맞추는 편이 낫다는 증거로 해석한다.
- §16.4는 model 수준에 따라 고르라고 권한다. 최신 상용 model에는 정확한 문자열 치환, 공개 model이나 약한 model에는 느슨한 매칭을 쓴다. 줄 번호로 위치를 지정하는 편집은 피하라고 한다. model은 주변 내용으로 위치를 맞출 때보다 줄 번호를 쓸 때 더 자주 어긋난다는 이유다.
- §16.3은 tool을 더하는 순서로 "bash → `read_file`, `write_file` → 파일 전체를 다시 쓰느라 token이 낭비되면 `search_replace`"를 든다. §16.10의 최소 harness 예시에도 `write_file`과 `search_replace`가 있다. `search_replace`는 바꿀 문자열이 한 번만 나오지 않으면 몇 번 나왔는지 알려 주고 바꾸지 않는다.

### 이번 Lab에서 다룰 편집 방법

§16.3에서 나온 순서대로 bash, `write_file`, `search_replace` 세 단계를 `hel`에서 따라가 본다.

H1의 `hel`에는 `read_file`과 `bash`가 있다. 파일을 고치는 전용 tool은 없어서, 지금 model이 파일을 고치려면 bash 명령을 써야 한다.

- **bash로 고치기**: `sed -i 's/old/new/' file`처럼 명령으로 바꾸거나, `cat <<'EOF' > file`로 파일 전체를 다시 쓴다. harness는 명령 문자열만 받으므로 어느 파일의 어디를 바꾸려는지 모른다. 명령이 아무것도 바꾸지 못해도 `sed`는 종료 코드 0을 돌려준다.
- **전체 쓰기(`write_file`)**: model이 경로와 파일의 새 내용 전체를 넘기면 harness가 그대로 저장한다. 한 줄만 바꿔도 파일 전체를 출력해야 하고, 옮겨 적다가 다른 줄이 바뀌어도 harness는 알 수 없다.
- **문자열 치환(`search_replace`)**: model이 바꿀 부분(`search`)과 새 내용(`replace`)을 넘긴다. harness는 `search`가 파일에 정확히 한 번 나올 때만 바꾸고, 없거나 여러 번 나오면 바꾸지 않고 오류를 돌려준다.

| | bash | `write_file` | `search_replace` |
| --- | --- | --- | --- |
| model이 넘기는 것 | 명령 문자열 | 경로, 파일 전체 내용 | 경로, 바꿀 문자열, 새 문자열 |
| 출력해야 하는 양 | 명령 길이 | 파일 크기 | 바뀌는 부분과 그 주변 |
| harness가 아는 것 | 문자열뿐 | 대상 파일 | 대상 파일과 바뀌는 위치 |
| 바꾸지 못했을 때 | 명령에 따라 다름. 오류 없이 끝나기도 함 | 해당 없음(항상 덮어씀) | 일치 없음, 여러 곳 일치 오류 |

bash만 있는 `hel`에 `write_file`을 더하고, 이어서 `search_replace`를 더하면서 단계마다 model이 파일을 어떻게 고치는지, token을 얼마나 쓰는지, 작업 디렉터리 밖에 접근하는지 본다.

### 참고 자료

- [Mini-SWE-Agent](https://github.com/SWE-agent/mini-swe-agent/tree/04d809ceab9df28f9adaed044884180159172930)에는 편집 전용 코드가 없다. 시스템 prompt에 편집 명령 예시를 넣어 가르친다. (2026-10 기준.)
  - 새 파일은 `cat <<'EOF' > newfile.py`, 고치기는 `sed -i 's/old/new/g'`과 줄 번호·줄 범위를 지정한 `sed` 예시, 보기는 `nl -ba file | sed -n '10,20p'`.
  - macOS에서는 `sed -i ''`를 써야 한다는 안내를 prompt에 넣는다. macOS의 `sed`는 Linux의 `sed`와 `-i` 옵션 사용법이 다르다.
- Claude Code([Tools reference](https://code.claude.com/docs/en/tools-reference), 2026-10 기준)는 부분 수정에 Edit, 새로 만들거나 전체를 다시 쓸 때 Write를 쓴다.
  - Edit는 `old_string`을 `new_string`으로 바꾼다. 정규식이나 유사도 매칭은 쓰지 않는다. 공백이나 들여쓰기가 한 글자만 달라도 실패한다.
  - `old_string`이 여러 번 나오면 주변 내용을 더 넣어 한 곳으로 좁히거나 `replace_all`로 전부 바꾼다.
  - 이번 대화에서 읽지 않은 파일은 고치지 못하게 한다(model에 따라 조건부로 완화). 읽은 뒤 파일이 바뀌었으면 현재 내용에 정확히 맞을 때만 적용하고 다른 변경이 있다고 알린다.

## 이번에 해볼 것

출발 상태는 `h02`의 `hel`이다. 기본 tool은 `bash` 하나이고, `--tools`로 `bash`와 `read_file` 중에서 고를 수 있다.

여기에 `write_file`과 `search_replace` tool을 만들고 `--tools`로 고를 수 있게 한다. 같은 task를 세 가지 구성으로 실행해 비교한다. `read_file`은 세 구성 모두 주지 않는다. 파일은 bash의 `cat` 등으로 읽는다.

| 구성 | 주는 tool | 확인할 것 |
| --- | --- | --- |
| 시작 상태 | `bash` | bash 명령만으로 파일을 고칠 때의 비용과 작업 범위 |
| 전체 쓰기 추가 | `bash`, `write_file` | `write_file`을 고르는가, 고르면 비용이 얼마나 느는가 |
| 문자열 치환 추가 | `bash`, `write_file`, `search_replace` | `search_replace`를 고르는가, 비용이 줄어드는가 |

task는 두 개다. 각 구성에서 task마다 3번씩 실행한다. 실행마다 model 호출은 10번까지 허용한다.

| task | 지시 | 확인할 것 |
| --- | --- | --- |
| `edit-line-01` | 237줄짜리 `settings.py`에서 `REQUEST_TIMEOUT_SECONDS`를 30에서 60으로 바꾸고 나머지는 그대로 둠 | 긴 파일 한 줄 수정의 비용. 값 `30`이 다른 줄에도 있어 값만 바꾸면 다른 줄이 바뀜 |
| `edit-ambiguous-01` | `services.ini`의 `[database]` 섹션에서만 `retries`를 3에서 5로 바꿈 | 같은 줄 `retries = 3`이 다섯 섹션에 있을 때 의도한 곳만 바꾸는가 |

채점은 실행이 끝난 뒤의 파일이 기대한 내용과 정확히 같은지만 본다. 끝 줄바꿈 한 개의 차이만 무시하므로, 다른 줄이 하나라도 바뀌면 실패다. model의 최종 답변은 채점하지 않고, 어떤 tool을 어떤 명령으로 썼는지와 token은 실행 기록으로 확인한다.

## 결과 확인

### 시작 상태로 실행

먼저 `h02`의 `hel`(tool은 `bash` 하나)로 두 task를 3번씩 실행했다. `evals`에서는 이 구성을 `baseline` 조건이라고 부른다.

```bash
evals run h02 --conditions baseline
```

| task | 통과 | input / output token (평균) | model 호출 (평균) | tool 호출 (평균) |
| --- | --- | --- | --- | --- |
| `edit-line-01` | 3/3 | 6110 / 1028 | 5 | `bash` 3.7 |
| `edit-ambiguous-01` | 3/3 | 7806 / 1376 | 5 | `bash` 4.7 |

(`hel` `091ee2c`, model `deepseek-flash`, 최대 model 호출 10번, macOS.)

bash만으로도 여섯 번 모두 파일을 정확히 고쳤다. 실행마다 편집에 쓴 명령은 이렇다.

| 실행 | `edit-line-01` | `edit-ambiguous-01` |
| --- | --- | --- |
| 1 | bash 안에서 python으로 `^REQUEST_TIMEOUT_SECONDS = 30$`를 정규식 치환, 바뀐 곳이 1개인지 `assert` | python으로 줄을 돌면서 현재 섹션이 `[database]`일 때만 바꿈 |
| 2 | `sed -i '35s/.../.../'` 실패 → python으로 다시 고침 | python으로 섹션을 따라가며 바꿈 |
| 3 | python으로 `s.count(old) == 1`을 확인한 뒤 `replace` | `awk`로 섹션을 따라가며 바꿈 |

`edit-line-01`의 두 번째 실행에서 쓴 `sed`는 이렇게 끝났다.

```text
exit=0
REQUEST_TIMEOUT_SECONDS = 30
--- diff ---
sed: 1: "settings.py": unterminated substitute pattern
```

- macOS의 `sed`는 `-i` 다음 인자를 백업 파일 확장자로 읽는다. 그래서 `'35s/.../'`를 확장자로, `settings.py`를 편집 명령으로 해석해 실패했다. Linux의 `sed`라면 성공했을 명령이다.
- 같은 명령 문자열 안의 마지막 명령(`echo`)이 성공해서 첫 줄은 `exit=0`이다. model은 바로 뒤에 출력한 35번째 줄이 그대로인 것을 보고 실패를 알아챘고, python으로 다시 고쳤다.
- 여섯 번 중 다섯 번은 처음부터 `sed`를 쓰지 않았다. 최종 편집은 다섯 번이 python, 한 번이 `awk`였다. python으로 고친 다섯 번은 모두 바뀐 곳이 정확히 하나가 아니면 `assert`로 멈추게 했다. bash 안에서 `search_replace`와 같은 검사를 스스로 만들어 쓴 셈이다.
- 고친 뒤에는 모두 `diff`, `grep`, `sed -n`으로 결과를 다시 확인했다. tool 호출이 2~6번으로 실행마다 달랐던 것은 이 확인 단계의 길이 차이다.

편집 결과와 별개로, bash로 고칠 때의 작업 범위 문제가 보였다.

- 여섯 번 중 네 번은 원본 백업을 작업 디렉터리 밖의 `/tmp`에 만들었다.
- `edit-ambiguous-01`의 두 번째 실행은 파일을 이미 읽은 뒤에 `find / -name "services.ini"`로 디스크 전체를 검색했다. 이 명령에만 약 100초가 걸렸고(제한은 120초), 결과에는 이전 실행들이 남긴 작업 디렉터리의 `services.ini`도 섞여 있었다.
- bash tool은 명령이 어디를 읽고 쓰는지 harness가 알 수 없다. 작업 디렉터리 밖 접근과 실행 사이의 격리는 H7 Permissions와 H8 Sandbox에서 다룬다.

### 편집 tool 만들기

두 tool은 모두 경로를 받고, 작업 디렉터리 밖이면 거부한다. `read_file`은 이미 있는 파일만 다루므로 경로 전체를 실제 위치로 풀어서 확인하면 됐다. `write_file`은 아직 없는 파일도 만들어야 해서, 상위 디렉터리를 실제 위치로 풀어 확인한다. 이미 있는 파일이 작업 디렉터리 밖을 가리키는 symlink이면 그것도 거부한다.

```rust
fn writable_target(workdir: &Path, path: &str) -> Result<PathBuf, String> {
    let root = fs::canonicalize(workdir).map_err(|e| format!("working directory: {e}"))?;
    let joined = root.join(path);
    let (Some(parent), Some(name)) = (joined.parent(), joined.file_name()) else {
        return Err(format!("{path}: not a file path"));
    };
    let parent = fs::canonicalize(parent).map_err(|e| format!("{path}: {e}"))?;
    let target = parent.join(name);
    let resolved = fs::canonicalize(&target).unwrap_or_else(|_| target.clone());
    if !resolved.starts_with(&root) {
        return Err(format!("{path}: outside the working directory"));
    }
    Ok(target)
}
```

`write_file`은 이 경로에 내용을 그대로 쓰고 `wrote N bytes to <path>`를 돌려준다. `search_replace`는 논문 §16.10의 예시와 같은 계약이다.

```rust
fn search_replace(workdir: &Path, args: &Value) -> Result<String, String> {
    let path = string_arg(args, "path")?;
    let search = string_arg(args, "search")?;
    let replace = string_arg(args, "replace")?;
    if search.is_empty() {
        return Err("search must not be empty".to_string());
    }
    let target = writable_target(workdir, path)?;
    let text = fs::read_to_string(&target).map_err(|e| format!("{path}: {e}"))?;
    let count = text.matches(search).count();
    if count != 1 {
        return Err(format!("search string occurs {count}x in {path}; must be unique"));
    }
    fs::write(&target, text.replacen(search, replace, 1)).map_err(|e| format!("{path}: {e}"))?;
    Ok(format!("replaced 1 occurrence in {path}"))
}
```

- `search`가 한 번도 없거나 여러 번 나오면 파일을 건드리지 않고 등장 횟수를 돌려준다. 빈 문자열은 모든 위치에 일치하므로 따로 거부한다.
- 두 tool 모두 실행 기록에는 편집(`edit`) 범주로 남는다.

tool은 지금처럼 `--tools`로 고른다.

```bash
hel --tools bash,write_file,search_replace
```

### 편집 tool을 줬을 때

```bash
evals run h02 --conditions variant-write,variant-write-replace
```

`variant-write`는 bash와 `write_file`, `variant-write-replace`는 bash, `write_file`, `search_replace`를 준 구성이다. 시작 상태의 결과와 함께 적었다.

| task | 구성 | 통과 | input / output token (평균) | model 호출 (평균) | tool 호출 (평균) |
| --- | --- | --- | --- | --- | --- |
| `edit-line-01` | 시작 상태 (`bash`) | 3/3 | 6110 / 1028 | 5 | `bash` 3.7 |
| | 전체 쓰기 추가 | 3/3 | 7742 / 1116 | 7 | `bash` 6.3 |
| | 문자열 치환 추가 | 3/3 | 6422 / 777 | 6 | `bash` 4.7, `search_replace` 1.0 |
| `edit-ambiguous-01` | 시작 상태 (`bash`) | 3/3 | 7806 / 1376 | 5 | `bash` 4.7 |
| | 전체 쓰기 추가 | 3/3 | 6111 / 954 | 5 | `bash` 5.0 |
| | 문자열 치환 추가 | 3/3 | 8218 / 887 | 5 | `bash` 3.7, `search_replace` 0.7 |

(`hel` `90c8058`, model `deepseek-flash`, 최대 model 호출 10번, macOS.)

**`write_file`은 한 번도 쓰지 않았다.** 전체 쓰기를 준 6번과 문자열 치환까지 준 6번, 모두 12번 중 `write_file` 호출은 0번이다. 전체 쓰기만 더한 구성에서 model은 시작 상태와 같이 bash 안에서 python, `awk`, `sed`로 고쳤다. 그래서 237줄 파일을 통째로 다시 쓰는 비용은 이번 실행에서 나타나지 않았다.

**`search_replace`는 여섯 번 중 다섯 번 썼다.** `edit-line-01`은 세 번 모두, `edit-ambiguous-01`은 두 번이다. `edit-ambiguous-01`에서 넘긴 `search`는 이렇다.

```text
실행 2  search='[database]\nhost = db.internal\nport = 5432\nretries = 3\ntimeout = 10\npool_size = 20'
실행 3  search='port = 5432\nretries = 3'
```

- 다섯 번 모두 첫 호출에 성공했다. "여러 곳 일치" 오류를 받고 고친 경우는 없었다. `edit-ambiguous-01`에서는 파일을 읽은 뒤 처음부터 `[database]` 섹션에만 있는 줄을 붙여 넘겼다. 실행 2는 섹션 전체를, 실행 3은 바로 앞줄(`port = 5432`)만 붙였다.
- 남은 한 번(`edit-ambiguous-01` 실행 1)은 `search_replace`를 두고 bash 안에서 python으로 섹션을 따라가며 고쳤다.
- `search_replace`를 쓴 실행도 bash 호출은 3~6번이었다. 편집 전에 `grep`, `sed -n`으로 위치를 확인하고, 편집 뒤에 다시 확인하는 흐름은 그대로였다.

output token은 문자열 치환을 준 구성이 두 task 모두 가장 적었다. 실행마다 차이가 커서 차이를 단정하기는 어렵다.

| task | 구성 | 실행별 output token |
| --- | --- | --- |
| `edit-line-01` | 시작 상태 | 659, 1955, 469 |
| | 문자열 치환 추가 | 670, 1010, 652 |
| `edit-ambiguous-01` | 시작 상태 | 2102, 1359, 668 |
| | 문자열 치환 추가 | 909, 1222, 531 |

- 시작 상태에서 output token이 컸던 실행(1955, 2102)은 python 스크립트를 길게 쓰거나 `sed` 실패 뒤 다시 고친 실행이다. 문자열 치환을 쓰면 이런 긴 편집 명령이 사라져서 위쪽 값이 낮아졌다. 짧게 끝난 실행끼리는 차이가 거의 없다.
- input token은 줄지 않았다(6110 → 6422, 7806 → 8218). tool이 셋으로 늘어 요청마다 tool 정의가 길어졌고, 확인용 bash 호출 수가 비슷해서 대화 길이도 비슷했다.

작업 디렉터리 밖에 접근한 실행 수는 이렇다. 실행 기록의 명령에 `/tmp`나 `/` 같은 작업 디렉터리 밖 경로가 나오는지로 셌다.

| 구성 | 작업 디렉터리 밖 접근 | 내용 |
| --- | --- | --- |
| 시작 상태 | 4/6 | `/tmp` 백업 4번, 그중 한 번은 `find /`도 실행 |
| 전체 쓰기 추가 | 4/6 | `/tmp` 백업이나 임시 파일 4번 |
| 문자열 치환 추가 | 1/6 | `/tmp` 백업 1번 |

- 백업은 bash로 직접 고치기 전에 만든다. `search_replace`로 고친 다섯 번 중 백업을 만든 실행은 한 번이었다.
- `search_replace` 자체는 작업 디렉터리 밖 경로를 거부하지만, bash가 함께 있으므로 model이 bash로 밖에 접근하는 것까지 막지는 못한다.

### 새 파일을 만들 때

`write_file`을 한 번도 쓰지 않은 것이 tool을 고르는 기준 때문인지 확인하려고, 새 파일을 만드는 task를 하나 더 만들어 구성마다 2번씩 실행했다. 지시는 `$`, 백틱, 따옴표가 섞인 20줄짜리 shell script를 그대로 `deploy.sh`로 만드는 것이다.

```bash
evals try write-new-01 --lab h02 --condition variant-write
```

| 구성 | 통과 | 파일을 만든 방법 | input / output token (실행별) |
| --- | --- | --- | --- |
| 시작 상태 (`bash`) | 2/2 | 따옴표로 감싼 heredoc(`cat > deploy.sh <<'EOF'`) | 5284 / 1225, 5354 / 1182 |
| 전체 쓰기 추가 | 2/2 | `write_file` | 6773 / 1790, 3913 / 1266 |
| 문자열 치환 추가 | 2/2 | `write_file` | 3890 / 1001, 11485 / 962 |

- `write_file`이 있으면 네 번 모두 첫 호출에서 `write_file`로 만들었다. 기존 파일의 한 줄을 고칠 때는 12번 중 한 번도 쓰지 않았으므로, model은 파일을 새로 만들 때만 전체 쓰기를 고른 것으로 보인다.
- bash만 있을 때도 두 번 모두 heredoc 구분자를 따옴표로 감싸서 `$`와 백틱이 해석되지 않게 했다.
- 만든 뒤에는 모든 실행이 내용을 다시 확인했다. 그중 다섯 번은 기대 내용 전체를 heredoc이나 python 문자열로 한 번 더 써서 `diff`로 비교했다. 그래서 파일 내용을 두 번 출력하는 셈이 되어 output token이 구성과 상관없이 1000 안팎이었다. 시작 상태의 두 번은 비교용 사본을 `/tmp`에 만들었다.

macOS 환경에서 생긴 bash 명령 실패도 있었다. macOS의 `cat`에는 `-A` 옵션이 없고, 측정 도구가 `hel`을 `PATH=/usr/bin:/bin`으로 실행해 `/sbin`에 있는 `md5sum`을 찾지 못해서, 여러 실행이 확인 명령에서 오류를 받고 다른 명령으로 바꿨다(PATH는 H3에서 `/usr/sbin:/sbin`을 더하도록 고쳤다). 전체 쓰기를 준 첫 실행은 시작 상태의 두 번째 실행과 같은 `sed -i '35s/.../'` 실패를 겪고 python으로 다시 고쳤다.

## 돌아보기

### 변경 사항

- 편집 tool을 주자 model은 작업에 맞춰 골라 썼다. 기존 파일의 한 줄을 고칠 때는 `search_replace`를 썼고(6번 중 5번), 새 파일을 만들 때는 `write_file`을 썼다(4번 중 4번). 기존 파일을 고치는 데 `write_file`을 쓴 적은 없다.
- 작업 디렉터리 밖에 접근한 실행이 6번 중 4번에서 1번으로 줄었다. bash로 직접 고칠 때 만들던 `/tmp` 백업이 필요 없어졌고, 전용 tool을 통해 model의 행동 예측과 결과 관리를 harness 내부로 숨길 수 있다는 것도 확인할 수 있었다.
- 정확성은 달라지지 않았다. bash만 있을 때도 model은 `sed` 대신 bash 안에서 python으로 고쳤고, 바뀐 곳이 하나인지 스스로 검사해서 모두 정확히 고쳤다.
- `--tools`에 `write_file`, `search_replace`를 넣어 준다. `--tools`를 생략하면 지금처럼 bash 하나로 시작한다.

### 트레이드오프

- input token은 새로운 tool에 대한 설명으로 늘어났다(6110 → 6422, 7806 → 8218). tool 정의가 길어진 만큼 매 호출에 붙고, 편집 전후의 확인 단계는 bash로 그대로 한다. output token은 줄었지만 실행마다 차이가 커서 지금 규모로는 의미 있는 차이로 보기 어렵다.
- `search_replace`는 공백이나 들여쓰기가 한 글자만 달라도 일치하지 않는다. 이번에는 다섯 번 모두 첫 호출에 성공했지만, 들여쓰기가 깊은 코드 파일에서는 실패할 수 있다. 느슨하게 맞추는 방법은 [FAQ](faq.md)에 남겼다.
- `write_file`은 기존 파일도 통째로 덮어쓴다. 읽지 않은 파일을 덮어쓰거나 다른 내용을 잃는 것을 harness가 막지 않는다. 읽은 파일만 고치게 하는 정책은 H7 Permissions와 H10 Skills / Hooks / MCP에서 다룬다.
- bash가 함께 있는 한 작업 디렉터리 밖 접근은 남는다. 전용 tool은 밖을 거부하지만 model이 bash로 `/tmp`에 쓰는 것까지 막지는 못한다. 명령 단위의 허용 여부는 H7, 실행을 가두는 문제는 H8에서 다룬다.

### 논문의 내용 또는 다른 harness와 비교하면

논문 §16.3은 "파일 전체를 다시 쓰느라 token이 낭비되면 `search_replace`를 더하라"고 했다. 이번 model은 처음부터 기존 파일을 통째로 다시 쓰지 않아서 그 신호가 생기지 않았다. `search_replace`가 바꾼 것은 token보다 작업 범위였다. bash로 고칠 때 따라오던 백업과 임시 파일이 줄었다.(추가적으로 저장소 외부에 대한 접근도 줄었다.)

§16.4는 공개 model이나 약한 model에는 느슨한 매칭을 권한다. 이번에 쓴 model은 공개 model이지만 정확 매칭만으로 다섯 번 모두 첫 호출에 성공했다. 여러 섹션에 같은 줄이 있을 때도 고유한 앞줄을 붙여 넘겼다. 이번 두 task 범위에서는 Mistral Vibe가 옮겨 간 쪽인 엄격한 계약으로 충분했다.

Mini-SWE-Agent는 시스템 prompt에 `sed` 예시와 macOS용 `sed -i ''` 안내를 넣는다. `hel`은 이런 안내 없이 tool 설명만 준다. model은 `sed`를 18번 중 세 번만 썼다. 그중 두 번은 macOS `sed`의 `-i` 차이로 실패한 뒤 python으로 다시 고쳤고, 한 번은 처음부터 `sed -i ''`로 썼다.

Claude Code는 부분 수정에 Edit, 새 파일과 전체 쓰기에 Write를 쓰게 한다. 이번 model이 두 tool을 나눠 쓴 방식과 같다. Claude Code는 여기에 읽기 전 편집 금지와 읽은 뒤 바뀐 파일 감지를 더한다. 지금의 `hel`에는 둘 다 없다.
