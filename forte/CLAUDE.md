# Claude 작업 가이드

이 문서는 Claude가 Forte 프로젝트에서 작업할 때 따라야 할 프로세스를 정의합니다.

---

## 📝 작업 프로세스

### 1. 작업 시작 전

작업을 시작하기 전에 **반드시** 다음을 수행하세요:

```bash
1. TODO.md 파일을 읽어서 현재 할 일 목록 확인
2. 우선순위를 고려하여 작업할 항목 선택
   - 🔴 Critical > 🟡 Important > 🟢 Nice to Have
3. 선택한 항목의 관련 파일들을 읽고 현재 상태 파악
```

### 2. 작업 중

```bash
1. 코드 작성 시 관련 파일에 명시된 패턴 따르기
2. 테스트 추가 (가능한 경우)
3. 변경사항이 다른 부분에 영향을 주는지 확인
```

### 3. 작업 완료 후

작업을 완료한 후 **반드시** 다음을 수행하세요:

```bash
1. TODO.md에서 완료한 항목의 **전체 섹션 삭제**
   - 체크박스만 체크하지 말 것!
   - 해당 제목(###) 부터 다음 제목 전까지 모두 삭제

2. 구현 중 발견한 새로운 TODO가 있다면 TODO.md에 추가
   - 적절한 우선순위 섹션에 추가
   - 파일 경로, 현재 상태, 구현 필요 사항 명시

3. git commit 생성 (사용자가 요청한 경우만)
```

---

## 🎯 작업 예시

### 예시 1: POST 액션 구현

**시작 전:**
```markdown
1. TODO.md 읽기
2. "POST 액션 완전 구현" 섹션 확인
3. src/codegen/backend.rs:199-206 읽기
4. 현재 상태 파악: wrapper가 501 반환
```

**작업:**
```markdown
1. wrapper_post 함수 실제 구현
2. 요청 바디 파싱 로직 추가
3. ActionResult 처리 로직 추가
4. 테스트 작성
```

**완료 후:**
```markdown
1. TODO.md에서 "### 1. POST 액션 완전 구현" 섹션 전체 삭제
   (제목부터 다음 "---" 또는 다음 "###" 전까지)

2. 새로운 TODO 발견 시 추가:
   예: "ActionResult에 Cookie 설정 기능 필요"
```

### 예시 2: 새로운 TODO 추가

발견한 이슈:
```
클라이언트 네비게이션 구현 중
Scroll Restoration 기능이 필요함을 발견
```

TODO.md에 추가:
```markdown
## 🟢 Nice to Have

### 10. Scroll Restoration
**파일:** `.generated/frontend/client.ts`

**구현 필요:**
- [ ] 페이지 전환 시 스크롤 위치 저장
- [ ] 뒤로가기 시 스크롤 위치 복원
- [ ] SessionStorage 활용

**관련 파일:**
- `.generated/frontend/client.ts` - 클라이언트 런타임
```

---

## 🚫 하지 말아야 할 것

### ❌ 체크박스만 체크하기
```markdown
# 잘못된 예
### 1. POST 액션 완전 구현
- [x] 요청 바디 파싱 로직
- [x] post_action 함수 호출
- [x] ActionResult 처리
```

이렇게 하면 TODO.md가 완료된 항목으로 가득 차서 지저분해집니다.

### ✅ 섹션 전체 삭제하기
```markdown
# 올바른 예
(해당 섹션을 완전히 삭제하여 TODO.md에서 사라지게 함)
```

---

## 📁 주요 파일 구조

작업 시 참고할 주요 파일들:

### CLI 명령어
- `src/commands/init.rs` - 프로젝트 초기화
- `src/commands/dev.rs` - 개발 서버
- `src/commands/build.rs` - 프로덕션 빌드
- `src/commands/test.rs` - 테스트 실행

### 코어 엔진
- `src/watcher/mod.rs` - 파일 감시 및 라우트 스캔
- `src/parser/rust_parser.rs` - Rust 코드 파싱
- `src/generator/typescript.rs` - TypeScript 생성
- `src/codegen/backend.rs` - 백엔드 코드 생성
- `src/codegen/frontend.rs` - 프론트엔드 코드 생성

### 런타임
- `src/runtime/mod.rs` - Wasmtime 임베딩
- `src/server/mod.rs` - HTTP 프록시 서버

### 템플릿
- `src/templates/mod.rs` - 템플릿 관리
- `src/templates/frontend_ssr.rs` - SSR 서버 템플릿

---

## 🧪 테스트 작성 가이드

새로운 기능 구현 시:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_새로운_기능() {
        // Given
        let input = ...;

        // When
        let result = 함수(input);

        // Then
        assert_eq!(result, expected);
    }
}
```

통합 테스트:
```rust
// tests/integration/기능명.rs
#[test]
fn test_e2e_새로운_기능() {
    // E2E 테스트 작성
}
```

---

## 💡 코딩 가이드라인

### 에러 처리
```rust
// anyhow::Result 사용
use anyhow::{Context, Result};

pub fn 함수() -> Result<T> {
    something()
        .context("설명적인 에러 메시지")?;
    Ok(result)
}
```

### 코드 생성
```rust
// quote! 매크로 사용
use quote::quote;

let generated = quote! {
    pub async fn wrapper() -> Response {
        // ...
    }
};
```

### 파일 감시
```rust
// Debounce 항상 사용 (300ms)
use notify_debouncer_full::new_debouncer;

let debouncer = new_debouncer(
    Duration::from_millis(300),
    None,
    handler
)?;
```

---

## 🔄 작업 흐름 요약

```
1. TODO.md 읽기
   ↓
2. 작업 항목 선택 (우선순위 고려)
   ↓
3. 관련 파일 읽고 현재 상태 파악
   ↓
4. 코드 구현
   ↓
5. 테스트 작성 (가능한 경우)
   ↓
6. TODO.md에서 완료한 섹션 **전체 삭제**
   ↓
7. 새로운 TODO 발견 시 추가
   ↓
8. (사용자 요청 시) git commit
```

---

## 📌 중요 원칙

1. **TODO.md는 항상 최신 상태 유지**
   - 완료된 항목은 즉시 삭제
   - 새로운 항목은 즉시 추가

2. **우선순위 존중**
   - 🔴 Critical 먼저 처리
   - 사용자가 특정 항목 요청 시 예외

3. **코드 품질 유지**
   - 테스트 작성
   - 에러 처리 철저히
   - 주석은 "왜"를 설명 (코드는 "무엇"을 설명)

4. **문서화**
   - 복잡한 로직은 주석 추가
   - TODO.md에 충분한 컨텍스트 제공

---

이 가이드를 따라 일관된 작업 프로세스를 유지하세요!
