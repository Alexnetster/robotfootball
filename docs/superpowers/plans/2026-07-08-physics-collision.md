# Physics / Collision (rapier2d) Implementation Plan — Plan 2

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`) syntax.

**Goal:** 걷는 뼈대의 운동학 적분을 **rapier2d 물리**로 교체한다. 로봇↔공 밀기 드리블, 벽 반사, 골 센서가 동작해 **공이 실제로 움직이고 라이브 골이 난다**. 라이브 루프를 고정스텝 누산기로 바꾸고, `rot`을 물리 바디에서 읽어 무한 누적을 없앤다.

**Architecture:** `sim.rs`의 순수 `step`를 rapier `PhysicsWorld`를 소유·전진시키는 형태로 바꾸되, **결정성·I/O 분리 유지**: 물리 스텝은 `(입력)→(상태)` 결정적 함수, 소켓/시간은 그대로 net/main. `Controller`/`tick` 인터페이스는 불변. 스냅샷(pos/rot/vel)은 rapier 바디에서 추출.

**Tech Stack:** rapier2d(핀 고정), nalgebra(rapier 재수출). 기존 tokio/axum/serde. 관련: [00 §12 전투는 Plan 3](../../00-개요-및-게임설계.md), [02 §4.4 월드 상수](../../02-네트워크-프로토콜.md), [07 ADR-002/007](../../07-결정기록-ADR.md), [09 AI](../../09-AI-설계.md).

> **rapier 버전 (착수 전 컴파일 프로브로 검증됨):** 아래 rapier 코드는 **0.22.0·0.26.1 모두 수정 없이 컴파일·실행**되며 결과가 byte-identical(프로브 확인). **`Cargo.toml`에 `rapier2d = { version = "0.26", features = ["enhanced-determinism"] }`로 핀 고정**(현 rustc 1.85에서 resolve되는 버전). **미핀 금지** — rustc≥1.86 환경에선 미검증 API의 0.34가 선택됨. `enhanced-determinism`은 크로스플랫폼 결정성용이라 [ADR-007](../../07-결정기록-ADR.md)(same-build)보다 넓지만 무해.

## ⚠️ 착수 전 필수 반영 (드라이런 점검 결과 — 이 목록이 태스크 코드보다 우선)

프로브 확인: rapier 코드는 0.22·0.26에서 **수정 없이 컴파일·실행**됨. 남은 건 배선/위생 5건:

1. **버전 핀 = 0.26** (반영됨). 미핀 금지(rustc≥1.86 → 미검증 0.34 선택).
2. **Task 4 — kinematic sim 은퇴 완결**: `main.rs`의 `mod sim;` 삭제 · `sim.rs` 삭제(또는 비움) · `world.rs`의 `BALL_FRICTION` 제거(dead-code 경고 방지, 리포는 warning-free 유지) · `loop_runner.rs`의 `use crate::sim::step;` 제거 · 기존 `loop_runner` 테스트 `tick_advances_time_and_moves_ball_when_pushed`와 `sim.rs` 테스트 4개는 **"유지"가 아니라 삭제/교체**.
3. **Task 5 — imports**: `main.rs`에 `use tokio::time::Instant;` 추가, 기존 `interval(Duration::from_secs_f32(DT))` 구동 제거(누산기+~120Hz 프레임으로 교체).
4. **Task 3 — 골 입구 벽 분리는 필수(선택 아님)**: 솔리드 좌우 벽이면 공이 튕겨 나와 **골이 절대 안 난다**(프로브 확인). 그래서 Task 2의 득점 테스트는 Task 3 적용 전까지 FAIL이 정상.
5. **Task 2/3 — 음성 골 테스트 추가**: 공이 골 입구 밖(|y| > GOAL_W/2)으로 나가면 **무득점**임을 단언(골 mouth 조건 회귀 방지). `#[cfg(test)] fn set_ball_for_test(&mut self, pos, vel)` 헬퍼로 공 위치·속도 지정.

*(enhanced-determinism은 ADR-007보다 넓은 크로스플랫폼 결정성 기능 — 무해하나 "크로스플랫폼 결정성이 목표"로 오인하지 말 것.)*

---

## File Structure
- Modify: `server/Cargo.toml` — rapier2d 의존성
- Create: `server/src/physics.rs` — `PhysicsWorld`(바디/콜라이더/파이프라인), `new_kickoff`, `step_physics`, `apply_control`, 스냅샷 추출, 골 판정, 리셋
- Modify: `server/src/sim.rs` — `step`가 physics를 구동하도록 (또는 sim을 physics로 대체하고 골/스코어 로직 유지)
- Modify: `server/src/world.rs` — `GameState`가 물리에서 추출된 값 보유(구조 유지), 필요 상수(벽 두께·반발계수)
- Modify: `server/src/loop_runner.rs` — `tick`가 physics step 호출
- Create: `server/src/accumulator.rs` — 고정스텝 누산기(순수 헬퍼)
- Modify: `server/src/main.rs` — 라이브 루프를 누산기로
- Create: `server/src/replay.rs` — (마지막 태스크) 입력·시드 기록/재생 + 상태 해시

---

## Task 1: rapier2d 의존성 + 물리 월드 생성

**Files:** Modify `server/Cargo.toml`; Create `server/src/physics.rs`; Modify `server/src/main.rs`(`mod physics;`)

- [ ] **Step 1: 의존성 추가**

`server/Cargo.toml [dependencies]`에 추가:
```toml
rapier2d = { version = "0.26", features = ["enhanced-determinism"] }
```
Run: `cargo build --manifest-path server/Cargo.toml` (다운로드/컴파일, 인터넷 필요). 프로브 검증: 0.22·0.26 모두 빌드 OK.

- [ ] **Step 2: 실패하는 테스트 — 월드가 기대 바디를 가진다**

`server/src/physics.rs`:
```rust
use rapier2d::prelude::*;
use crate::world::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kickoff_world_has_ball_and_two_robots_in_bounds() {
        let w = PhysicsWorld::new_kickoff();
        let s = w.snapshot();
        assert_eq!(s.robots.len(), 2);
        assert_eq!(s.ball.pos, Vec2 { x: 0.0, y: 0.0 });
        // 경계 안
        assert!(s.ball.pos.x.abs() <= FIELD_W / 2.0);
    }
}
```

- [ ] **Step 3: 최소 구현 — 월드 구성 + 스냅샷**

`physics.rs`(테스트 모듈 위):
```rust
use rapier2d::prelude::*;
use crate::world::*;

const WALL_T: f32 = 0.2;         // 벽 두께
const BALL_R: f32 = 0.2;
const ROBOT_HX: f32 = 0.25;      // 로봇 반폭
const ROBOT_HY: f32 = 0.2;
const RESTITUTION: f32 = 0.85;

pub struct PhysicsWorld {
    bodies: RigidBodySet,
    colliders: ColliderSet,
    gravity: Vector<Real>,
    params: IntegrationParameters,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad: DefaultBroadPhase,
    narrow: NarrowPhase,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
    query: QueryPipeline,
    ball: RigidBodyHandle,
    robots: Vec<RigidBodyHandle>,
    pub score: (u32, u32),
    pub time: f32,
}

impl PhysicsWorld {
    pub fn new_kickoff() -> Self {
        let mut bodies = RigidBodySet::new();
        let mut colliders = ColliderSet::new();

        // 벽 4개 (고정)
        let hw = FIELD_W / 2.0;
        let hh = FIELD_H / 2.0;
        for (hx, hy, x, y) in [
            (hw, WALL_T, 0.0,  hh), (hw, WALL_T, 0.0, -hh),
            (WALL_T, hh, hw, 0.0), (WALL_T, hh, -hw, 0.0),
        ] {
            colliders.insert(
                ColliderBuilder::cuboid(hx, hy)
                    .translation(vector![x, y])
                    .restitution(RESTITUTION)
                    .build(),
            );
        }

        // 공 (동적)
        let ball = bodies.insert(
            RigidBodyBuilder::dynamic().translation(vector![0.0, 0.0])
                .linear_damping(0.4).build(),
        );
        colliders.insert_with_parent(
            ColliderBuilder::ball(BALL_R).restitution(RESTITUTION).build(),
            ball, &mut bodies,
        );

        // 로봇 2대
        let mut robots = Vec::new();
        for (x, rot) in [(-3.0f32, 0.0f32), (3.0, std::f32::consts::PI)] {
            let rb = bodies.insert(
                RigidBodyBuilder::dynamic().translation(vector![x, 0.0])
                    .rotation(rot).linear_damping(2.0).angular_damping(4.0).build(),
            );
            colliders.insert_with_parent(
                ColliderBuilder::cuboid(ROBOT_HX, ROBOT_HY).build(),
                rb, &mut bodies,
            );
            robots.push(rb);
        }

        PhysicsWorld {
            bodies, colliders,
            gravity: vector![0.0, 0.0],
            params: IntegrationParameters { dt: DT, ..Default::default() },
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad: DefaultBroadPhase::new(),
            narrow: NarrowPhase::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            query: QueryPipeline::new(),
            ball, robots,
            score: (0, 0), time: 0.0,
        }
    }

    pub fn snapshot(&self) -> GameState {
        let b = &self.bodies[self.ball];
        let ball = BallState {
            pos: to_vec2(b.translation()),
            vel: to_vec2(b.linvel()),
        };
        let robots = self.robots.iter().enumerate().map(|(i, h)| {
            let rb = &self.bodies[*h];
            RobotState {
                id: if i == 0 { Team::Blue } else { Team::Red },
                pos: to_vec2(rb.translation()),
                rot: rb.rotation().angle(), // rapier가 정규화된 각도 반환
                vel: to_vec2(rb.linvel()),
            }
        }).collect();
        GameState { robots, ball, score: self.score, time: self.time }
    }
}

fn to_vec2(v: &Vector<Real>) -> Vec2 { Vec2 { x: v.x, y: v.y } }
```
> 컴파일 오류 시 핀 버전 docs로 `DefaultBroadPhase`/`insert_with_parent`/`rotation().angle()` 시그니처 대조.

- [ ] **Step 4: 통과 확인** — Run: `cargo test --manifest-path server/Cargo.toml physics` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: rapier2d physics world (walls/ball/robots) [KB-11]"`

---

## Task 2: 물리 스텝 + 골 판정·리셋

**Files:** Modify `server/src/physics.rs`

- [ ] **Step 1: 실패하는 테스트 — 스텝이 시간을 전진, 공은 경계 이탈 없음(불변식)**

tests 모듈에 추가:
```rust
#[test]
fn stepping_keeps_ball_in_bounds_and_advances_time() {
    let mut w = PhysicsWorld::new_kickoff();
    // 공에 강한 초기 속도
    w.kick_ball_for_test(vector![50.0, 30.0]);
    for _ in 0..600 { w.step(&[ControlOutput::default(); 2]); } // 10초
    let s = w.snapshot();
    assert!(s.time > 9.0);
    assert!(s.ball.pos.x.abs() <= FIELD_W / 2.0 + 0.5); // 벽 안(여유)
    assert!(s.ball.pos.y.abs() <= FIELD_H / 2.0 + 0.5);
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test physics` → FAIL(`step`/`kick_ball_for_test` 없음).

- [ ] **Step 3: 구현 — step + 골 판정 + 리셋**

`impl PhysicsWorld`에 추가:
```rust
pub fn step(&mut self, controls: &[ControlOutput]) {
    apply_controls(&mut self.bodies, &self.robots, controls);
    self.pipeline.step(
        &self.gravity, &self.params, &mut self.islands, &mut self.broad,
        &mut self.narrow, &mut self.bodies, &mut self.colliders,
        &mut self.impulse_joints, &mut self.multibody_joints,
        &mut self.ccd, Some(&mut self.query), &(), &(),
    );
    self.check_goal();
    self.time += DT;
}

fn check_goal(&mut self) {
    let bp = *self.bodies[self.ball].translation();
    let half_w = FIELD_W / 2.0;
    let in_mouth = bp.y.abs() <= GOAL_W / 2.0;
    if bp.x > half_w && in_mouth { self.score.0 += 1; self.reset_kickoff(); }
    else if bp.x < -half_w && in_mouth { self.score.1 += 1; self.reset_kickoff(); }
}

fn reset_kickoff(&mut self) {
    // 공
    let b = &mut self.bodies[self.ball];
    b.set_translation(vector![0.0, 0.0], true);
    b.set_linvel(vector![0.0, 0.0], true);
    b.set_angvel(0.0, true);
    // 로봇
    for (h, (x, rot)) in self.robots.iter().zip([(-3.0f32, 0.0f32), (3.0, std::f32::consts::PI)]) {
        let rb = &mut self.bodies[*h];
        rb.set_translation(vector![x, 0.0], true);
        rb.set_rotation(Rotation::new(rot), true);
        rb.set_linvel(vector![0.0, 0.0], true);
        rb.set_angvel(0.0, true);
    }
}

#[cfg(test)]
pub fn kick_ball_for_test(&mut self, v: Vector<Real>) {
    self.bodies[self.ball].set_linvel(v, true);
}
```
그리고 컨트롤 적용 함수(자유 함수):
```rust
fn apply_controls(bodies: &mut RigidBodySet, robots: &[RigidBodyHandle], controls: &[ControlOutput]) {
    const THRUST: f32 = 6.0;
    const TURN_RATE: f32 = 3.0;
    for (h, c) in robots.iter().zip(controls.iter()) {
        let rb = &mut bodies[*h];
        rb.set_angvel(c.turn * TURN_RATE, true);
        let angle = rb.rotation().angle();
        let dir = vector![angle.cos(), angle.sin()];
        rb.apply_impulse(dir * (c.thrust * THRUST * DT), true);
    }
}
```

- [ ] **Step 4: 통과 확인** — Run: `cargo test physics` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: physics step, goal detection, kickoff reset [KB-12]"`

---

## Task 3: 골 득점 테스트 (불변식: 공을 골로 밀면 스코어)

**Files:** Modify `server/src/physics.rs`

- [ ] **Step 1: 실패하는 테스트**
```rust
#[test]
fn ball_driven_into_right_goal_scores_blue() {
    let mut w = PhysicsWorld::new_kickoff();
    w.kick_ball_for_test(vector![40.0, 0.0]); // 오른쪽으로 강하게
    let mut scored = false;
    for _ in 0..300 {
        w.step(&[ControlOutput::default(); 2]);
        if w.score.0 == 1 { scored = true; break; }
    }
    assert!(scored, "공이 오른쪽 골로 들어가 Blue 득점해야 함");
    // 득점 후 공은 킥오프로 리셋
    assert!(w.snapshot().ball.pos.x.abs() < 0.1);
}
```

- [ ] **Step 2: 실패/통과 확인** — 이미 Task 2에서 골 로직 구현됨 → 이 테스트는 **통합 검증**. Run: `cargo test physics`. FAIL이면 골/리셋 수정, PASS면 진행.

- [ ] **Step 3: (필요 시 수정)** — 벽이 골 입구를 막지 않도록: 좌우 벽에 **골 입구(y ∈ [−GOAL_W/2, GOAL_W/2]) 구간을 비워** 공이 통과하게 한다. Task 1의 좌우 벽을 위/아래 두 조각으로 분리:
```rust
// 좌우 벽: 골 입구 위/아래 두 조각
for side in [hw, -hw] {
    let seg = (hh - GOAL_W / 2.0) / 2.0;       // 각 조각 반높이
    let cy = GOAL_W / 2.0 + seg;               // 조각 중심 y
    for sy in [cy, -cy] {
        colliders.insert(ColliderBuilder::cuboid(WALL_T, seg)
            .translation(vector![side, sy]).restitution(RESTITUTION).build());
    }
}
```
(Task 1의 단일 좌우 벽 코드를 이걸로 교체.)

- [ ] **Step 4: 통과 확인** — Run: `cargo test physics` → PASS.
- [ ] **Step 5: Commit** — `git commit -m "feat: goal opening in side walls, live-goal test [KB-13]"`

---

## Task 4: sim/tick 배선 교체 (physics 구동)

**Files:** Modify `server/src/sim.rs`, `server/src/loop_runner.rs`, `server/src/world.rs`

- [ ] **Step 1: 실패하는 테스트 — tick가 physics를 전진**

`loop_runner.rs` tests에 추가(기존 테스트는 유지하되 GameState 소스가 physics로 바뀜):
```rust
#[test]
fn tick_drives_physics_and_ball_moves_when_robot_pushes() {
    let mut w = PhysicsWorld::new_kickoff();
    let mut ctrls: Vec<Box<dyn Controller>> =
        vec![Box::new(ChaseBallAi), Box::new(ChaseBallAi)];
    for _ in 0..300 { tick(&mut w, &mut ctrls); } // 5초
    let s = w.snapshot();
    // 로봇이 공을 밀어 공 속도가 생겼거나 위치가 원점에서 벗어남
    let ball_moved = s.ball.pos.x.abs() > 0.05 || s.ball.pos.y.abs() > 0.05
        || s.ball.vel.x.abs() > 0.05 || s.ball.vel.y.abs() > 0.05;
    assert!(ball_moved, "AI가 공을 밀면 공이 움직여야 함");
}
```

- [ ] **Step 2: 실패 확인** — `tick`가 아직 `GameState`용. Run: `cargo test` → FAIL.

- [ ] **Step 3: 구현 — tick 시그니처를 PhysicsWorld로**

`loop_runner.rs`의 `tick`를 교체:
```rust
use crate::control::Controller;
use crate::physics::PhysicsWorld;
use crate::world::*;

/// 한 tick: 각 컨트롤러 decide → physics step. (결정적)
pub fn tick(world: &mut PhysicsWorld, controllers: &mut [Box<dyn Controller>]) {
    let snap = world.snapshot();
    let outs: Vec<ControlOutput> = controllers.iter_mut().enumerate().map(|(i, c)| {
        let view = GameView { me: &snap.robots[i], ball: &snap.ball };
        c.decide(&view)
    }).collect();
    world.step(&outs);
}
```
`sim.rs`의 기존 순수 `step`/골 로직은 physics로 이전됐으므로, **sim.rs의 이전 kinematic `step`와 goal 함수·관련 테스트를 제거**(physics.rs가 대체). `world.rs`의 `GameState::new_kickoff` 등 타입은 유지(스냅샷 컨테이너로 계속 사용).
`main.rs`/`net.rs`는 `PhysicsWorld`를 상태 소스로 사용하도록 다음 태스크에서 배선.

- [ ] **Step 4: 통과 확인** — Run: `cargo test` → 전체 PASS(제거된 kinematic 테스트 제외).
- [ ] **Step 5: Commit** — `git commit -m "refactor: tick drives PhysicsWorld; retire kinematic sim [KB-14]"`

---

## Task 5: 고정스텝 누산기 + main/net 배선

**Files:** Create `server/src/accumulator.rs`; Modify `server/src/main.rs`, `server/src/net.rs`(필요 시)

- [ ] **Step 1: 실패하는 테스트 — 누산기가 고정 dt 스텝 수 산출**

`server/src/accumulator.rs`:
```rust
/// 경과 시간을 고정 dt 스텝 수로 변환(잔여 누적). 순수·결정적.
pub struct Accumulator { acc: f32, dt: f32 }

impl Accumulator {
    pub fn new(dt: f32) -> Self { Accumulator { acc: 0.0, dt } }
    /// elapsed를 더하고, 소비할 스텝 수를 반환(잔여는 보존).
    pub fn feed(&mut self, elapsed: f32) -> u32 {
        self.acc += elapsed;
        let mut n = 0;
        while self.acc >= self.dt { self.acc -= self.dt; n += 1; }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feed_yields_fixed_steps_and_keeps_remainder() {
        let mut a = Accumulator::new(1.0 / 60.0);
        assert_eq!(a.feed(1.0 / 60.0 * 2.5), 2); // 2스텝, 0.5 잔여
        assert_eq!(a.feed(1.0 / 60.0 * 0.5), 1); // 잔여 합쳐 1스텝
    }
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test accumulator` → FAIL(모듈 미선언). `main.rs`에 `mod accumulator;` 추가.
- [ ] **Step 3: 통과 확인** — Run: `cargo test accumulator` → PASS.

- [ ] **Step 4: main 루프를 누산기+PhysicsWorld로 배선**

`main.rs`의 sim 루프를 교체: `interval`로 프레임을 받되, 프레임 경과를 누산기에 먹여 **고정 dt 스텝을 N회** 돌리고, 매 30Hz로 `watch`에 `world.snapshot()` 발행. (`watch<GameState>` 채널·`net::serve`는 그대로.) 시간 계측은 `tokio::time::Instant` 사용(결정적 코어 밖 = 허용).
```rust
// 개략:
let mut world = PhysicsWorld::new_kickoff();
let mut ctrls: Vec<Box<dyn Controller>> = vec![Box::new(ChaseBallAi), Box::new(ChaseBallAi)];
let mut acc = Accumulator::new(world::DT);
let mut ticker = interval(Duration::from_millis(8)); // ~120Hz 프레임
let mut last = Instant::now();
let mut since_pub = 0u32;
loop {
    ticker.tick().await;
    let now = Instant::now();
    let elapsed = (now - last).as_secs_f32(); last = now;
    let steps = acc.feed(elapsed);
    for _ in 0..steps { loop_runner::tick(&mut world, &mut ctrls); since_pub += 1; }
    if since_pub >= 2 { since_pub = 0; let _ = tx.send(world.snapshot()); } // ~30Hz
}
```

- [ ] **Step 5: 빌드 + 수동 확인** — Run: `cargo run --manifest-path server/Cargo.toml`. curl로 WS 프레임의 `ball.pos`가 시간에 따라 **0이 아닌 값으로 변함**(공이 밀림) 확인. Commit: `git commit -m "feat: fixed-step accumulator loop driving physics [KB-15]"`

---

## Task 6: 골든 리플레이 (결정성 회귀)

**Files:** Create `server/src/replay.rs`; Modify `server/src/main.rs`(`mod replay;`)

- [ ] **Step 1: 실패하는 테스트 — 같은 입력 → 같은 상태 해시**
```rust
use crate::physics::PhysicsWorld;
use crate::control::{Controller, ChaseBallAi};
use crate::world::GameState;

/// 결정적 상태 해시(부동소수를 비트로).
pub fn hash_state(s: &GameState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for r in &s.robots { r.pos.x.to_bits().hash(&mut h); r.pos.y.to_bits().hash(&mut h); r.rot.to_bits().hash(&mut h); }
    s.ball.pos.x.to_bits().hash(&mut h); s.ball.pos.y.to_bits().hash(&mut h);
    s.score.hash(&mut h);
    h.finish()
}

pub fn run_headless(steps: u32) -> u64 {
    let mut w = PhysicsWorld::new_kickoff();
    let mut c: Vec<Box<dyn Controller>> = vec![Box::new(ChaseBallAi), Box::new(ChaseBallAi)];
    for _ in 0..steps { crate::loop_runner::tick(&mut w, &mut c); }
    hash_state(&w.snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn same_inputs_same_hash_same_build() {
        assert_eq!(run_headless(600), run_headless(600));
    }
}
```

- [ ] **Step 2: 실패 확인** — Run: `cargo test replay` → FAIL(모듈 미선언). `main.rs`에 `mod replay;` 추가.
- [ ] **Step 3: 통과 확인** — Run: `cargo test replay` → PASS. (실패 시: `enhanced-determinism` 피처 확인, RNG·HashMap 이터레이션 배제.)
- [ ] **Step 4: Commit** — `git commit -m "feat: deterministic headless replay + state hash [KB-16]"`

---

## Task 7: E2E 검증 + 문서/KANBAN

- [ ] **Step 1: 전체 테스트** — Run: `cargo test --manifest-path server/Cargo.toml` → 전부 PASS.
- [ ] **Step 2: 수동 E2E** — `cargo run` + `cd client && npm run dev` → 브라우저에서 **로봇이 공을 밀고, 공이 벽에 튕기고, 골이 들어가면 스코어가 오르는지** 확인. (포트 8090)
- [ ] **Step 3: 문서 갱신** — [02 §4.4] 주석에서 "공 정지/보간 없음"을 갱신(공 이동은 이제 됨, 보간은 여전히 Plan 3+). [KANBAN](../../../KANBAN.md) Plan 2 카드 Done 이동, Plan 2 인입 메모의 처리 항목 체크.
- [ ] **Step 4: Commit** — `git commit -m "docs: physics/collision done, kanban update [KB-17]"`

---

## Self-Review 결과

- **스펙 커버리지**: rapier2d 물리([07 ADR-002]) ✅, 밀기 드리블(자유 물리 공, [00 §12]) ✅, 벽 반사 ✅, 골 센서·리셋 ✅, 고정스텝 누산기([07 ADR-007]) ✅, `rot` 물리 바디에서 정규화 추출(무한 누적 해소) ✅, 골든 리플레이([05 §6]) ✅. **전투·부위HP·파츠 = Plan 3**, **제어 모드·입력 = Plan 4**.
- **플레이스홀더 없음**: 코드 제공. rapier 시그니처 대조 지침은 외부 라이브러리 통합의 정상 절차(플레이스홀더 아님).
- **타입 일관성**: `PhysicsWorld`·`snapshot()→GameState`·`tick(&mut PhysicsWorld, ...)`·`Controller`/`GameView`/`ControlOutput` 명칭 전 태스크 일치. `tick` 시그니처가 Plan 1의 `GameState`에서 `PhysicsWorld`로 바뀌는 점 Task 4에 명시.
- **리스크**: rapier 버전 API 편차(→ 핀+docs 대조), 골 입구 벽 분리 누락 시 공이 못 들어감(Task 3에서 처리), 누산기 나선(spiral of death) 방지 위해 프레임당 최대 스텝 상한은 필요 시 추가.

**다음 Plan(3): 전투/데미지/파츠** — 복합 콜라이더(부위별)·상호 데미지·부위HP·파손다운·넉백/스턴, 로드아웃 스탯.
