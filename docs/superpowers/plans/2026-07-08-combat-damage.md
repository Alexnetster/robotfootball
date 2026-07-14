# Combat / Damage (per-part) — Plan 3b

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. 체크박스(`- [ ]`) 추적.

**Goal:** 로봇을 **부위별 콜라이더**(복합)로 만들고, 로봇↔로봇 충돌을 rapier 충돌 이벤트로 감지해 **상호 데미지**(부위별 HP 차감)를 적용한다. 부위 HP가 0이면 **파손 다운**(행동불능) → 일정 시간 뒤 전체 리페어. 스냅샷에 부위 HP·다운 상태를 실어 클라가 표시할 수 있게 한다. ([00 §12 전투 모델](../../00-개요-및-게임설계.md))

**Architecture:** 데미지·HP·파손다운은 **결정적 순수 로직**(`combat.rs`), 충돌 감지는 rapier 이벤트 수집(물리 I/O 경계). `Controller`/`tick` 불변. 공↔로봇은 데미지 없음(물리만). **효과 선택(넉백/스턴)은 Plan 3c** — 여기선 데미지·HP·파손다운까지.

**Tech Stack:** rapier2d 0.26(충돌 이벤트), 기존 스택. 설계: [00 §12](../../00-개요-및-게임설계.md), [02 §4 스냅샷/이벤트](../../02-네트워크-프로토콜.md), [07 ADR-008/009](../../07-결정기록-ADR.md).

> **rapier 충돌 이벤트:** 착수 전 컴파일 프로브(0.26.1)로 **검증 완료** — 아래 필수 반영 참조.

## ⚠️ 착수 전 필수 반영 (드라이런 점검 — 이 목록이 태스크 코드보다 우선)

프로브 결과: 충돌 이벤트 API는 0.26.1에서 동작. 아래 반영해야 컴파일/정확성/결정성이 산다.

1. **[하드 API] `user_data`는 필드**(메서드 아님): `let ud = colliders[h1].user_data;` (괄호 없음). 플랜/설계의 `.user_data()`는 오기.
2. **[하드 API] 채널 = `rapier2d::crossbeam` 재수출, 2개 필요** (신규 크레이트 X):
   ```rust
   let (cs, cr) = rapier2d::crossbeam::channel::unbounded(); // collision
   let (fs, _fr) = rapier2d::crossbeam::channel::unbounded(); // contact-force (미사용도 필요)
   let ev = ChannelEventCollector::new(cs, fs);
   self.pipeline.step(&self.gravity, &self.params, /*...*/, Some(&mut self.query), &(), &ev);
   while let Ok(CollisionEvent::Started(h1, h2, _)) = cr.try_recv() { /* ... */ }
   ```
   `pipeline.step`의 **마지막 인자만** `&()`→`&ev` (뒤에서 2번째 physics-hooks `&()`는 유지).
3. **[HIGH 정확성] 태그 없는 콜라이더 함정**: 벽/공은 `user_data==0` → `(robot0,part0)`로 오독. robot-1이 벽 치면 "robot1 vs robot0"로 **오데미지**. → **decode-only `r1!=r2` 금지.** 로봇 부위 콜라이더를 **`HashSet<ColliderHandle>` 멤버십**으로 관리하거나 **비로봇=robot_idx 0, 로봇=1·2 sentinel offset**을 써서 "양쪽 다 로봇 부위"인 쌍만 처리. **wall-no-damage · self-part-no-damage 테스트 추가**(Task 4).
4. **[결정성] 이벤트 정렬**: 수집한 충돌을 `(rA,rB,pA,pB)`로 **정렬 후 데미지 적용**. 단일스레드라 방출 순서는 same-build 결정적이지만, 한 스텝 다중 히트 시 f32 비결합성으로 HP(→hash)가 순서 민감 → 정렬로 안정화(골든 리플레이 보호).
5. **[재베이스라인] 복합 콜라이더 = 질량/관성 변화**: 단일 큐보이드→부위별 자식 콜라이더로 바꾸면 기존 `replay` 골든 해시·`goal`/`robot_speed_capped` 테스트가 흔들릴 수 있음. **재베이스라인 + 재검증** 예상(Task 3·6).
6. **[설계 정합 명시]** impact = **상대 linvel 근사**(ADR-009의 접촉 임펄스 간소화; 진짜 임펄스는 `ContactForceEvent`=`CONTACT_FORCE_EVENTS`+threshold, 3c/튜닝). 부위별 **취약도** 항도 생략(3c). 이 2가지를 Task 4/문서에 "의도적 간소화"로 명기.
7. **[스냅샷 명명]** 이 플랜은 `parts: Vec<(String,f32)>`·snake_case `repair_in` — [02 §4](../../02-네트워크-프로토콜.md)의 object/`repairIn`(camelCase)과 다름. 기존 관례(`id:"Blue"` 등)와 동일 방향이라 3b 블로커 아님 — 와이어 스키마를 snake/tuple 관례로 [02] 갱신 시 명기.
8. **[커버리지]** Task별 테스트에 추가: self-part 무데미지, wall 무데미지, 다운 중 히트 재트리거 없음, 리페어가 **전체 부위** 복구.

---

## File Structure
- Create: `server/src/combat.rs` — `PartId`(로봇idx+Slot), `damage_on_contact(...)` 순수 함수, 부위 HP 상태 `CombatState`, 파손다운 판정·리페어 타이머
- Modify: `server/src/physics.rs` — 로봇당 **부위별 콜라이더**(복합) + `user_data` 태깅, 충돌 이벤트 수집, step에서 이벤트→combat 적용, 다운 중 입력 무시
- Modify: `server/src/world.rs` — `RobotState`에 `parts: Vec<(String,f32)>`(부위 HP비율) · `down: Down{broken,repair_in}` · `st: Vec<String>`(3b엔 `["downed"]`만)
- Modify: `server/src/net.rs` — 스냅샷 확장(위 필드), `event`에 `hit`/`broken`/`repair`(선택)
- Modify: `server/src/parts.rs` — 부위별 `hp`/`defense`/`attack`가 이미 `StatSet`에 있음 → 부위별 콜라이더에 매핑할 부위별 스탯 접근자 추가

---

## Task 1: 부위 스탯 접근 + 데미지 공식(순수)

**Files:** Create `server/src/combat.rs`; Modify `server/src/parts.rs`; `main.rs`(`mod combat;`)

- [ ] **Step 1: 실패하는 테스트 — 데미지 공식 불변식**

`server/src/combat.rs`:
```rust
use crate::parts::StatSet;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn damage_scales_with_impact_and_attack_over_defense() {
        // 공격력↑ 또는 impact↑ → 데미지↑, 방어력↑ → 데미지↓
        let atk = StatSet { attack: 10.0, ..Default::default() };
        let def_low = StatSet { defense: 2.0, ..Default::default() };
        let def_high = StatSet { defense: 8.0, ..Default::default() };
        let d_low = damage_on_contact(&atk, &def_low, 1.0);
        let d_high = damage_on_contact(&atk, &def_high, 1.0);
        let d_big = damage_on_contact(&atk, &def_low, 2.0);
        assert!(d_low > d_high, "방어 높으면 데미지 감소");
        assert!(d_big > d_low, "impact 크면 데미지 증가");
        assert!(d_low >= 0.0);
    }
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test combat` → FAIL. `main.rs`에 `mod combat;`.

- [ ] **Step 3: 최소 구현**
```rust
use crate::parts::StatSet;

/// 상호 데미지 한쪽 산출: impact × (공격합 / 방어합) × 계수. 결정적·비음수.
pub fn damage_on_contact(attacker: &StatSet, defender: &StatSet, impact: f32) -> f32 {
    const K: f32 = 1.0;
    let atk = attacker.attack.max(0.0);
    let def = (defender.defense.max(0.0)) + 1.0; // +1로 0방어 폭주 방지
    (impact.max(0.0) * (atk / def) * K).max(0.0)
}
```
> attack/defense는 **로봇 총합**(3b) — 부위별 세분화는 Plan 3c 여지. 여기선 "부딪힌 부위에 로봇 총 공/방 기반 데미지".

- [ ] **Step 4: 통과** — Run: `cargo test combat` → PASS.
- [ ] **Step 5: Commit** — `[KB-24] feat: combat damage formula (pure)`

---

## Task 2: 부위별 HP 상태 + 파손 다운 로직(순수)

**Files:** Modify `server/src/combat.rs`

- [ ] **Step 1: 실패하는 테스트**
```rust
#[test]
fn part_hp_depletes_and_triggers_down_then_repairs() {
    let mut cs = CombatState::new(&[40.0, 30.0]); // 2 부위
    assert!(!cs.broken());
    cs.apply_damage(0, 100.0);            // 부위0 과다 피해
    assert!(cs.broken(), "부위 HP 0 → 파손 다운");
    // 다운 지속 후 리페어
    for _ in 0..(cs.repair_ticks()) { cs.tick_down(); }
    assert!(!cs.broken(), "일정 시간 뒤 전체 리페어");
    assert!(cs.hp_ratio(0) > 0.99, "리페어 시 HP 복구");
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test combat` → FAIL.

- [ ] **Step 3: 구현** — `CombatState`:
```rust
pub struct CombatState { max: Vec<f32>, hp: Vec<f32>, down_timer: u32 }
const REPAIR_TICKS: u32 = 180; // 3초 @60Hz (튜닝)

impl CombatState {
    pub fn new(max_hp: &[f32]) -> Self {
        Self { max: max_hp.to_vec(), hp: max_hp.to_vec(), down_timer: 0 }
    }
    pub fn broken(&self) -> bool { self.down_timer > 0 }
    pub fn repair_ticks(&self) -> u32 { REPAIR_TICKS }
    pub fn hp_ratio(&self, i: usize) -> f32 { if self.max[i] > 0.0 { self.hp[i] / self.max[i] } else { 1.0 } }
    pub fn apply_damage(&mut self, part: usize, dmg: f32) {
        if self.broken() { return; }
        self.hp[part] = (self.hp[part] - dmg).max(0.0);
        if self.hp.iter().any(|&h| h <= 0.0) { self.down_timer = REPAIR_TICKS; }
    }
    /// 다운 중 매 tick 호출. 타이머 소진 시 전체 리페어.
    pub fn tick_down(&mut self) {
        if self.down_timer > 0 {
            self.down_timer -= 1;
            if self.down_timer == 0 { self.hp = self.max.clone(); }
        }
    }
}
```

- [ ] **Step 4: 통과** — Run: `cargo test combat` → PASS.
- [ ] **Step 5: Commit** — `[KB-25] feat: per-part HP + 파손 다운/리페어 (pure)`

---

## Task 3: 부위별 복합 콜라이더 + user_data 태깅

**Files:** Modify `server/src/physics.rs`

- [ ] **Step 1: 실패하는 테스트 — 로봇당 부위 콜라이더 다수**
```rust
#[test]
fn robot_has_multiple_tagged_part_colliders() {
    let w = PhysicsWorld::new_kickoff_with(/* 기존 인자 */);
    // 콜라이더 수: 벽(6) + 공(1) + 로봇2×부위N
    assert!(w.robot_part_count() >= 2, "로봇당 부위 콜라이더 ≥2");
}
```
> 정확한 부위 수/배치는 구현 세부. 최소 3부위(몸통·앞다리·뒷다리)로 시작 가능.

- [ ] **Step 2: 실패 확인** — Run: `cargo test physics` → FAIL.

- [ ] **Step 3: 구현** — 로봇 바디에 **부위별 자식 콜라이더** 부착(단일 큐보이드 교체). 각 콜라이더 `user_data`에 `(robot_idx, part_idx)` 인코딩:
```rust
// user_data(u128): 상위=robot_idx, 하위=part_idx
fn tag(robot: u32, part: u32) -> u128 { ((robot as u128) << 64) | (part as u128) }
fn untag(u: u128) -> (u32, u32) { ((u >> 64) as u32, u as u32) }
```
부위 콜라이더에 `.active_events(ActiveEvents::COLLISION_EVENTS).user_data(tag(i, p))`. 부위 배치(중심 오프셋)는 몸통/앞/뒤 정도로. `robot_part_count()` 헬퍼 추가.
> **드라이런 대상**: `ActiveEvents`/`user_data`/`insert_with_parent` 시그니처(0.26) 확인.

- [ ] **Step 4: 통과** — Run: `cargo test physics` → PASS.
- [ ] **Step 5: Commit** — `[KB-26] feat: per-part compound colliders + user_data tagging`

---

## Task 4: 충돌 이벤트 수집 → 상호 데미지 적용

**Files:** Modify `server/src/physics.rs`

- [ ] **Step 1: 실패하는 테스트 — 로봇끼리 충돌하면 양쪽 HP 감소**
```rust
#[test]
fn robots_colliding_take_mutual_damage() {
    // 두 로봇을 서로 마주보게 근접 배치 + 서로를 향해 돌진
    let mut w = /* 로봇을 가깝게 둔 테스트 월드 */;
    let before = (w.hp_ratio_min(0), w.hp_ratio_min(1));
    for _ in 0..120 { w.step(&[toward_each_other(); 2]); }
    let after = (w.hp_ratio_min(0), w.hp_ratio_min(1));
    assert!(after.0 < before.0 && after.1 < before.1, "충돌 시 양쪽 부위 HP 감소");
}
#[test]
fn ball_contact_does_no_damage() {
    // 로봇이 공만 밀 때 HP 불변
    let mut w = PhysicsWorld::new_kickoff_with(/*..*/);
    for _ in 0..300 { w.step(&[chase(); 2]); }
    assert!(w.hp_ratio_min(0) > 0.99, "공 접촉은 무데미지");
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test physics` → FAIL.

- [ ] **Step 3: 구현** — step에서 `ChannelEventCollector`로 충돌 이벤트 수집:
```rust
// step 내부:
let (col_send, col_recv) = crossbeam_channel::unbounded(); // 또는 rapier 재수출
let events = ChannelEventCollector::new(col_send, /* contact_force_send */);
self.pipeline.step(&self.gravity, &self.params, ..., &(), &events);
// 이벤트 처리:
while let Ok(ev) = col_recv.try_recv() {
    if let CollisionEvent::Started(h1, h2, _) = ev {
        // 두 콜라이더의 user_data → (robot,part). 서로 다른 로봇의 부위끼리면 상호 데미지.
        // impact = 두 바디 상대속도 크기(근사).
        // combat.apply_damage(robotA_part, dmg_a); combat.apply_damage(robotB_part, dmg_b);
    }
}
// 다운 타이머 tick, 다운 로봇은 apply_controls에서 입력 무시.
```
공↔로봇/공↔벽은 무시(둘 다 로봇 부위일 때만 데미지). 임팩트 근사: 접촉 시 두 로봇 바디 `linvel` 차의 크기. `hp_ratio_min(i)` 헬퍼 추가.
> **드라이런 대상**: `ChannelEventCollector`/`CollisionEvent`/채널 크레이트(`crossbeam` vs rapier 재수출) 확정.

- [ ] **Step 4: 통과** — Run: `cargo test physics` → PASS.
- [ ] **Step 5: Commit** — `[KB-27] feat: collision-event mutual damage (robot↔robot only)`

---

## Task 5: 파손 다운 → 입력 무시 + 스냅샷 디버프 필드

**Files:** Modify `server/src/physics.rs`, `server/src/world.rs`

- [ ] **Step 1: 실패하는 테스트 — 다운 중 입력 무시 + 스냅샷 반영**
```rust
#[test]
fn downed_robot_ignores_input_and_snapshot_shows_state() {
    let mut w = /* 로봇0을 강제 파손시킨 월드 (test helper) */;
    w.force_break_for_test(0);
    let s = w.snapshot();
    assert!(s.robots[0].down.broken, "스냅샷에 파손 다운 표시");
    assert!(s.robots[0].st.iter().any(|x| x == "downed"));
    // 다운 중 전진 입력 줘도 크게 안 움직임
    let p0 = w.snapshot().robots[0].pos.x;
    for _ in 0..30 { w.step(&[ControlOutput{thrust:1.0,turn:0.0}, ControlOutput::default()]); }
    assert!((w.snapshot().robots[0].pos.x - p0).abs() < 0.5);
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test` → FAIL.

- [ ] **Step 3: 구현**
- `world.rs`: `RobotState`에 `pub parts: Vec<(String, f32)>`(부위명·HP비율), `pub down: Down`, `pub st: Vec<String>`. `Down { pub broken: bool, pub repair_in: f32 }`(Serialize). `GameState::new_kickoff`도 기본값 채움(빈 st, broken=false).
- `physics.rs`: `apply_controls`에서 다운(broken)인 로봇은 입력 스킵. `snapshot()`에서 `CombatState`로부터 부위 HP비율·down 채움. `force_break_for_test`(cfg(test)) 헬퍼.

- [ ] **Step 4: 통과** — Run: `cargo test` → 전체 PASS, warning 0.
- [ ] **Step 5: Commit** — `[KB-28] feat: downed robots ignore input + snapshot debuff fields`

---

## Task 6: 결정성 회귀 + E2E + 문서/KANBAN

- [ ] **Step 1: 골든 리플레이 유지** — `replay::hash_state`에 부위 HP·다운을 포함(전투도 결정적 회귀 검증). Run: `cargo test replay` → 동일입력 동일해시.
- [ ] **Step 2: 전체 테스트** — Run: `cargo test` → PASS, warning 0(debug+release).
- [ ] **Step 3: E2E** — `cargo run` + curl: 두 로봇이 부딪히면 스냅샷 `parts` HP가 줄고, 0이면 `down.broken=true` 후 리페어되는지 관찰(포트 8090, 서버 정리).
- [ ] **Step 4: 문서** — [00 §12]/[02 §4] 스냅샷 필드 반영, [KANBAN] Plan 3b 카드 Done.
- [ ] **Step 5: Commit** — `[KB-29] docs: combat/damage (3b) done, kanban update`

---

## Self-Review 결과
- **스펙 커버리지**: 부위 콜라이더 ✅, 충돌 이벤트 상호 데미지([00 §12]) ✅, 부위 HP·파손다운·리페어 ✅, 공 무데미지 ✅, 스냅샷 디버프 필드 ✅. **효과 선택(넉백/스턴)=Plan 3c**, 부위별 공/방 세분화도 3c 여지.
- **플레이스홀더 주의**: Task 3·4의 rapier 이벤트 배관(채널 크레이트·시그니처)은 **착수 전 드라이런 컴파일 프로브로 확정**(플레이스홀더 아님, 외부 라이브러리 통합 정상 절차). 순수 로직(combat.rs)은 완전 코드.
- **타입 일관성**: `CombatState`/`damage_on_contact`/`PhysicsWorld` 확장/`RobotState`(parts/down/st) 명칭 태스크 간 일치. `RobotState`는 이미 `Copy` 없음(3a) → Vec/String 필드 추가 자유.
- **결정성 리스크**: **충돌 이벤트 순서**가 비결정적이면 데미지 합이 흔들림 → 이벤트를 `(robotA,robotB,partA,partB)` 키로 **정렬 후 처리**해 결정성 확보(드라이런/구현 시 확인). 채널 수신 순서 의존 금지.
- **큰 리스크**: 부위 콜라이더가 서로(같은 로봇 내) 또는 공과 이벤트를 다수 유발 → 필터링(다른 로봇의 부위쌍만) 필수.

**다음 Plan(3c): 효과 선택(넉백/스턴)** — 부위 effect 프로필 × impact × 저항으로 넉백/스턴/데미지 중첩 선택([00 §12], [07 ADR-009]).
