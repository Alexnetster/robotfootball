# Combat Effects (넉백/스턴/데미지 선택) — Plan 3c

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. 체크박스(`- [ ]`) 추적.

**Goal:** 로봇↔로봇 충돌이 **데미지만** 주던 것을, 부위 **effect 프로필**에 따라 **넉백/스턴/데미지를 선택·중첩**해 적용하도록 확장한다. 선택은 **결정적**, **임팩트 클수록 중첩**(약한 접촉=데미지만, 강한 태클=넉백+스턴+데미지), 세기 = `effect프로필 × impact × 피격부위 저항`. ([00 §12](../../00-개요-및-게임설계.md), [07 ADR-009](../../07-결정기록-ADR.md))

**Architecture:** 효과 선택·세기 = **결정적 순수 함수**(`combat.rs` 확장). 적용은 3b의 충돌 이벤트 루프(`physics.rs`)에서 — 넉백=임펄스(물리), 스턴=상태(입력 무시), 데미지=기존 HP. `Controller`/`tick`/결정성 경계 불변. 3b(부위 콜라이더·이벤트·상호 데미지·파손다운) 위에 얹음.

**Tech Stack:** 새 rapier API 없음(3b 이벤트 재사용). 기존 스택.

> **선행**: Plan 3b 완료(부위 콜라이더 + `part_map` 멤버십 필터 + 충돌 이벤트 상호 데미지 + `CombatState` + 스냅샷 `parts/down/st`). 3c는 여기에 **효과 선택**을 추가.

---

## 착수 전 확인
- 현재 `StatSet`(parts.rs)에 **`effect` 프로필 없음**(3a는 max_speed/accel/turn_rate/mass/kick_power/attack/defense/hp만). → Task 1에서 추가.
- 스턴은 3b의 **파손 다운과 유사한 "입력 무시" 상태**지만 **짧고, HP 0 아님**. 별도 타이머.

---

## Task 1: 부위 effect 프로필 + 효과 선택(순수)

**Files:** Modify `server/src/parts.rs`(`StatSet`에 effect), `server/src/combat.rs`(선택 함수)

- [ ] **Step 1: 실패하는 테스트 — 임팩트 비례 중첩 + 세기 불변식**

`combat.rs` tests에 추가:
```rust
#[test]
fn effects_stack_with_impact_and_scale_by_profile_and_resistance() {
    // 프로필: 넉백0.6 스턴0.3 데미지0.5
    let prof = EffectProfile { knockback: 0.6, stun: 0.3, damage: 0.5 };
    let weak = resolve_effects(&prof, 0.2, 1.0);   // 약한 접촉
    let hard = resolve_effects(&prof, 2.0, 1.0);   // 강한 태클
    // 약한 접촉: 데미지만(넉백/스턴 임계 미달)
    assert!(weak.damage > 0.0 && weak.knockback == 0.0 && weak.stun == 0.0);
    // 강한 태클: 셋 다 발동, 데미지도 더 큼
    assert!(hard.damage > weak.damage && hard.knockback > 0.0 && hard.stun > 0.0);
    // 저항↑ → 효과↓
    let resisted = resolve_effects(&prof, 2.0, 4.0);
    assert!(resisted.damage < hard.damage);
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test combat` → FAIL.

- [ ] **Step 3: 구현**

`parts.rs` `StatSet`에 필드 추가:
```rust
// StatSet에 추가 (기본 0.0)
pub kb_w: f32,   // 넉백 성향
pub stun_w: f32, // 스턴 성향
pub dmg_w: f32,  // 데미지 성향
```
(`add()` 합산에도 3개 추가. 기존 attack/defense/hp 유지.)

`combat.rs`:
```rust
#[derive(Clone, Copy)]
pub struct EffectProfile { pub knockback: f32, pub stun: f32, pub damage: f32 }

#[derive(Clone, Copy, Default)]
pub struct Effects { pub knockback: f32, pub stun: f32, pub damage: f32 }

// 임팩트 임계(튜닝): 이 이상이어야 해당 효과 발동
const T_KNOCK: f32 = 0.8;
const T_STUN: f32 = 1.5;

/// 결정적. 임팩트 비례 중첩 + 프로필/저항 스케일.
pub fn resolve_effects(p: &EffectProfile, impact: f32, resistance: f32) -> Effects {
    let i = impact.max(0.0);
    let r = resistance.max(1.0); // +하한으로 폭주 방지
    let mut e = Effects::default();
    e.damage = p.damage * i / r;                    // 데미지는 항상(임팩트 비례)
    if i >= T_KNOCK { e.knockback = p.knockback * i / r; }
    if i >= T_STUN  { e.stun = p.stun * i / r; }
    e
}
```

- [ ] **Step 4: 통과** — Run: `cargo test combat` → PASS.
- [ ] **Step 5: Commit** — `[KB-30] feat: effect profile + impact-scaled effect selection (pure)`

---

## Task 2: 스턴 상태 (짧은 입력 무시, 순수 타이머)

**Files:** Modify `server/src/combat.rs`(스턴 타이머를 `CombatState`에)

- [ ] **Step 1: 실패하는 테스트**
```rust
#[test]
fn stun_blocks_input_for_duration_then_clears() {
    let mut cs = CombatState::new(&[40.0]);
    assert!(!cs.stunned());
    cs.apply_stun(0.5); // 0.5초
    assert!(cs.stunned());
    // dt 진행하면 언젠가 해제
    let steps = (0.5 / crate::world::DT).ceil() as u32 + 1;
    for _ in 0..steps { cs.tick_status(); }
    assert!(!cs.stunned());
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test combat` → FAIL.

- [ ] **Step 3: 구현** — `CombatState`에 `stun_timer: f32` 추가:
```rust
// impl CombatState
pub fn stunned(&self) -> bool { self.stun_timer > 0.0 }
pub fn apply_stun(&mut self, secs: f32) { self.stun_timer = self.stun_timer.max(secs); } // 갱신(더 길면)
/// 매 tick: 스턴·(기존 다운) 타이머 감소. 기존 tick_down과 통합 or 병행.
pub fn tick_status(&mut self) {
    if self.stun_timer > 0.0 { self.stun_timer = (self.stun_timer - crate::world::DT).max(0.0); }
    self.tick_down(); // 기존 다운 타이머
}
```
> 기존 `tick_down` 호출부를 `tick_status`로 교체(스턴+다운 함께 진행).

- [ ] **Step 4: 통과** — Run: `cargo test combat` → PASS.
- [ ] **Step 5: Commit** — `[KB-31] feat: stun timer in CombatState (pure)`

---

## Task 3: 충돌 시 효과 적용 (넉백=임펄스 / 스턴=상태 / 데미지=기존)

**Files:** Modify `server/src/physics.rs`(3b 충돌 이벤트 루프)

- [ ] **Step 1: 실패하는 테스트 — 강한 충돌은 넉백+스턴 유발**
```rust
#[test]
fn strong_collision_applies_knockback_and_stun() {
    // 두 로봇을 서로 강하게 충돌시키는 테스트 월드(3b 헬퍼 활용)
    let mut w = /* 강한 상호 돌진 세팅 */;
    let before_speed = /* 피격 로봇 속도 */;
    let mut stunned_seen = false;
    for _ in 0..120 {
        w.step(&[toward_each_other(); 2]);
        if w.is_stunned_for_test(1) { stunned_seen = true; }
    }
    // 넉백으로 속도가 튐 + 스턴 관측
    assert!(stunned_seen, "강한 충돌은 스턴을 유발");
    // (넉백은 속도/위치 변화로 간접 확인)
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test physics` → FAIL.

- [ ] **Step 3: 구현** — 3b의 충돌 처리부(정렬된 이벤트 루프)에서 `apply_damage` 대신 **효과 전체 적용**:
```rust
// 각 (공격 로봇 a, 피격 로봇 b, 피격 부위 pb) 에 대해:
let profile = self.effect_profile(a);          // a의 부위/총합 effect 프로필
let resistance = self.defense_of(b);           // b 방어(부위 취약도는 Task 5)
let impact = relative_speed;                   // 3b와 동일 근사
let eff = resolve_effects(&profile, impact, resistance);
// 데미지
self.combat[b].apply_damage(pb, eff.damage);
// 스턴
if eff.stun > 0.0 { self.combat[b].apply_stun(eff.stun); }
// 넉백: b 바디에 a→b 방향 임펄스
if eff.knockback > 0.0 {
    let dir = (pos_b - pos_a).normalize();
    self.bodies[self.robots[b]].apply_impulse(dir * eff.knockback, true);
}
```
`apply_controls`에서 **스턴 중인 로봇도 입력 무시**(다운과 동일 처리): `if self.combat[i].broken() || self.combat[i].stunned() { continue; }`. `is_stunned_for_test`(cfg(test)) 헬퍼 추가.
> 스턴/다운 중에도 넉백 임펄스는 받음(물리 몸체 유지) — 입력만 차단.

- [ ] **Step 4: 통과** — Run: `cargo test physics` → PASS.
- [ ] **Step 5: Commit** — `[KB-32] feat: apply knockback/stun/damage on collision`

---

## Task 4: 스냅샷 `st` 확장 + `event` hit 효과

**Files:** Modify `server/src/physics.rs`(snapshot), `server/src/world.rs`(필요 시)

- [ ] **Step 1: 실패하는 테스트 — 스턴이 스냅샷 st에 반영**
```rust
#[test]
fn snapshot_st_shows_stun() {
    let mut w = /* 로봇1을 강제 스턴 (test helper force_stun_for_test) */;
    w.force_stun_for_test(1, 0.5);
    let s = w.snapshot();
    assert!(s.robots[1].st.iter().any(|x| x == "stun"));
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test` → FAIL.

- [ ] **Step 3: 구현** — `snapshot()`에서 `st` 채울 때: 다운이면 `"downed"`, 스턴이면 `"stun"` 추가(둘 다 가능). `force_stun_for_test`(cfg(test)) 추가.
- (선택) `event`에 `hit`의 `effects` 필드(넉백/스턴/데미지 발동 여부)를 넣어 클라 이펙트용으로. 없으면 스냅샷 st만으로도 충분(YAGNI).

- [ ] **Step 4: 통과** — Run: `cargo test` → 전체 PASS, warning 0.
- [ ] **Step 5: Commit** — `[KB-33] feat: stun in snapshot st (+optional hit event effects)`

---

## Task 5: (선택) 부위별 취약도

**Files:** Modify `server/src/combat.rs`, `server/src/physics.rs`

- [ ] **Step 1: 실패하는 테스트** — 같은 임팩트라도 **취약 부위**(예: 머리) 피격이 데미지↑.
- [ ] **Step 2~4**: `resolve_effects`에 부위 취약도 계수 추가(피격 부위 index→취약도). 머리>몸통>다리 등. ADR-009의 "피격부위 취약도" 항 완성.
- [ ] **Step 5: Commit** — `[KB-34] feat: per-part vulnerability multiplier`
> 밸런스 복잡도↑ — 여유 없으면 스킵하고 로봇 총 방어만 유지(3b 수준). 그땐 KB-34 생략.

---

## Task 6: 결정성 회귀 + E2E + 문서/KANBAN

- [ ] **Step 1: 골든 리플레이** — `hash_state`에 스턴 타이머(또는 st) 포함 여부 결정. 동일입력 동일해시 유지. Run: `cargo test replay`.
- [ ] **Step 2: 전체 테스트** — Run: `cargo test` → PASS, warning 0(debug+release).
- [ ] **Step 3: E2E** — `cargo run` + curl: 강한 충돌 시 스냅샷 `st`에 `"stun"`이 뜨고 넉백으로 위치가 튀는지 관찰(대칭 AI라 충돌 자체가 드묾 → 강제 세팅/비대칭 프리셋로 확인, 포트 8090, 서버 정리).
- [ ] **Step 4: 문서** — [00 §12] 효과 선택 반영, [KANBAN] Plan 3c 카드 Done.
- [ ] **Step 5: Commit** — `[KB-35] docs: combat effects (3c) done, kanban update`

---

## Self-Review 결과
- **스펙 커버리지**: effect 프로필([00 §6]) ✅, 결정적 임팩트 비례 중첩·곱셈 세기([07 ADR-009]) ✅, 넉백(임펄스)·스턴(입력차단)·데미지(HP) ✅, 스냅샷 st ✅. 부위별 취약도(Task 5)로 ADR-009 공식 완성(선택).
- **결정성**: `resolve_effects`·스턴 타이머 = 순수. 넉백 임펄스는 3b 정렬된 이벤트 순서 위에서 적용 → 결정적 유지. hash에 스턴 반영 시 회귀 안정.
- **타입 일관성**: `EffectProfile`/`Effects`/`resolve_effects`/`CombatState.stun_timer`/`apply_stun`/`tick_status` 명칭 태스크 간 일치. `StatSet`에 `kb_w/stun_w/dmg_w` 추가는 additive.
- **리스크**: 스턴/다운 이중 상태 → `apply_controls` 스킵 조건에 **둘 다** 포함 확인. 넉백이 대칭 AI 평형을 깰 수도(오히려 라이브 충돌 관찰에 도움).

**다음 Plan(4): 제어 모드/입력** — 직접(키보드)·전략(마우스)·런타임 전환. 사람 조작이 들어오면 비대칭 충돌이 흔해져 전투 효과가 라이브로 잘 보임.
