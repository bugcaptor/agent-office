# scripts

프로젝트 유지보수용 스크립트 모음.

## bump-version.mjs

버전이 들어있는 5개 파일을 한 번에 갱신한다:
`package.json`, `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, `src-tauri/tauri.conf.json`.

### 사용법

```bash
node scripts/bump-version.mjs [major|minor|patch]   # 기본값: patch
node scripts/bump-version.mjs <x.y.z>               # 특정 버전 직접 지정
```

npm 스크립트로도 실행할 수 있다:

```bash
npm run bump          # patch
npm run bump:patch
npm run bump:minor
npm run bump:major
```

### 자리수 규칙

자리수를 올리면 아래 자리수는 0으로 리셋된다.

| 인자 | 예시 |
| --- | --- |
| `major` | `1.4.2` → `2.0.0` |
| `minor` | `1.4.2` → `1.5.0` |
| `patch` | `1.4.2` → `1.4.3` |

`x.y.z` 형식으로 직접 지정하면 계산 없이 그 버전으로 맞춘다 (예: `node scripts/bump-version.mjs 2.3.4`).

### 참고

- 현재 버전은 `package.json`의 `version`을 기준으로 읽는다.
- `Cargo.lock`은 `name = "agent-office"` 패키지 블록의 `version`만 교체하므로 의존성 버전은 건드리지 않는다.
- 스크립트는 버전만 바꾸며 커밋은 하지 않는다. 변경 후 직접 커밋하면 된다.
