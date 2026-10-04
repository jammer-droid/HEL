# guide/

공개용 학습 가이드의 원본이다. 사이트: <https://jammer-droid.github.io/HEL/>

`main`에 `guide/` 변경이 올라가면 GitHub Actions(`.github/workflows/guide.yml`)가 [mdBook](https://rust-lang.github.io/mdBook/)으로 빌드하고, [Pagefind](https://pagefind.app/)로 한국어 검색 index를 만들어 GitHub Pages에 배포한다. 저장소에는 Markdown 원본만 둔다.

> mdBook 내장 검색은 한국어를 index하지 않는다. 그래서 `book.toml`에서 내장 검색을 끄고, `theme/pagefind-search.js`가 Pagefind 검색창을 붙인다.

## 구성

| 경로 | 내용 |
| --- | --- |
| `src/SUMMARY.md` | 목차. 링크가 빈 항목(`[H1 Tool Loop]()`)은 아직 작성되지 않은 장이다 |
| `src/introduction/` | Introduction |
| `src/labs/` | Lab 글과 FAQ |
| `src/appendix/` | Appendix |
| `book.toml` | mdBook 설정 |
| `build.sh` | 빌드 스크립트. Actions가 실행한다 |
| `theme/` | Pagefind 검색 연동(JS, CSS) |
| `mermaid.min.js`, `mermaid-init.js` | Mermaid 다이어그램 렌더링 |
| `pagefind.yml` | Pagefind 설정 (본문만 index, 인쇄용/사이드바 페이지 제외) |
