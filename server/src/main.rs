mod accumulator;
mod world;
mod combat;
mod control;
mod human;
mod loop_runner;
mod net;
mod parts;
mod physics;
#[cfg(test)]
mod replay;
mod session;
mod stamina;

use accumulator::Accumulator;
use control::{ChaseBallAi, Controller, DefenderAi};
use human::HumanController;
use physics::PhysicsWorld;
use session::{SessionId, Uplink};
use std::sync::Arc;
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, Duration, Instant};
use world::Team;

/// 슬롯(로봇)마다 Controller를 AI↔사람으로 스왑한다. sim 태스크가 배타 소유
/// (Mutex 불필요). `owner`는 그 슬롯을 현재 잡고 있는 세션(사람일 때만 Some).
///
/// 로스터(고정, KB-57): 0=Blue striker, 1=Blue guard, 2=Red striker, 3=Red guard.
/// 사람은 자기 팀의 striker(0 또는 2)만 조종할 수 있고, guard(1/3)는 항상 AI
/// (DefenderAi)로 남아 협동 역할 분담이 깨지지 않는다.
struct SlotControllers {
    ctrls: Vec<Box<dyn Controller>>,
    owner: Vec<Option<SessionId>>,
}

impl SlotControllers {
    fn new_ai() -> Self {
        Self {
            ctrls: vec![
                Box::new(ChaseBallAi::default()), // 0: Blue striker(Attacker)
                Box::new(DefenderAi::default()),  // 1: Blue guard(Defender)
                Box::new(ChaseBallAi::default()), // 2: Red striker(Attacker)
                Box::new(DefenderAi::default()),  // 3: Red guard(Defender)
            ],
            owner: vec![None, None, None, None],
        }
    }

    /// 팀 → 그 팀의 striker 슬롯 인덱스(사람이 조종 가능한 유일한 슬롯).
    fn slot_index(team: Team) -> usize {
        match team {
            Team::Blue => 0,
            Team::Red => 2,
        }
    }

    fn owner_slot(&self, sid: SessionId) -> Option<usize> {
        self.owner.iter().position(|o| *o == Some(sid))
    }

    /// 다운링크(스냅샷 `ctrl`)·AFK 판정·테스트용 조회 헬퍼.
    fn is_human(&self, i: usize) -> bool {
        self.owner[i].is_some()
    }

    /// join/input/leave를 슬롯 상태에 반영. 이미 다른 세션이 점유한 슬롯의
    /// join은 거부(무시) — 슬롯 경합 시 기존 점유자가 유지된다.
    fn apply(&mut self, uplink: Uplink, sid: SessionId) {
        match uplink {
            Uplink::Join(team) => {
                let i = Self::slot_index(team);
                // 대상 슬롯이 이미 점유돼 있으면 컨트롤러 재생성 없이 종료:
                // 다른 세션이면 경합 거부, 같은 세션이면 멱등(보유 입력 보존).
                if self.owner[i].is_some() {
                    return;
                }
                // 같은 세션이 다른 슬롯을 이미 잡고 있었다면 그쪽은 AI로 되돌린다
                // (한 세션 = 최대 한 슬롯).
                if let Some(prev) = self.owner_slot(sid) {
                    if prev != i {
                        self.ctrls[prev] = Box::new(ChaseBallAi::default());
                        self.owner[prev] = None;
                    }
                }
                self.ctrls[i] = Box::new(HumanController::default());
                self.owner[i] = Some(sid);
            }
            Uplink::Leave => {
                if let Some(i) = self.owner_slot(sid) {
                    self.ctrls[i] = Box::new(ChaseBallAi::default());
                    self.owner[i] = None;
                }
            }
            Uplink::Input(input) => {
                if let Some(i) = self.owner_slot(sid) {
                    if let Some(hc) = self.ctrls[i].as_any_mut().downcast_mut::<HumanController>() {
                        hc.set(input);
                    }
                }
            }
        }
    }

    fn as_mut_slice(&mut self) -> &mut [Box<dyn Controller>] {
        &mut self.ctrls
    }
}

/// 사람 점유 슬롯에 이만큼 Input이 없으면 자동 Leave(AI 인계, KB-55).
/// 튜닝 여지: 데모 스코프 기본값(30s). 필요 시 조정.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() {
    // 4대 매치(팀당 공격형+수비형, 로스터 고정) 월드를 먼저 만들어 그 스냅샷으로
    // 초기 방송 상태를 시드한다 — 물리 루프가 첫 틱을 발행하기 전에도 다운링크
    // ctrl/robots 길이가 항상 4로 일관되게 유지된다.
    let mut world = PhysicsWorld::new_match();
    let (tx, rx) = watch::channel(world.snapshot());
    let (uplink_tx, mut uplink_rx) = mpsc::unbounded_channel::<(SessionId, Uplink)>();

    // 물리 루프: ~120Hz 프레임을 실제 경과 시간으로 계측해 고정스텝 누산기에
    // 먹이고, 누산된 만큼 물리를 전진(고정 dt). 2스텝마다(=30Hz) 상태 발행.
    tokio::spawn(async move {
        let mut slots = SlotControllers::new_ai();
        let mut acc = Accumulator::new(world::DT);
        let mut ticker = interval(Duration::from_millis(8)); // ~120Hz 프레임
        let mut last = Instant::now();
        let mut since_pub: u32 = 0;
        // AFK 자동 해제(KB-55): 슬롯별 마지막 활동(join/input) 시각. 슬롯 수(4)만큼.
        let mut last_input = vec![Instant::now(); slots.ctrls.len()];
        loop {
            ticker.tick().await;
            // 업링크 논블로킹 드레인 → 슬롯 컨트롤러(AI↔사람) 반영.
            while let Ok((sid, u)) = uplink_rx.try_recv() {
                slots.apply(u, sid);
                // join(성공 시)/input 모두 "활동"으로 간주해 해당 슬롯 타이머 리셋.
                // leave나 거부된 join은 owner_slot(sid)가 None이라 자연히 no-op.
                if let Some(i) = slots.owner_slot(sid) {
                    last_input[i] = Instant::now();
                }
            }
            let now = Instant::now();
            // AFK 타이머: 사람 점유 슬롯(striker만 해당)이 IDLE_TIMEOUT 동안
            // 무입력이면 강제 leave. guard 슬롯은 애초 사람 소유가 안 되므로 no-op.
            for i in 0..last_input.len() {
                if slots.is_human(i) && now.duration_since(last_input[i]) > IDLE_TIMEOUT {
                    if let Some(sid) = slots.owner[i] {
                        slots.apply(Uplink::Leave, sid);
                    }
                }
            }
            let elapsed = now.duration_since(last).as_secs_f32();
            last = now;
            let steps = acc.feed(elapsed);
            for _ in 0..steps {
                loop_runner::tick(&mut world, slots.as_mut_slice());
                since_pub += 1;
            }
            if since_pub >= 2 {
                since_pub = 0;
                let mut snap = world.snapshot();
                // 물리 레이어는 소유자를 모르므로 항상 "ai"를 채운다; 사람 점유
                // 슬롯(striker)만 여기서 "human"으로 덮어써 다운링크에 반영(KB-55).
                for i in 0..snap.ctrl.len() {
                    if slots.is_human(i) {
                        snap.ctrl[i] = "human".to_string();
                    }
                }
                // AI 의사결정 상태 라벨/목표 좌표(KB-68/69): 사람 슬롯은 Controller
                // 기본 구현이 None을 주므로 별도 분기 없이 그대로 둔다.
                for i in 0..snap.ai_state.len() {
                    snap.ai_state[i] = slots.ctrls[i].state_label();
                    snap.ai_target[i] = slots.ctrls[i]
                        .debug_target()
                        .map(|(x, y)| world::Vec2 { x, y });
                }
                let _ = tx.send(snap); // ~30Hz
            }
        }
    });

    net::serve(Arc::new(rx), uplink_tx).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use world::ControlOutput;

    #[test]
    fn join_swaps_slot_to_human_leave_reverts_to_ai() {
        let mut slots = SlotControllers::new_ai();
        assert!(!slots.is_human(0));
        slots.apply(Uplink::Join(Team::Blue), 1);
        assert!(slots.is_human(0));
        slots.apply(Uplink::Leave, 1);
        assert!(!slots.is_human(0));
    }

    #[test]
    fn join_rejected_when_slot_already_taken() {
        let mut slots = SlotControllers::new_ai();
        slots.apply(Uplink::Join(Team::Blue), 1);
        assert!(slots.is_human(0));
        slots.apply(Uplink::Join(Team::Blue), 2); // 다른 세션의 경합 join
        assert_eq!(
            slots.owner[0],
            Some(1),
            "기존 점유 세션이 유지되어야 함(거부)"
        );
        // 경합 세션(2)의 입력은 이 슬롯에 적용되면 안 됨: leave(2)해도 슬롯은 그대로 사람(1) 소유.
        slots.apply(Uplink::Leave, 2);
        assert!(slots.is_human(0));
    }

    #[test]
    fn input_only_applies_to_owning_session() {
        let mut slots = SlotControllers::new_ai();
        slots.apply(Uplink::Join(Team::Blue), 1);
        slots.apply(
            Uplink::Input(ControlOutput {
                thrust: 1.0,
                turn: 0.5,
                run: false,
                kick: false,
            }),
            1,
        );
        let out = slots.ctrls[0]
            .as_any_mut()
            .downcast_mut::<HumanController>()
            .unwrap();
        // decide()는 view를 쓰지 않으므로 임의 뷰로도 최근 입력을 그대로 반환.
        let robot = world::RobotState {
            id: Team::Blue,
            pos: world::Vec2 { x: 0.0, y: 0.0 },
            rot: 0.0,
            vel: world::Vec2 { x: 0.0, y: 0.0 },
            robot: String::new(),
            parts: Vec::new(),
            down: world::Down::default(),
            st: Vec::new(),
            stamina: 1.0,
        };
        let ball = world::BallState {
            pos: world::Vec2 { x: 0.0, y: 0.0 },
            vel: world::Vec2 { x: 0.0, y: 0.0 },
        };
        let decided = out.decide(&world::GameView {
            me: &robot,
            ball: &ball,
        });
        assert_eq!(decided.thrust, 1.0);
        assert_eq!(decided.turn, 0.5);

        // 슬롯을 점유하지 않은 세션(2)의 input은 무시된다.
        slots.apply(
            Uplink::Input(ControlOutput {
                thrust: -1.0,
                turn: -1.0,
                run: false,
                kick: false,
            }),
            2,
        );
        let hc = slots.ctrls[0]
            .as_any_mut()
            .downcast_mut::<HumanController>()
            .unwrap();
        let decided2 = hc.decide(&world::GameView {
            me: &robot,
            ball: &ball,
        });
        assert_eq!(decided2.thrust, 1.0, "타 세션 입력은 반영되면 안 됨");
    }

    #[test]
    fn rejoin_owned_slot_preserves_input() {
        let mut slots = SlotControllers::new_ai();
        slots.apply(Uplink::Join(Team::Blue), 1);
        slots.apply(
            Uplink::Input(ControlOutput {
                thrust: 1.0,
                turn: 0.0,
                run: false,
                kick: false,
            }),
            1,
        );
        // 같은 세션이 이미 점유한 슬롯에 재-join → 컨트롤러 재생성 없이 입력 보존.
        slots.apply(Uplink::Join(Team::Blue), 1);
        let robot = world::RobotState {
            id: Team::Blue,
            pos: world::Vec2 { x: 0.0, y: 0.0 },
            rot: 0.0,
            vel: world::Vec2 { x: 0.0, y: 0.0 },
            robot: String::new(),
            parts: Vec::new(),
            down: world::Down::default(),
            st: Vec::new(),
            stamina: 1.0,
        };
        let ball = world::BallState {
            pos: world::Vec2 { x: 0.0, y: 0.0 },
            vel: world::Vec2 { x: 0.0, y: 0.0 },
        };
        let hc = slots.ctrls[0]
            .as_any_mut()
            .downcast_mut::<HumanController>()
            .unwrap();
        let out = hc.decide(&world::GameView {
            me: &robot,
            ball: &ball,
        });
        assert_eq!(out.thrust, 1.0, "같은 슬롯 재-join 시 보유 입력 보존");
    }

    /// KB-57: 로스터 고정(0=Blue striker,1=Blue guard,2=Red striker,3=Red guard).
    /// join(Blue)은 striker(0)만 사람으로 바꾸고 guard(1)는 AI로 남아야 하며,
    /// leave 시 0은 다시 AI(Attacker)로 복귀해야 한다.
    #[test]
    fn join_blue_makes_striker_human_guard_stays_ai_then_leave_reverts() {
        let mut slots = SlotControllers::new_ai();
        assert_eq!(slots.ctrls.len(), 4, "로스터는 4대여야 함");
        assert!(!slots.is_human(0) && !slots.is_human(1));
        slots.apply(Uplink::Join(Team::Blue), 1);
        assert!(slots.is_human(0), "Blue striker(0)가 사람이어야 함");
        assert!(!slots.is_human(1), "Blue guard(1)는 항상 AI로 남아야 함");
        assert!(!slots.is_human(2) && !slots.is_human(3), "Red 슬롯은 영향받지 않아야 함");
        slots.apply(Uplink::Leave, 1);
        assert!(!slots.is_human(0), "leave 후 striker(0)는 AI로 복귀해야 함");
    }

    /// join(Red)은 Red striker(슬롯 2)를 사람으로 바꾼다(팀→striker 인덱스 매핑).
    #[test]
    fn join_red_makes_slot_two_human() {
        let mut slots = SlotControllers::new_ai();
        slots.apply(Uplink::Join(Team::Red), 1);
        assert!(slots.is_human(2), "Red striker(2)가 사람이어야 함");
        assert!(!slots.is_human(3), "Red guard(3)는 AI로 남아야 함");
        assert!(!slots.is_human(0) && !slots.is_human(1), "Blue 슬롯은 영향받지 않아야 함");
    }
}
