# FAQ

## bash로는 작업 디렉터리 밖도 읽을 수 있다

H0의 `read_file`은 작업 디렉터리 밖의 경로를 거부한다. bash tool에는 이런 검사가 없어서 `cat ../다른파일`이나 `cat ~/.ssh/...` 같은 명령도 그대로 실행된다. 이번 실행에서는 model이 작업 디렉터리 밖으로 나간 명령을 쓰지 않았다. 명령 문자열만으로 무엇을 하는지 판단해야 하는 문제는 H7 Permissions에서, 실행 자체를 가두는 문제는 H8 Sandbox에서 다룬다.

## 긴 출력은 그대로 model에게 간다

bash tool은 출력을 자르지 않는다. 큰 파일을 `cat`하거나 넓은 범위를 `find`하면 출력 전체가 대화에 들어가 token을 쓴다. Mini-SWE-Agent는 10,000자가 넘으면 앞뒤만 보내고, Claude Code는 약 30,000자가 넘으면 파일로 저장한 뒤 앞부분만 보낸다. 이번 task는 출력이 짧아서 차이가 없었다. context가 커질 때의 문제는 H5 Context Budget에서 다룬다.

## 전용 읽기 tool의 이점은 아직 보이지 않았다

작은 파일을 읽는 task에서는 `read_file`이 bash `cat`보다 token을 조금 덜 썼을 뿐이고, 두 tool을 함께 주면 tool 정의가 늘어 오히려 더 비쌌다. 논문과 Claude Code가 드는 전용 tool의 이점(큰 파일 일부 읽기, 권한 판단, 사용자가 작업을 검토하기 쉬움)은 이번 task로는 드러나지 않는다. 파일을 고치는 task에서는 전용 tool과 bash의 차이가 더 클 수 있어서 H2 Editing에서 다시 본다.
