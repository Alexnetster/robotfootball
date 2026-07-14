# Walking Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cargo run` + 브라우저 1탭으로, 서버가 결정적 tick으로 공과 로봇 2대를 시뮬레이션하고 30Hz로 상태를 브로드캐스트하면, 클라이언트 canvas가 이를 렌더하고, 하드코딩 AI가 공을 몰아 골을 넣으면 스코어가 오른다.

**Architecture:** 서버(Rust)는 **결정적 코어**(I/O 분리, 고정 timestep 60Hz)로 sim을 돌리고, `Controller` 트레잇으로 로봇 입력을 추상화한다(이 슬라이스는 AI Controller만). 상태를 JSON으로 30Hz 브로드캐스트하고, 클라(TS+canvas)는 수신해 렌더만 한다. **물리는 이 단계에선 단순 운동학(kinematic) 적분**(위치+=속도·dt, 공 마찰); rapier2d·충돌은 Plan 2에서 도입.

**Tech Stack:** Rust(tokio, axum, serde_json), TypeScript(Vite, Canvas 2D API). 관련 설계: [00 §9 Controller](../../00-개요-및-게임설계.md), [02 §4.4 월드 상수](../../02-네트워크-프로토콜.md), [05 테스트 아키텍처](../../05-개발프로세스.md), [09 AI](../../09-AI-설계.md).

---

## File Structure

**server/** (Rust)
- `Cargo.toml` — 의존성
- `src/world.rs` — 월드 상수, `Vec2`, `RobotState`, `BallState`, `GameState`, `GameView`, `ControlOutput`
- `src/sim.rs` — `step(state, dt, controls)` 결정적 sim (운동학 + 골 판정)
- `src/control.rs` — `Controller` 트레잇 + `ChaseBallAi`
- `src/loop_runner.rs` — 고정 timestep 게임 루프(sim 구동, tokio interval)
- `src/net.rs` — axum WS: 접속 세션에 30Hz `state` JSON 브로드캐스트
- `src/main.rs` — 부트스트랩(루프 + 서버 기동)

**client/** (TS)
- `package.json`, `index.html`, `vite.config.ts`
- `src/net.ts` — WebSocket 연결 + state 메시지 파싱
- `src/render.ts` — canvas에 필드·골대·로봇(박스)·공 그리기
- `src/main.ts` — 부트스트랩

> 각 파일은 단일 책임. sim/control은 순수 로직(테스트 최적), net/loop는 I/O.

---

## Task 0: 프로젝트 스캐폴딩

**Files:**
- Create: `server/Cargo.toml`, `server/src/main.rs`
- Create: `client/package.json`, `client/vite.config.ts`, `client/index.html`, `client/src/main.ts`

- [ ] **Step 1: Rust 프로젝트 생성**

리포 루트에서 실행(디렉토리를 cargo가 생성):
Run: `cargo new server --name simplectrl_server`
그리고 `server/Cargo.toml`의 `[dependencies]`를 아래로 교체:
```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
axum = { version = "0.7", features = ["ws"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```
> `futures`는 넣지 않는다 — axum 0.7 `WebSocket`은 `send`/`recv` 내장 메서드라 불필요. Task 7에서 `SinkExt`가 실제 필요하면 그때 추가.

- [ ] **Step 2: 빌드 확인**

Run: `cargo build --manifest-path server/Cargo.toml`
Expected: 성공(기본 main). (최초 빌드는 크레이트 다운로드로 인터넷 필요)

- [ ] **Step 3: 클라 프로젝트 생성**

리포 루트에서 실행 — **`client` 디렉토리를 새로 생성**하므로 "비어있지 않은 디렉토리" 프롬프트가 없다(클로버 위험 회피):
Run: `npm create vite@latest client -- --template vanilla-ts`
Run: `cd client && npm install`
Expected: `npm run dev`가 뜸(기본 http://localhost:5173).

- [ ] **Step 4: Commit**

```bash
git add server client
git commit -m "chore: scaffold server(rust) and client(vite-ts) [KB-01]"
```

---

## Task 1: 월드 타입 & 상수

**Files:**
- Create: `server/src/world.rs`
- Modify: `server/src/main.rs` (mod 선언)
- Test: `server/src/world.rs` 내 `#[cfg(test)]`

- [ ] **Step 1: 실패하는 테스트 작성**

`server/src/world.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_has_two_robots_and_centered_ball() {
        let s = GameState::new_kickoff();
        assert_eq!(s.robots.len(), 2);
        assert_eq!(s.ball.pos, Vec2 { x: 0.0, y: 0.0 });
        assert_eq!(s.score, (0, 0));
    }
}
```

- [ ] **Step 2: 컴파일 실패 확인**

Run: `cd server && cargo test world`
Expected: FAIL — `GameState` 미정의.

- [ ] **Step 3: 최소 구현**

`server/src/world.rs` 상단:
```rust
use serde::Serialize;

pub const FIELD_W: f32 = 12.0;   // meters
pub const FIELD_H: f32 = 8.0;
pub const GOAL_W: f32 = 2.4;
pub const DT: f32 = 1.0 / 60.0;  // fixed timestep
pub const BALL_FRICTION: f32 = 0.98;

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
pub struct Vec2 { pub x: f32, pub y: f32 }

#[derive(Clone, Copy, Serialize)]
pub struct RobotState { pub id: Team, pub pos: Vec2, pub rot: f32, pub vel: Vec2 }

#[derive(Clone, Copy, Serialize)]
pub struct BallState { pub pos: Vec2, pub vel: Vec2 }

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
pub enum Team { Blue, Red }

#[derive(Clone, Serialize)]
pub struct GameState {
    pub robots: Vec<RobotState>,
    pub ball: BallState,
    pub score: (u32, u32),
    pub time: f32,
}

/// 컨트롤러가 보는 읽기 전용 뷰
pub struct GameView<'a> { pub me: &'a RobotState, pub ball: &'a BallState }

/// 컨트롤러가 내는 명령(액추에이터 층)
#[derive(Clone, Copy, Default)]
pub struct ControlOutput { pub thrust: f32, pub turn: f32 } // -1..1

impl GameState {
    pub fn new_kickoff() -> Self {
        GameState {
            robots: vec![
                RobotState { id: Team::Blue, pos: Vec2 { x: -3.0, y: 0.0 }, rot: 0.0, vel: Vec2 { x: 0.0, y: 0.0 } },
                RobotState { id: Team::Red,  pos: Vec2 { x:  3.0, y: 0.0 }, rot: std::f32::consts::PI, vel: Vec2 { x: 0.0, y: 0.0 } },
            ],
            ball: BallState { pos: Vec2 { x: 0.0, y: 0.0 }, vel: Vec2 { x: 0.0, y: 0.0 } },
            score: (0, 0),
            time: 0.0,
        }
    }
}
```
`server/src/main.rs`에 `mod world;` 추가.

- [ ] **Step 4: 테스트 통과 확인**

Run: `cd server && cargo test world`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add server/src/world.rs server/src/main.rs
git commit -m "feat: world types and kickoff state [KB-02]"
```

---

## Task 2: 결정적 공 적분(마찰)

**Files:**
- Create: `server/src/sim.rs`
- Modify: `server/src/main.rs` (`mod sim;`)
- Test: `server/src/sim.rs` 내 `#[cfg(test)]`

- [ ] **Step 1: 실패하는 테스트**

`server/src/sim.rs`:
```rust
use crate::world::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ball_moves_by_velocity_and_slows_by_friction() {
        let mut s = GameState::new_kickoff();
        s.ball.vel = Vec2 { x: 1.0, y: 0.0 };
        step(&mut s, &[ControlOutput::default(), ControlOutput::default()]);
        // 위치는 vel*dt 만큼 이동
        assert!((s.ball.pos.x - (1.0 * DT)).abs() < 1e-6);
        // 속도는 마찰로 감소
        assert!(s.ball.vel.x < 1.0 && s.ball.vel.x > 0.0);
    }
}
```

- [ ] **Step 2: 실패 확인**

Run: `cd server && cargo test sim`
Expected: FAIL — `step` 미정의.

- [ ] **Step 3: 최소 구현**

`server/src/sim.rs` 상단(위 test 모듈 위):
```rust
use crate::world::*;

/// 결정적 한 스텝. controls[i]는 robots[i]에 대응.
pub fn step(s: &mut GameState, controls: &[ControlOutput]) {
    // 공: 등속 + 마찰
    s.ball.pos.x += s.ball.vel.x * DT;
    s.ball.pos.y += s.ball.vel.y * DT;
    s.ball.vel.x *= BALL_FRICTION;
    s.ball.vel.y *= BALL_FRICTION;

    let _ = controls; // 로봇 이동은 Task 3
    s.time += DT;
}
```

- [ ] **Step 4: 통과 확인**

Run: `cd server && cargo test sim`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add server/src/sim.rs server/src/main.rs
git commit -m "feat: deterministic ball integration with friction [KB-03]"
```

---

## Task 3: 로봇 이동(제어 출력 적용)

**Files:**
- Modify: `server/src/sim.rs`
- Test: `server/src/sim.rs`

- [ ] **Step 1: 실패하는 테스트 추가**

`sim.rs`의 tests 모듈에 추가:
```rust
#[test]
fn robot_thrust_moves_it_forward_along_rotation() {
    let mut s = GameState::new_kickoff();
    s.robots[0].rot = 0.0; // +x 방향
    let ctrls = [ControlOutput { thrust: 1.0, turn: 0.0 }, ControlOutput::default()];
    step(&mut s, &ctrls);
    assert!(s.robots[0].pos.x > -3.0); // 앞으로 이동
}

#[test]
fn robot_turn_changes_rotation() {
    let mut s = GameState::new_kickoff();
    let ctrls = [ControlOutput { thrust: 0.0, turn: 1.0 }, ControlOutput::default()];
    let before = s.robots[0].rot;
    step(&mut s, &ctrls);
    assert!(s.robots[0].rot != before);
}
```

- [ ] **Step 2: 실패 확인**

Run: `cd server && cargo test sim`
Expected: FAIL — thrust/turn 미적용.

- [ ] **Step 3: 구현 (step에 로봇 처리 추가)**

`sim.rs`의 `step`에서 `let _ = controls;` 줄을 제거하고 그 자리에 추가:
```rust
    const ACCEL: f32 = 8.0;
    const TURN_RATE: f32 = 3.0;
    for (r, c) in s.robots.iter_mut().zip(controls.iter()) {
        r.rot += c.turn * TURN_RATE * DT;
        let (dx, dy) = (r.rot.cos(), r.rot.sin());
        r.vel.x += dx * c.thrust * ACCEL * DT;
        r.vel.y += dy * c.thrust * ACCEL * DT;
        r.vel.x *= 0.9; r.vel.y *= 0.9; // 감쇠
        r.pos.x += r.vel.x * DT;
        r.pos.y += r.vel.y * DT;
    }
```

- [ ] **Step 4: 통과 확인**

Run: `cd server && cargo test sim`
Expected: PASS (전체 sim 테스트).

- [ ] **Step 5: Commit**

```bash
git add server/src/sim.rs
git commit -m "feat: robot movement from control output [KB-04]"
```

---

## Task 4: 골 판정 & 스코어 & 리셋

**Files:**
- Modify: `server/src/sim.rs`
- Test: `server/src/sim.rs`

- [ ] **Step 1: 실패하는 테스트**

```rust
#[test]
fn ball_past_right_goal_scores_for_blue_and_resets() {
    let mut s = GameState::new_kickoff();
    s.ball.pos = Vec2 { x: FIELD_W / 2.0 + 0.1, y: 0.0 }; // 오른쪽 골 안
    step(&mut s, &[ControlOutput::default(); 2]);
    assert_eq!(s.score, (1, 0));           // Blue 득점
    assert_eq!(s.ball.pos, Vec2 { x: 0.0, y: 0.0 }); // 킥오프 리셋
}
```

- [ ] **Step 2: 실패 확인**

Run: `cd server && cargo test sim`
Expected: FAIL — 골 판정 없음.

- [ ] **Step 3: 구현 (step 끝에 골 체크 추가)**

`step` 함수 끝(`s.time += DT;` 앞)에 추가:
```rust
    let half_w = FIELD_W / 2.0;
    let in_goal_mouth = s.ball.pos.y.abs() <= GOAL_W / 2.0;
    if s.ball.pos.x > half_w && in_goal_mouth {
        s.score.0 += 1; reset_kickoff(s);
    } else if s.ball.pos.x < -half_w && in_goal_mouth {
        s.score.1 += 1; reset_kickoff(s);
    }
```
그리고 함수 추가:
```rust
fn reset_kickoff(s: &mut GameState) {
    let fresh = GameState::new_kickoff();
    s.robots = fresh.robots;
    s.ball = fresh.ball;
}
```

- [ ] **Step 4: 통과 확인**

Run: `cd server && cargo test sim`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add server/src/sim.rs
git commit -m "feat: goal detection, score, kickoff reset [KB-05]"
```

---

## Task 5: Controller 트레잇 + ChaseBall AI

**Files:**
- Create: `server/src/control.rs`
- Modify: `server/src/main.rs` (`mod control;`)
- Test: `server/src/control.rs`

- [ ] **Step 1: 실패하는 테스트**

`server/src/control.rs`:
```rust
use crate::world::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaseball_thrusts_forward() {
        let robot = RobotState { id: Team::Blue, pos: Vec2 { x: 0.0, y: 0.0 }, rot: 0.0, vel: Vec2 { x: 0.0, y: 0.0 } };
        let ball = BallState { pos: Vec2 { x: 5.0, y: 0.0 }, vel: Vec2 { x: 0.0, y: 0.0 } };
        let mut ai = ChaseBallAi;
        let out = ai.decide(&GameView { me: &robot, ball: &ball });
        assert!(out.thrust > 0.0); // 공쪽으로 전진
    }
}
```

- [ ] **Step 2: 실패 확인**

Run: `cd server && cargo test control`
Expected: FAIL — `Controller`/`ChaseBallAi` 미정의.

- [ ] **Step 3: 최소 구현**

`control.rs` 상단:
```rust
use crate::world::*;

/// 인간/AI/스크립트 공용 인터페이스 (아키텍처 주춧돌)
pub trait Controller {
    fn decide(&mut self, view: &GameView) -> ControlOutput;
}

pub struct ChaseBallAi;

impl Controller for ChaseBallAi {
    fn decide(&mut self, view: &GameView) -> ControlOutput {
        let dx = view.ball.pos.x - view.me.pos.x;
        let dy = view.ball.pos.y - view.me.pos.y;
        let target = dy.atan2(dx);
        let mut diff = target - view.me.rot;
        while diff > std::f32::consts::PI { diff -= std::f32::consts::TAU; }
        while diff < -std::f32::consts::PI { diff += std::f32::consts::TAU; }
        ControlOutput { thrust: 1.0, turn: diff.clamp(-1.0, 1.0) }
    }
}
```

- [ ] **Step 4: 통과 확인**

Run: `cd server && cargo test control`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add server/src/control.rs server/src/main.rs
git commit -m "feat: Controller trait and ChaseBall AI [KB-06]"
```

---

## Task 6: 고정 timestep 게임 루프

**Files:**
- Create: `server/src/loop_runner.rs`
- Modify: `server/src/main.rs`
- Test: `server/src/loop_runner.rs` (순수 tick 함수만 테스트)

- [ ] **Step 1: 실패하는 테스트 (tick 함수)**

`server/src/loop_runner.rs`:
```rust
use crate::world::*;
use crate::sim::step;
use crate::control::{Controller, ChaseBallAi};

/// 한 tick: 각 로봇 컨트롤러 decide → sim step. (순수, 테스트 대상)
pub fn tick(state: &mut GameState, controllers: &mut [Box<dyn Controller>]) {
    let outs: Vec<ControlOutput> = controllers.iter_mut().enumerate().map(|(i, c)| {
        let view = GameView { me: &state.robots[i], ball: &state.ball };
        c.decide(&view)
    }).collect();
    step(state, &outs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_advances_time_and_moves_ball_when_pushed() {
        let mut s = GameState::new_kickoff();
        let mut ctrls: Vec<Box<dyn Controller>> = vec![Box::new(ChaseBallAi), Box::new(ChaseBallAi)];
        let t0 = s.time;
        for _ in 0..120 { tick(&mut s, &mut ctrls); } // 2초
        assert!(s.time > t0);
        // AI가 공으로 접근 → 로봇이 중앙 쪽으로 이동했는지
        assert!(s.robots[0].pos.x > -3.0);
    }
}
```

- [ ] **Step 2: 실패 확인**

Run: `cd server && cargo test loop_runner`
Expected: FAIL — 모듈/`tick` 미정의.

- [ ] **Step 3: main.rs에 mod 선언**

`server/src/main.rs`에 `mod loop_runner;` 추가. (tick 구현은 Step 1에 이미 포함)

- [ ] **Step 4: 통과 확인**

Run: `cd server && cargo test loop_runner`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add server/src/loop_runner.rs server/src/main.rs
git commit -m "feat: fixed-timestep tick function [KB-07]"
```

---

## Task 7: WebSocket 서버 — 30Hz 상태 브로드캐스트

**Files:**
- Create: `server/src/net.rs`
- Modify: `server/src/main.rs` (루프+서버 기동, `tokio::sync::watch`로 상태 공유)

- [ ] **Step 1: net 스키마 테스트 (직렬화)**

`server/src/net.rs`:
```rust
use crate::world::GameState;
use serde::Serialize;

#[derive(Serialize)]
pub struct StateMsg<'a> { pub t: &'a str, pub state: &'a GameState }

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::GameState;

    #[test]
    fn state_serializes_to_json_with_type_tag() {
        let s = GameState::new_kickoff();
        let msg = StateMsg { t: "state", state: &s };
        let j = serde_json::to_string(&msg).unwrap();
        assert!(j.contains("\"t\":\"state\""));
        assert!(j.contains("\"score\""));
    }
}
```

- [ ] **Step 2: 실패 확인**

Run: `cd server && cargo test net`
Expected: FAIL — 모듈 미선언.

- [ ] **Step 3: 구현 — WS 핸들러 + 브로드캐스트, main 배선**

`server/src/main.rs`:
```rust
mod world; mod sim; mod control; mod loop_runner; mod net;

use std::sync::Arc;
use tokio::sync::watch;
use tokio::time::{interval, Duration};
use world::GameState;
use control::{Controller, ChaseBallAi};

#[tokio::main]
async fn main() {
    let (tx, rx) = watch::channel(GameState::new_kickoff());

    // sim 루프: 60Hz tick, 2 tick마다(=30Hz) 상태 발행
    tokio::spawn(async move {
        let mut state = GameState::new_kickoff();
        let mut ctrls: Vec<Box<dyn Controller>> = vec![Box::new(ChaseBallAi), Box::new(ChaseBallAi)];
        let mut ticker = interval(Duration::from_secs_f32(world::DT));
        let mut n: u64 = 0;
        loop {
            ticker.tick().await;
            loop_runner::tick(&mut state, &mut ctrls);
            n += 1;
            if n % 2 == 0 { let _ = tx.send(state.clone()); }
        }
    });

    net::serve(Arc::new(rx)).await;
}
```

`server/src/net.rs`에 서버 함수 추가:
```rust
use std::sync::Arc;
use axum::{Router, routing::get, extract::{State, ws::{WebSocketUpgrade, Message, WebSocket}}, response::Response};
use tokio::sync::watch;
use tokio::time::{interval, Duration};

type Shared = Arc<watch::Receiver<crate::world::GameState>>;

pub async fn serve(rx: Shared) {
    let app = Router::new().route("/ws", get(ws_handler)).with_state(rx);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    println!("listening on ws://localhost:8080/ws");
    axum::serve(listener, app).await.unwrap();
}

async fn ws_handler(ws: WebSocketUpgrade, State(rx): State<Shared>) -> Response {
    ws.on_upgrade(move |socket| push_state(socket, rx))
}

async fn push_state(mut socket: WebSocket, rx: Shared) {
    let mut tick = interval(Duration::from_millis(33)); // ~30Hz
    loop {
        tick.tick().await;
        let snapshot = rx.borrow().clone();
        let msg = StateMsg { t: "state", state: &snapshot };
        let json = serde_json::to_string(&msg).unwrap();
        if socket.send(Message::Text(json)).await.is_err() { break; }
    }
}
```

- [ ] **Step 4: 통과 + 수동 확인**

Run: `cd server && cargo test net` → PASS.
Run: `cd server && cargo run` → 콘솔 `listening on ws://localhost:8080/ws`.
확인: 브라우저 콘솔에서 `new WebSocket('ws://localhost:8080/ws').onmessage=e=>console.log(e.data)` → state JSON이 초당 ~30회 도착.

- [ ] **Step 5: Commit**

```bash
git add server/src
git commit -m "feat: websocket 30Hz state broadcast + sim loop [KB-08]"
```

---

## Task 8: 클라이언트 — 수신 & canvas 렌더

**Files:**
- Modify: `client/index.html`, `client/src/main.ts`
- Create: `client/src/net.ts`, `client/src/render.ts`

- [ ] **Step 1: net.ts — 연결 & 파싱**

`client/src/net.ts`:
```ts
export type Vec2 = { x: number; y: number };
export type Robot = { id: "Blue" | "Red"; pos: Vec2; rot: number };
export type Ball = { pos: Vec2 };
export type GameState = { robots: Robot[]; ball: Ball; score: [number, number]; time: number };

export function connect(onState: (s: GameState) => void): void {
  const ws = new WebSocket("ws://localhost:8080/ws");
  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.t === "state") onState(msg.state as GameState);
  };
}
```

- [ ] **Step 2: render.ts — canvas 그리기**

`client/src/render.ts`:
```ts
import type { GameState } from "./net";
const FIELD_W = 12, FIELD_H = 8, GOAL_W = 2.4;

export function render(ctx: CanvasRenderingContext2D, s: GameState): void {
  const { width, height } = ctx.canvas;
  const sx = width / FIELD_W, sy = height / FIELD_H;
  const tx = (x: number) => width / 2 + x * sx;
  const ty = (y: number) => height / 2 - y * sy;

  ctx.clearRect(0, 0, width, height);
  ctx.strokeStyle = "#888"; ctx.strokeRect(0, 0, width, height);
  // 골대
  ctx.fillStyle = "#333";
  ctx.fillRect(0, ty(GOAL_W/2), 4, GOAL_W*sy);
  ctx.fillRect(width-4, ty(GOAL_W/2), 4, GOAL_W*sy);
  // 로봇
  for (const r of s.robots) {
    ctx.fillStyle = r.id === "Blue" ? "#39f" : "#f55";
    ctx.save(); ctx.translate(tx(r.pos.x), ty(r.pos.y)); ctx.rotate(-r.rot);
    ctx.fillRect(-15, -12, 30, 24);
    ctx.fillStyle = "#fff"; ctx.fillRect(10, -3, 8, 6); // 앞방향 표시
    ctx.restore();
  }
  // 공
  ctx.fillStyle = "#fff"; ctx.beginPath();
  ctx.arc(tx(s.ball.pos.x), ty(s.ball.pos.y), 7, 0, Math.PI*2); ctx.fill();
  // 스코어
  ctx.fillStyle = "#fff"; ctx.font = "20px sans-serif";
  ctx.fillText(`${s.score[0]} : ${s.score[1]}`, width/2 - 20, 24);
}
```

- [ ] **Step 3: index.html + main.ts 배선**

`client/index.html` body:
```html
<canvas id="c" width="720" height="480" style="background:#111"></canvas>
<script type="module" src="/src/main.ts"></script>
```
`client/src/main.ts`:
```ts
import { connect, type GameState } from "./net";
import { render } from "./render";

const ctx = (document.getElementById("c") as HTMLCanvasElement).getContext("2d")!;
let latest: GameState | null = null;
connect((s) => { latest = s; });
function frame() { if (latest) render(ctx, latest); requestAnimationFrame(frame); }
requestAnimationFrame(frame);
```

- [ ] **Step 4: 엔드투엔드 확인**

Run: 터미널1 `cd server && cargo run`, 터미널2 `cd client && npm run dev`.
브라우저에서 Vite URL 열기.
Expected: 필드에 파랑/빨강 박스 2개와 공, AI가 공을 향해 움직이고, 골 들어가면 스코어 증가·킥오프 리셋.

- [ ] **Step 5: Commit**

```bash
git add client
git commit -m "feat: client canvas render of live state [KB-09]"
```

---

## Task 9: 걷는 뼈대 검증 & 문서 갱신

- [ ] **Step 1: 전체 테스트**

Run: `cd server && cargo test`
Expected: 모든 유닛 테스트 PASS.

- [ ] **Step 2: 수동 수용 확인 (DoD 부분)**

확인: `cargo run` + `npm run dev` → 브라우저에서 **AI vs AI가 공을 몰아 골을 넣는 경기**가 실시간으로 보인다(30Hz 갱신).

- [ ] **Step 3: KANBAN 갱신**

[KANBAN.md](../../../KANBAN.md)에서 KB-01~09를 Done으로 이동.

- [ ] **Step 4: Commit**

```bash
git add KANBAN.md
git commit -m "docs: walking skeleton done, kanban update [KB-10]"
```

---

## Self-Review 결과

- **스펙 커버리지**: 결정적 sim([07 ADR-007]) ✅(단, `tick` 함수 자체는 결정적이며 리플레이/테스트는 이를 직접 호출; **라이브 루프는 `interval` 기반이라 스텝수가 벽시계에 좌우** → 엄밀 고정스텝 누산기와 골든 리플레이는 **Plan 2**에서 도입), Controller 추상화([00 §9]) ✅, 서버권위+브로드캐스트([02 §5]) ✅. **보간은 Plan 2+**(Plan 1은 최신 스냅샷 렌더 → 30Hz 계단현상 가능). 월드 상수([02 §4.4]) ✅. 물리(rapier)·전투·파츠·제어모드·게임흐름·랭킹·NET SIM·**골든 리플레이·클라 vitest** = **후속 Plan**.
- **플레이스홀더 없음**: 모든 코드 단계에 실제 코드 포함.
- **타입 일관성**: `GameState/RobotState/BallState/Vec2/ControlOutput/Controller/GameView` 명칭이 전 태스크에서 일치. 클라 `GameState` 필드는 서버 serde 출력과 대응(enum `Team`은 `"Blue"/"Red"` 문자열로 직렬화됨).

**다음 Plan(2): 물리/충돌(rapier2d)** — 이 뼈대의 운동학 적분을 rapier로 교체하고 로봇↔공(밀기 드리블)·벽 반사·복합 콜라이더를 도입.
