# Parts / Loadout / Stats — Plan 3a

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development (or executing-plans). 체크박스(`- [ ]`) 추적.

**Goal:** 로봇을 파츠(7슬롯) 조립체로 만들고, **로드아웃별 스탯이 물리에 다르게 반영**되게 한다. 파츠 카탈로그 + 프리셋 2종(비대칭)을 두고, `join`이 로드아웃을 싣고 `catalog`/스냅샷에 반영한다. 결과: 스탯이 다른 두 로봇이 서로 다르게 움직여 **비대칭 플레이**가 성립(라이브 골 관찰 완화).

**Architecture:** 파츠·스탯은 **데이터 주도 순수 로직**(`parts.rs`), 물리(`physics.rs`)는 로봇별 `StatSet`를 받아 `apply_controls`/바디 질량에 사용. `Controller`/`tick`/결정성 경계 불변. 전투(데미지/효과/HP)는 **Plan 3b**, 커스터마이즈 UI는 **Plan 5(게임 흐름)** — 여기선 서버가 프리셋을 슬롯에 배정.

**Tech Stack:** 기존 rapier2d 0.26 / tokio / axum / serde. 설계: [00 §6 파츠](../../00-개요-및-게임설계.md), [02 §4.2 카탈로그/§4.3 로드아웃](../../02-네트워크-프로토콜.md).

> **범위 경계(중요):** 이 Plan은 **이동 스탯만** 물리에 반영(`maxSpeed/accel/turnRate/mass`). `kickPower/attack/defense/hp/effect`는 `StatSet`에 **정의만** 하고 사용은 Plan 3b/4. 클라는 현행 박스 렌더 유지(파츠 시각화는 Plan 5).

## ⚠️ 착수 전 필수 반영 (드라이런 점검 — 이 목록이 태스크 코드보다 우선)

검증됨: borrow-check 컴파일 OK(disjoint 필드), rapier `additional_mass`(0.26) 존재. 착수 즉시 반영:

1. **[치명] `RobotState`에서 `Copy` 제거.** `world.rs`의 `#[derive(Clone, Copy, Serialize)]`에 `pub robot: String`을 넣으면 Copy가 깨져 컴파일 실패 → **`#[derive(Clone, Serialize)]`**. (Copy 의존 사용처 없음 확인; `GameState`는 Clone만 필요.)
2. **[높음] `default_stats()`에 유효값.** `StatSet::default().max_speed==0`이면 Task 2의 maxSpeed 클램프가 **기본 로봇을 정지**시켜 기존 물리/골/tick 테스트가 붕괴. `default_stats()`(→ `parts.rs`에 정의)를 `max_speed=10.0, accel=6.0, turn_rate=3.0, mass=0.0`(=기존 THRUST/TURN_RATE 등가, mass는 가산이라 0=no-op로 기존 거동 보존)로 둔다. `new_kickoff()`는 이 값으로 `new_kickoff_with`에 위임.
3. **robot id 배선.** `new_kickoff_with([StatSet;2])`만으로는 프리셋 이름을 소실해 스냅샷 `robot` id를 못 채운다. → 시그니처를 **`new_kickoff_with(stats: [StatSet;2], preset_ids: [String;2])`** 로 확장(테스트 호출부도 갱신)하거나 `set_preset_ids(&mut self,[String;2])` 세터 추가. 안 하면 `robot`이 항상 빈 문자열.
4. **`welcome` 메시지 없음.** 서버에 `welcome` 없음 → "welcome 다음에" 대신 **`net.rs::push_state`의 틱 루프 진입 전 `catalog_msg()` 1회 전송**. `catalog()`는 순수 함수라 핸들러에서 직접 호출 가능.
5. **`aggregate` 시그니처 = `(&Catalog, &str)`** (아래 File Structure의 `(&Loadout)`는 오기 — 2인자형으로 통일).
6. **`catalog_msg` DTO/JSON**: 스탯은 [02 §4.2](../../02-네트워크-프로토콜.md)대로 `stats` 중첩 객체로 낸다. 3a 스냅샷은 `robot: presetId`(문자열)만, `loadout` 객체는 Plan 5. [02 §4] 갱신 시 이 경계 명기.
7. `additional_mass`는 콜라이더 밀도 유래 질량에 **가산**(대체 아님) — mass=0=no-op.

---

## File Structure
- Create: `server/src/parts.rs` — `StatSet`, `Part`, `Slot`, `Loadout`, `catalog()`(파츠+프리셋), `aggregate(&Catalog, &str)->StatSet`, `default_stats()->StatSet`
- Modify: `server/src/physics.rs` — `PhysicsWorld::new_kickoff_with(loadouts)`, 로봇별 `StatSet` 보유, `apply_controls`가 스탯 사용, maxSpeed 클램프
- Modify: `server/src/world.rs` — 스냅샷 `RobotState`에 `robot: String`(로드아웃/프리셋 id) 추가
- Modify: `server/src/net.rs` — `catalog` 다운링크 메시지, 스냅샷에 robot id
- Modify: `server/src/main.rs` — Blue/Red에 서로 다른 프리셋 배정
- (`join` 로드아웃 수신은 여기선 프리셋 배정으로 대체; 클라 선택 UI는 Plan 5)

---

## Task 1: 파츠·스탯 데이터 모델 + 카탈로그 + 집계

**Files:** Create `server/src/parts.rs`; Modify `server/src/main.rs`(`mod parts;`)

- [ ] **Step 1: 실패하는 테스트**

`server/src/parts.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_aggregates_part_stats_and_presets_differ() {
        let cat = catalog();
        let striker = aggregate(&cat, "striker");
        let guard = aggregate(&cat, "guard");
        // 집계는 부위 기여 합
        assert!(striker.max_speed > 0.0 && striker.accel > 0.0);
        // 프리셋이 서로 다르다(비대칭)
        assert!(striker.max_speed != guard.max_speed || striker.accel != guard.accel);
    }
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test --manifest-path server/Cargo.toml parts` → FAIL(미정의). `main.rs`에 `mod parts;` 추가.

- [ ] **Step 3: 최소 구현**

`parts.rs` 상단:
```rust
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default)]
pub struct StatSet {
    pub max_speed: f32,
    pub accel: f32,
    pub turn_rate: f32,
    pub mass: f32,
    // 정의만(Plan 3b/4에서 사용):
    pub kick_power: f32,
    pub attack: f32,
    pub defense: f32,
    pub hp: f32,
}

impl StatSet {
    fn add(&mut self, o: &StatSet) {
        self.max_speed += o.max_speed; self.accel += o.accel;
        self.turn_rate += o.turn_rate; self.mass += o.mass;
        self.kick_power += o.kick_power; self.attack += o.attack;
        self.defense += o.defense; self.hp += o.hp;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Slot { Head, Neck, Body, ForelegL, ForelegR, HindlegL, HindlegR, Tail }

#[derive(Clone)]
pub struct Part { pub id: &'static str, pub slot: Slot, pub stats: StatSet }

pub struct Loadout { pub parts: Vec<&'static str> } // 파츠 id 목록

pub struct Catalog { pub parts: HashMap<&'static str, Part>, pub presets: HashMap<&'static str, Loadout> }

/// 데이터 주도 카탈로그(개발자 배포). 값은 밸런싱 대상 초기값.
pub fn catalog() -> Catalog {
    let mut parts = HashMap::new();
    let mut add = |id, slot, s: StatSet| { parts.insert(id, Part { id, slot, stats: s }); };
    // 공용 부위(간략): 몸통/다리/머리 계열
    add("body-std",   Slot::Body,     StatSet { mass: 1.0, hp: 40.0, defense: 6.0, ..Default::default() });
    add("body-light", Slot::Body,     StatSet { mass: 0.7, hp: 30.0, defense: 4.0, ..Default::default() });
    add("hind-speed", Slot::HindlegL, StatSet { max_speed: 3.5, accel: 5.0, ..Default::default() });
    add("hind-power", Slot::HindlegL, StatSet { max_speed: 2.6, accel: 6.5, ..Default::default() });
    add("fore-std",   Slot::ForelegL, StatSet { accel: 2.0, attack: 5.0, ..Default::default() });
    add("neck-std",   Slot::Neck,     StatSet { turn_rate: 3.0, ..Default::default() });
    add("head-std",   Slot::Head,     StatSet { ..Default::default() });
    add("tail-std",   Slot::Tail,     StatSet { ..Default::default() });

    let mut presets = HashMap::new();
    presets.insert("striker", Loadout { parts: vec![
        "head-std","neck-std","body-light","fore-std","hind-speed","tail-std"] });
    presets.insert("guard", Loadout { parts: vec![
        "head-std","neck-std","body-std","fore-std","hind-power","tail-std"] });

    Catalog { parts, presets }
}

/// 프리셋 id의 총 스탯 = 부위 기여 합.
pub fn aggregate(cat: &Catalog, preset: &str) -> StatSet {
    let mut s = StatSet::default();
    if let Some(lo) = cat.presets.get(preset) {
        for pid in &lo.parts {
            if let Some(p) = cat.parts.get(pid) { s.add(&p.stats); }
        }
    }
    s
}
```
> `HashMap` 이터레이션은 **집계(합산)에만** 쓰여 순서 무관 → 결정성 안전. sim 경로에서 HashMap 순회 없음.

- [ ] **Step 4: 통과 확인** — Run: `cargo test parts` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: parts/stats catalog + loadout aggregate [KB-18]"`

---

## Task 2: 물리에 로봇별 스탯 반영

**Files:** Modify `server/src/physics.rs`

- [ ] **Step 1: 실패하는 테스트 — 가속 큰 로봇이 더 멀리 간다(불변식)**

`physics.rs` tests에 추가:
```rust
#[test]
fn higher_accel_robot_travels_farther() {
    use crate::parts::{catalog, aggregate};
    let cat = catalog();
    // 두 로봇에 서로 다른 스탯: robot0=guard(accel↑), robot1=striker
    let mut w = PhysicsWorld::new_kickoff_with([aggregate(&cat, "guard"), aggregate(&cat, "striker")]);
    let fwd = [ControlOutput { thrust: 1.0, turn: 0.0 }, ControlOutput { thrust: 1.0, turn: 0.0 }];
    let x0 = w.snapshot().robots.iter().map(|r| r.pos.x).collect::<Vec<_>>();
    for _ in 0..60 { w.step(&fwd); }
    let s = w.snapshot();
    let d0 = (s.robots[0].pos.x - x0[0]).abs();
    let d1 = (s.robots[1].pos.x - x0[1]).abs();
    assert!(d0 != d1, "스탯이 다르면 이동 거리가 달라야 함");
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test physics` → FAIL(`new_kickoff_with` 없음).

- [ ] **Step 3: 구현**

`physics.rs`:
- 구조체에 `stats: Vec<StatSet>` 필드 추가(로봇 index 대응).
- `new_kickoff()` 는 기본 스탯으로 위임: `Self::new_kickoff_with([default_stats(), default_stats()])`. `default_stats()`는 기존 하드코딩과 동등한 값(THRUST/TURN_RATE→accel/turn_rate, maxSpeed).
- `new_kickoff_with(stats: [StatSet; 2])`: 기존 월드 생성 + 로봇 바디 `additional_mass(stats[i].mass)` 반영, `self.stats = stats.to_vec()`.
- `apply_controls`를 메서드로 바꿔 스탯 사용:
```rust
fn apply_controls(&mut self, controls: &[ControlOutput]) {
    for (i, (h, c)) in self.robots.iter().zip(controls.iter()).enumerate() {
        let st = &self.stats[i];
        let rb = &mut self.bodies[*h];
        rb.set_angvel(c.turn * st.turn_rate, true);
        let angle = rb.rotation().angle();
        let dir = vector![angle.cos(), angle.sin()];
        rb.apply_impulse(dir * (c.thrust * st.accel * DT), true);
        // maxSpeed 클램프
        let v = rb.linvel();
        let sp = (v.x * v.x + v.y * v.y).sqrt();
        if sp > st.max_speed && sp > 0.0 {
            let k = st.max_speed / sp;
            rb.set_linvel(vector![v.x * k, v.y * k], true);
        }
    }
}
```
`step`에서 `apply_controls(&mut self.bodies, ...)` 자유함수 호출을 `self.apply_controls(controls)`로 교체(빌림 주의: step 내에서 self 메서드 호출 순서 조정).

- [ ] **Step 4: 통과 확인** — Run: `cargo test physics` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: per-robot stats drive movement (accel/turn/maxSpeed/mass) [KB-19]"`

---

## Task 3: maxSpeed 클램프 검증

**Files:** Modify `server/src/physics.rs`

- [ ] **Step 1: 실패하는 테스트**
```rust
#[test]
fn robot_speed_capped_by_max_speed() {
    use crate::parts::StatSet;
    let slow = StatSet { max_speed: 1.0, accel: 10.0, turn_rate: 1.0, mass: 1.0, ..Default::default() };
    let mut w = PhysicsWorld::new_kickoff_with([slow, slow]);
    let fwd = [ControlOutput { thrust: 1.0, turn: 0.0 }; 2];
    for _ in 0..120 { w.step(&fwd); }
    let v = w.snapshot().robots[0].vel;
    let sp = (v.x*v.x + v.y*v.y).sqrt();
    assert!(sp <= 1.05, "속도는 max_speed 근처로 제한되어야 함 (got {sp})");
}
```
- [ ] **Step 2: 실패/통과** — Task 2에서 클램프 구현됨 → 통합 검증. FAIL이면 클램프 수정.
- [ ] **Step 3: (필요 시 조정)** — 클램프가 impulse 이후 적용되는지 확인.
- [ ] **Step 4: 통과 확인** — Run: `cargo test physics` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "test: max_speed clamp invariant [KB-20]"`

---

## Task 4: 프로토콜 — catalog 다운링크 + 스냅샷 robot id

**Files:** Modify `server/src/world.rs`, `server/src/net.rs`

- [ ] **Step 1: 실패하는 테스트 — catalog 직렬화 + 스냅샷 robot id**

`net.rs` tests에 추가:
```rust
#[test]
fn catalog_msg_serializes_parts_and_presets() {
    let j = serde_json::to_string(&catalog_msg()).unwrap();
    assert!(j.contains("\"t\":\"catalog\""));
    assert!(j.contains("striker"));
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test net` → FAIL.

- [ ] **Step 3: 구현**
- `world.rs`: `RobotState`에 `pub robot: String` 추가(프리셋 id). `snapshot()`에서 각 로봇의 프리셋 id를 채움(physics가 로봇별 preset 이름 보유하도록 `Vec<String>` 필드 추가, `new_kickoff_with`에 id 전달 or 별도 세터).
- `net.rs`: `catalog_msg()` — parts(id/slot/스탯)·presets를 직렬화 가능한 형태로. `#[derive(Serialize)]` DTO.
```rust
#[derive(Serialize)]
pub struct CatalogMsg { pub t: &'static str, pub presets: Vec<String>, pub parts: Vec<PartDto> }
#[derive(Serialize)]
pub struct PartDto { pub id: String, pub slot: String, /* 스탯 필드 */ }
pub fn catalog_msg() -> CatalogMsg { /* parts.rs catalog()에서 변환 */ }
```
접속 시 `welcome` 다음에 `catalog_msg()`를 1회 전송(net serve의 세션 시작부).
- 클라 계약 유지: 스냅샷 robot에 `robot` 필드가 늘 뿐, 기존 `id/pos/rot` 불변([02 §4](../../02-네트워크-프로토콜.md)).

- [ ] **Step 4: 통과 확인** — Run: `cargo test` → 전체 PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: catalog downlink + robot preset id in snapshot [KB-21]"`

---

## Task 5: main에 비대칭 프리셋 배정 + 헤드리스 검증

**Files:** Modify `server/src/main.rs`; Modify `server/src/replay.rs`(헤드리스에 스탯 반영)

- [ ] **Step 1: 실패하는 테스트 — 서로 다른 프리셋이면 상태가 갈린다**

`replay.rs` tests(또는 physics)에 추가:
```rust
#[test]
fn asymmetric_presets_diverge_from_symmetric() {
    use crate::parts::{catalog, aggregate};
    let cat = catalog();
    let asym = crate::replay::run_headless_with(
        [aggregate(&cat,"striker"), aggregate(&cat,"guard")], 300);
    let sym = crate::replay::run_headless_with(
        [aggregate(&cat,"striker"), aggregate(&cat,"striker")], 300);
    assert_ne!(asym, sym, "비대칭 로드아웃은 대칭과 다른 경기 전개");
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test` → FAIL(`run_headless_with` 없음).

- [ ] **Step 3: 구현**
- `replay.rs`: `run_headless_with(stats: [StatSet;2], steps) -> u64` 추가(기존 `run_headless`는 default 스탯 위임).
- `main.rs`: 카탈로그에서 `striker`/`guard`를 Blue/Red에 배정해 `PhysicsWorld::new_kickoff_with([...])` 로 시작.

- [ ] **Step 4: 통과 확인 + 수동** — Run: `cargo test` → PASS. `cargo run` 후 curl로 두 로봇의 이동이 다른지 관찰(서버 정리 잊지 말 것, 포트 8090).
- [ ] **Step 5: Commit** — `git commit -m "feat: asymmetric presets (striker vs guard) in main [KB-22]"`

---

## Task 6: 검증 + 문서/KANBAN

- [ ] **Step 1: 전체 테스트** — Run: `cargo test --manifest-path server/Cargo.toml` → 전부 PASS, warning 0.
- [ ] **Step 2: 수동** — `cargo run` + curl: 비대칭 로봇 이동 확인.
- [ ] **Step 3: 문서** — [KANBAN](../../../KANBAN.md): Plan 3a 카드 Done. [02 §4.2]에 catalog 실제 필드 반영(스탯 포함).
- [ ] **Step 4: Commit** — `git commit -m "docs: parts/loadout (3a) done, kanban update [KB-23]"`

---

## Self-Review 결과
- **스펙 커버리지**: 파츠/스탯 데이터 모델([00 §6]) ✅, 로드아웃 집계 ✅, 물리에 스탯 반영(이동) ✅, catalog 다운링크([02 §4.2]) ✅, 비대칭 프리셋 ✅. **전투 속성(attack/defense/hp/effect)은 정의만** → Plan 3b. 커스터마이즈 UI → Plan 5.
- **플레이스홀더 없음**: 코드 제공.
- **타입 일관성**: `StatSet`/`Catalog`/`aggregate`/`new_kickoff_with([StatSet;2])`/`run_headless_with` 명칭 태스크 간 일치. `RobotState.robot: String` 추가가 스냅샷·클라 계약에 additive(기존 필드 불변).
- **결정성**: HashMap은 집계에만(sim 경로 무순회). `new_kickoff`는 default 스탯로 위임해 기존 리플레이 해시 테스트 유지(단, 값이 바뀌면 same-hash 테스트는 여전히 자기 일관적).
- **리스크**: `apply_controls` 자유함수→메서드 전환 시 borrow 충돌(스텝 내 self 다중 차용) 주의 — 입력 적용을 물리 step 전에 분리.

**다음 Plan(3b): 전투/데미지** — 복합 콜라이더(부위별)·충돌 이벤트·상호 데미지(attack/defense)·부위 HP·파손 다운·넉백/스턴.
