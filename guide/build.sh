#!/usr/bin/env bash
# 가이드를 빌드하고 한국어 검색 index를 만든다.
# 필요한 도구와 버전은 guide/README.md를 참고한다.
set -euo pipefail
cd "$(dirname "$0")"
mdbook build
"${PAGEFIND:-pagefind}" --site book
