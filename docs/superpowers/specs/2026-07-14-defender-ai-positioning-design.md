# 수비 AI 방어 범위 확대 + 조기 인지 · 설계

> 작성 2026-07-14. 대상: `server/src/control.rs`의 `DefenderAi`.
> 배경: [11-진행상태-핸드오프.md](../../11-진행상태-핸드오프.md) §7 "바로 다음" 작업.

## 문제

현재 `DefenderAi`는 **자기 골–공을 잇는 선 위, 골에서 최대 `DEFENDER_GUARD_DIST`(2.5m) 지점**을
목표로 한다. 결과적으로:

1. **방어 범위가 좁다** — 공이 골 2.5m 이내로 들어와야만 나가서 막는다.
2. **공을 늦게 인지한다** — 공의 *현재* 위치만 보고 목표를 잡아, 굴러오는 공에 뒤늦게 반응한다.

## 제약 (설계 공간)

- `DefenderAi`가 보는 정보는 `GameView { me, ball }` 뿐 — **자기 자신 + 공(pos, vel)**.
  팀메이트/상대 위치는 보이지 않는다. → 유일하게 활용 가능한 예측 신호는 **공 속도 `ball.vel`**.
- 결정적 코어 원칙: 순수 산술만(RNG·시간·HashMap 순회 배제). 골든 리플레이 안전 유지.
- 기존 동작(도착 대기 `DEFENDER_ARRIVE`, 회전 데드존 `TURN_DEADZONE`, 스턱 탈출,
  자책골 회피 슛 `wants_kick`)은 **변경하지 않는다**.

## 설계: 두 레버 결합

### 레버 1 — 속도 예측 (조기 인지)

목표선을 공의 현재 위치가 아니라 **예측 위치**로 긋는다:

```
pred = ball.pos + ball.vel * LOOKAHEAD
```

공이 자기 진영으로 굴러오면 도착 전에 미리 그 선상으로 이동한다. 정지한 공은
`pred == ball.pos`이라 기존 동작과 동일.

### 레버 2 — 진영 기반 2단 가드 거리 (범위 확대)

유효 가드 거리를 **예측 위치의 x**로 가변한다:

- 공(예측)이 **상대 진영** → `GUARD_HOME` (골 앞 대기, 보수적)
- 공(예측)이 **자기 진영** → `GUARD_ENGAGE` (더 멀리 나가 요격, 공격적)

블루 기준 자기 진영 = `pred.x <= 0`(own_goal_x가 −x쪽), 레드는 반대(`own_goal_x`부호로 일반화).
미드필드 경계에서의 급변을 막기 위해 판정 기준은 **현재 위치가 아닌 예측 위치 x**.

### 통합

`guard_target(team, ball)`은 시그니처 유지. 내부에서:

1. `pred = ball.pos + ball.vel * LOOKAHEAD`
2. 유효 거리 `guard = if own_half(team, pred.x) { GUARD_ENGAGE } else { GUARD_HOME }`
3. `own_goal → pred` 선 위, 골에서 `min(dist, guard)` 지점을 반환

`decide()`의 나머지 로직(도착 대기·데드존·스턱·슛)은 그대로.

## 제안 수치 (튜닝 시작점)

| 상수 | 현재 | 제안 | 근거 |
|---|---|---|---|
| `GUARD_HOME` | `DEFENDER_GUARD_DIST`=2.5 | **2.5** | 상대 진영 시 골 앞 유지(기존값 계승) |
| `GUARD_ENGAGE` | — | **4.0** | 자기 진영 시 x≈−2까지 전진해 요격 |
| `LOOKAHEAD` | — | **0.35s** | 6m/s 공을 ~2m 앞서 예측(공 max 12m/s, 필드폭 12m 기준 오버슛 방지) |

`DEFENDER_GUARD_DIST`는 `GUARD_HOME`으로 개명(의미가 "홈 대기 거리"로 명확해짐).
사용자 플레이테스트로 재튜닝 가능.

## 테스트 (TDD, 헤드리스 순수 함수)

신규:
- 공이 자기 진영으로 있을 때 목표가 `GUARD_ENGAGE`까지 확장되는지(골에서의 거리 > `GUARD_HOME`).
- 공이 상대 진영이면 목표가 `GUARD_HOME` 이내 유지(뭉침 방지 회귀).
- 속도 예측: 정지 공은 목표가 기존과 동일; 자기 진영으로 굴러오는 공(vel<0)은
  정지 공 대비 목표가 공 진행 방향으로 이동.

회귀(기존 defender 테스트 전부 통과):
- `defender_target_stays_near_own_goal_when_ball_is_far` — 단, 공이 **상대 진영** 멀리일 때로
  전제 정합(현재 테스트의 공 위치 x=5.0은 블루 기준 상대 진영이라 `GUARD_HOME` 적용, 통과).
- `defender_advances_when_ball_is_close_to_own_goal`
- `defender_holds_without_spinning_when_arrived`
- `defender_kicks_*` / `defender_stuck_*`

## 범위 밖 (YAGNI)

- 상대/팀메이트 인지(뷰에 없음), 부드러운 진영 전환 램프(하드 경계로 충분),
  요격 지점 정밀 예측(선형 예측이면 충분).
