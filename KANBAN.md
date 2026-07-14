# KANBAN — 로봇 축구 1:1 데모

> 진행 방식: TDD(테스트 먼저) + 칸반 순서. 자세한 규칙은 [docs/05-개발프로세스.md](docs/05-개발프로세스.md).
> WIP 한도: **In Progress = 1** (한 번에 한 카드).

**카드 형식**
```
- [ ] KB-NN 제목 — 한 줄 설명 (테스트: 검증 내용) [의존: KB-xx]
```

**Definition of Done**: 관련 테스트 통과 + 커밋 완료 + (해당 시) 문서 갱신.

---

### 계획(에픽) 로드맵
수직 슬라이스 우선. 각 Plan은 독립 동작 소프트웨어를 낸다.

| Plan | 내용 | 문서 |
|---|---|---|
| **Plan 1 — 걷는 뼈대** ✅완료 | 결정적 sim + Controller + WS 30Hz + canvas + 골/스코어 | [계획](docs/superpowers/plans/2026-07-03-walking-skeleton.md) |
| **Plan 2 — 물리/충돌(rapier2d)** ✅완료 | 밀기 드리블·벽 반사·골 센서·누산기·리플레이 | [계획](docs/superpowers/plans/2026-07-08-physics-collision.md) |
| **Plan 3a — 파츠/로드아웃/스탯** ✅완료 | 파츠 조립·스탯→물리·카탈로그·비대칭 프리셋 | [계획](docs/superpowers/plans/2026-07-08-parts-loadout.md) |
| **Plan 3b — 전투/데미지** ✅완료 | 부위 콜라이더·충돌 이벤트·상호 데미지·부위HP·파손다운 | [계획](docs/superpowers/plans/2026-07-08-combat-damage.md) |
| **Plan 3c — 효과 선택** ✅완료 | 넉백/스턴/데미지 effect 프로필·impact 비례 중첩 | [계획](docs/superpowers/plans/2026-07-08-combat-effects.md) |
| **Plan 4a — 사람 조작 최소 슬라이스** ✅완료 | 슬롯 참가+키보드 uplink+사람 조종+클라 상태렌더 | [계획](docs/superpowers/plans/2026-07-08-interactivity.md) |
| **Plan 4b — 전략 모드·AI 토글** ⭐다음 | 마우스 전략·런타임 제어 전환 | (예정) |
| Plan 5 — 게임 흐름 | ATTRACT/SELECT/PLAYING/RESULT·슬롯 UI | (예정) |
| Plan 6 — 랭킹 | 로봇별 승률 인메모리 | (예정) |
| **Plan 7a — 넷코드 견고성 시연** ✅완료 | 보간·서버 링크심(지연/지터/드랍)·ping/RTT | [ADR-013](docs/07-결정기록-ADR.md) |
| Plan 7 — 넷코드 잔여 | 인바운드 링크심·재연결·슬롯 유예 | (예정) |
| Plan 8 — 폴리싱·README·GIF·CI | 관측성·ADR·데모 | (예정) |

---

## Backlog

**Plan 4b+** — 각 Plan 착수 시 writing-plans로 카드 추가.

**남은 관찰/부채 (후속)**
- 클라 보간 — 아직 최신 스냅샷 렌더 / 포트·URL 상수화(8090×2), 클라 재연결·try-catch / main publish 프레임당 1회(스톨 시 순간 <30Hz) / 클라 vitest 미설정
- (Plan 3a Minor) `apply_controls` 중복 가드·테스트명 정확성·aggregate slot 유니크 assert — 코스메틱
- (Plan 3b Minor) 리플레이 전투 해시 테스트가 대칭AI라 데미지 없이 통과 가능(메커니즘은 강제충돌 테스트로 검증됨) / 다운 로봇도 접촉 데미지 가함(물리 장애물) / PART_NAMES↔part_count 결합 debug_assert — 전부 선택
- (Plan 3c 튜닝/후속) impact=상대속도(post-step)·**부위별 취약도(KB-34 스킵)**·다중 부위쌍 동시 데미지·**효과 가중치 본격 밸런싱**(현재 초기값만: fore-std kb 0.6 / body-std stun 0.5 / body-light dmg 0.4)
- (Plan 4a 후속) **`Controller::as_any_mut`(Any 다운캐스트) → Plan 4b에서 슬롯 상태 늘면 `SlotController` enum으로 리팩터** / mpsc 백프레셔 없음(하드닝 시) / **브라우저 시각검증 미완**(회전방향 ←=CCW·HP바·스턴/다운 렌더·상대와 충돌 시 라이브 전투 — 사용자가 `cargo run`+`npm run dev`로 확인)

## Todo
- [ ] KB-60 방어형 범위 확대 + 공 인지 빠르게 — DefenderAi가 공을 늦게 인지하고 방어 범위가 좁음. `DEFENDER_GUARD_DIST`(2.5m) 상향 + 자기 진영 접근을 이르게 감지해 나가 막기 (사용자 요청, 내일 진행)
- [ ] 밸런스 결정 — striker가 guard와 몸싸움서 항상 밀림(의도된 비대칭). 유지/guard약화/대칭 중 택 (미정)
- [ ] 다리로 슛 연출(앞다리 차는 애니, 클라 렌더)

## In Progress
_(비어 있음)_

## Done
**UI 비주얼 디자인 ✅** (branch `feat/interactivity`)
- [x] KB-51 미드나잇 프리시전 콘솔(크롬) + Neon Telemetry Arena(게임화면) — 독립 리뷰어 2패널로 방향 결정([ADR-014](docs/07-결정기록-ADR.md)), 시안 승인 후 클라 적용(index.html 콘솔 레이아웃·style.css·render.ts 아트·hud.ts/devpanel.ts HUD 배선). netsim/RTT를 일급 LINK MONITOR로 격상, 스코어/시계 크롬 HUD 이전. 기능 불변(보간·ping/RTT·netsim·참가·입력), 클라 빌드 clean, 배선 id 12개 정합 확인.

**Plan 7a — 넷코드 견고성 시연 ✅** (branch `feat/interactivity`)
- [x] KB-50 보간 + 서버 아웃바운드 링크심(지연/지터/드랍) + ping/RTT + 개발패널 (테스트: 서버 81/81, debug+release warning 0, 클라 빌드 clean, sim/리플레이 불변) *(인바운드·재연결·예측 제외 = YAGNI)*

**팀 로스터/협동 ✅** (branch `feat/interactivity`)
- [x] KB-59 방어형 제자리 스핀 수정 — 목표(골-공 선상) 도착 시 목표방향이 ≈0→각도 노이즈로 뱅뱅 돌던 문제. 도착하면 전진 멈추고 공 바라보며 대기 + 회전 데드존. (테스트: 도착 시 thrust 0·turn≈0, cargo test 99/99)
- [x] KB-58 공 이탈 버그 수정 — AI 강킥이 가벼운 공을 수십 m/s로 쏴 벽 터널링→필드 이탈(공 안 보이고 로봇이 흩어짐). 4중 수정: kick_power 재조정(12/9→1.4/1.05, 세기 gradient 보존), 공 속도 상한(12m/s), 공 CCD, 이탈 시 중앙 복귀 안전망. + 공 렌더 글로우. (헤드리스 재현 테스트 30초 4대: 공 유한·필드 내, cargo test 98/98)
- [x] KB-57 팀당 2대(공격형 striker + 방어형 guard, 총 4대) + 협동 AI(역할 분담: 공격=추적·슛 / 수비=골 지킴·클리어) + 친선 데미지 금지. 사람은 팀 striker만 조종. 2대 레거시 생성자 보존, 신규 new_match(4대) ([ADR-015]). (서버 97/97, debug+release warning 0, 클라 빌드 clean; 클라 gait·interp 인덱스 매칭 수정)

**플레이테스트 후속 ✅** (branch `feat/interactivity`)
- [x] KB-56 모델 외형 구분 — guard=통통(폭↑·두꺼운 다리), striker=얇고 길쭉·가는 다리. 스냅샷 `robot` 프리셋 id로 렌더 사이징(클라 전용)
- [x] KB-55 슬롯 참가 UX — 참가 버튼 토글(내 팀=나가기/타 팀=전환), AFK 30초 자동 해제→AI, 참가 상태 표시(사람/AI/YOU 배지). 스냅샷 `ctrl[]` 방출, 클라 낙관적 추적+`ctrl` 재조정(세션id 미도입). (서버 85/85, 클라 빌드 clean, DOM id 18개 정합)
- [x] KB-54 코너 챔퍼 시각화(팔각 경계 + 잘린 코너 벽)
- [x] KB-53 스태미나 회복 정책 — 가만히 있을 때(이동 입력 없음)만 회복, 걷는 중엔 유지 ([ADR-011] addendum). (테스트: idle 회복/walk 유지, cargo test 85/85)
- [x] KB-52 AI 슛 — `ChaseBallAi`가 공 정면 사거리 안 + 상대 골 정렬 시 슛(자책골 방지). "AI 킥 없음"(KB-48) 갱신 ([ADR-012] addendum). (테스트: 정렬 슛/자기골 무슛/사거리밖 무슛, cargo test 84/84)
- [x] KB-49 AI 스턱 탈출 — 벽/펜스/코너에 박혀 정지(속도≈0)가 ~1초 지속되면 후진+중앙쪽 회전으로 빠져나옴 (테스트: 스턱→후진 전환·정상주행 오탐 없음, 리플레이 해시 불변)
- [x] KB-47 섀시 6족(거미형) 확정 + 다리 렌더·보행 애니(트로트/삼각보행, 클라 전용)
- [x] KB-48 차기(kick) 풀스택 — 탭 발사·세기(↑↓)·방향(←→)·로봇별 kick_power·정면 사거리·shoot_lock 쿨다운 (테스트: cargo test 통과, warning 0, 클라 빌드)

**플레이테스트 후속 — 필드/체력 ✅** (branch `feat/interactivity`)
- [x] KB-43 골 입구 펜스(GOALFENCE 충돌그룹): 로봇은 막히고 공은 통과
- [x] KB-44 코너 45° 챔퍼: 공이 구석에 끼는 문제 완화
- [x] KB-45 스태미나/스프린트 — 걷기 상시·달리기(Shift)는 stamina>0에서만, 0이면 걷기로 폴백·재생 (테스트: cargo test 48/48, debug+release warning 0, 순수/결정성 유지, 프리셋 sprint>walk 불변식 가드, 클라 스태미나바) *(오버히트·AI스프린트 제외 = YAGNI)*

**Plan 4a — 사람 조작 최소 슬라이스 ✅** (branch `feat/interactivity`)
- [x] KB-36 HumanController(최근 입력 보유)
- [x] KB-37 업링크 파싱(join/input/leave, serde_json::Value)
- [x] KB-38 WS select! recv→mpsc + AtomicU64 세션id
- [x] KB-39 슬롯 Controller 스왑(사람↔AI)+입력 적용(+멱등 join)
- [x] KB-40 클라 키보드 입력 + 참가 버튼
- [x] KB-41 클라 HP/스턴/다운 렌더
- [x] KB-42 검증: cargo test 41/41, debug+release warning 0, 클라 빌드, WS 사람입력 스모크(blue 이동·st 존재). 브라우저 시각검증은 사용자 몫

**Plan 3c — 효과 선택 ✅** (branch `feat/combat-damage`)
- [x] KB-30 effect 프로필 + 임팩트 비례 중첩 선택(순수)
- [x] KB-31 스턴 타이머(순수)
- [x] KB-32 충돌 시 넉백(임펄스)/스턴(입력차단)/데미지 적용
- [x] KB-33 스냅샷 st에 "stun"
- [x] KB-34 dmg_w 가산 배선 + 카탈로그 효과값(실전 넉백/스턴 발동, 비대칭) *(부위 취약도는 스킵)*
- [x] KB-35 검증: cargo test 33/33, debug+release warning 0, 실전 프리셋 충돌 효과 확인

**Plan 3b — 전투/데미지 ✅** (branch `feat/combat-damage`)
- [x] KB-24 데미지 공식(순수)
- [x] KB-25 부위 HP + 파손다운/리페어(순수)
- [x] KB-26 부위별 복합 콜라이더 + user_data 태깅
- [x] KB-27 충돌 이벤트→상호 데미지(로봇↔로봇만, part_map 멤버십 필터)
- [x] KB-28 다운 입력 무시 + 스냅샷 디버프 필드(parts/down/st)
- [x] KB-29 검증: cargo test 27/27, debug+release warning 0, 스냅샷 디버프 필드 확인. 라이브 충돌은 비대칭 필요(대칭 AI 미접촉)

**Plan 3a — 파츠/로드아웃/스탯 ✅** (branch `feat/walking-skeleton`)
- [x] KB-18 파츠/스탯 카탈로그 + 로드아웃 집계
- [x] KB-19 물리에 로봇별 스탯 반영(accel/turn/maxSpeed/mass)
- [x] KB-20 maxSpeed 클램프
- [x] KB-21 catalog 다운링크 + 스냅샷 robot preset id
- [x] KB-22 main 비대칭 프리셋(striker/guard) + 헤드리스 검증
- [x] KB-23 검증: cargo test 17/17, 릴리스 warning 0, WS 비대칭 이동+catalog 확인

**Plan 2 — 물리/충돌(rapier2d) ✅** (branch `feat/walking-skeleton`)
- [x] KB-11 rapier2d 0.26 + 물리 월드(벽/공/로봇2)
- [x] KB-12 물리 스텝 + 골 판정·리셋
- [x] KB-13 골 입구 벽 분리 + 라이브 득점 로직
- [x] KB-14 tick→PhysicsWorld, kinematic sim 은퇴 (+ KICKOFF 단일 소스)
- [x] KB-15 고정스텝 누산기(+spiral cap) + main 배선
- [x] KB-16 골든 리플레이 + 상태 해시 (#[cfg(test)])
- [x] KB-17 검증: cargo test 11/11, WS E2E(공 물리 이동 확인). 대칭 AI라 라이브 골은 비대칭 필요

**Plan 1 — 걷는 뼈대 ✅** (branch `feat/walking-skeleton`)
- [x] KB-01 프로젝트 스캐폴딩 — server(cargo)·client(vite-ts)
- [x] KB-02 월드 타입·상수
- [x] KB-03 결정적 공 적분(마찰)
- [x] KB-04 로봇 이동(thrust/turn)
- [x] KB-05 골 판정·스코어·리셋
- [x] KB-06 Controller 트레잇 + ChaseBall AI
- [x] KB-07 고정 timestep tick 함수
- [x] KB-08 WebSocket 30Hz 브로드캐스트 + sim 루프 (+ 경고 정리)
- [x] KB-09 클라 수신·canvas 렌더
- [x] KB-10 검증: cargo test 8/8, WS E2E(curl로 101+state 프레임), 포트 8080→8090
