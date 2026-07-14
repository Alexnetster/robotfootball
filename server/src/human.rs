use crate::control::Controller;
use crate::world::{ControlOutput, GameView};
use std::any::Any;

/// 사람이 조작하는 슬롯의 컨트롤러. 최근 입력을 보유했다가 그대로 반환한다.
/// (다운/스턴 중 입력 무시는 physics 쪽에서 이미 처리 — 여기선 순수 보유/반환만.)
#[derive(Default)]
pub struct HumanController {
    last: ControlOutput,
}

impl HumanController {
    /// mpsc로 들어온 최신 입력을 갱신. `SlotControllers.apply`가 호출.
    pub fn set(&mut self, input: ControlOutput) {
        self.last = input;
    }
}

impl Controller for HumanController {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn decide(&mut self, _view: &GameView) -> ControlOutput {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{BallState, Down, RobotState, Team, Vec2};

    fn dummy_view() -> (RobotState, BallState) {
        let robot = RobotState {
            id: Team::Blue,
            pos: Vec2 { x: 0.0, y: 0.0 },
            rot: 0.0,
            vel: Vec2 { x: 0.0, y: 0.0 },
            robot: String::new(),
            parts: Vec::new(),
            down: Down::default(),
            st: Vec::new(),
            stamina: 1.0,
        };
        let ball = BallState {
            pos: Vec2 { x: 0.0, y: 0.0 },
            vel: Vec2 { x: 0.0, y: 0.0 },
        };
        (robot, ball)
    }

    #[test]
    fn human_controller_returns_held_input() {
        let mut hc = HumanController::default();
        hc.set(ControlOutput {
            thrust: 1.0,
            turn: -1.0,
            run: false,
            kick: false,
        });
        let (robot, ball) = dummy_view();
        let out = hc.decide(&GameView {
            me: &robot,
            ball: &ball,
        });
        assert_eq!(out.thrust, 1.0);
        assert_eq!(out.turn, -1.0);
    }

    #[test]
    fn human_controller_default_is_idle() {
        let mut hc = HumanController::default();
        let (robot, ball) = dummy_view();
        let out = hc.decide(&GameView {
            me: &robot,
            ball: &ball,
        });
        assert_eq!(out.thrust, 0.0);
        assert_eq!(out.turn, 0.0);
    }
}
