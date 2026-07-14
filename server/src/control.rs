use crate::world::*;
use std::any::Any;

/// 인간/AI/스크립트 공용 인터페이스 (아키텍처 주춧돌)
pub trait Controller: Send {
    fn decide(&mut self, view: &GameView) -> ControlOutput;
    /// 슬롯 컨트롤러 스왑(AI↔사람) 시 구체 타입으로 downcast하기 위함
    /// (예: `SlotControllers`가 사람 슬롯에 최신 입력을 주입).
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// 공을 향해 전진하는 기본 AI. 벽/펜스/코너에 박혀 정지하면(공을 계속 밀어도
/// 속도≈0) 스턱으로 판정하고 잠깐 후진+회전으로 빠져나온다(KB-49).
#[derive(Default)]
pub struct ChaseBallAi {
    /// 정지(속도<STUCK_SPEED) 지속 프레임 수.
    stuck: u32,
    /// 남은 탈출 기동 프레임(>0이면 후진+회전).
    escape: u32,
}

/// 스턱 판정 속도 임계(m/s). 이 미만을 정지로 본다.
const STUCK_SPEED: f32 = 0.25;
/// 정지가 이만큼(프레임=~1초 @60Hz) 지속되면 스턱으로 보고 탈출 시작.
const STUCK_LIMIT: u32 = 60;
/// 탈출 기동 지속(프레임=~0.66초).
const ESCAPE_FRAMES: u32 = 40;

/// AI 슛 판정(KB-52). 공이 이 거리(m) 안(서버 사거리 0.9보다 안쪽)이고,
/// 정면 콘 안이며, 상대 골 방향으로 정렬됐을 때만 찬다(자책골 방지).
const AI_KICK_RANGE: f32 = 0.85;
const AI_KICK_FRONT_COS: f32 = 0.5; // 공이 정면 ~60° 이내
const AI_KICK_GOAL_COS: f32 = 0.55; // 상대 골 방향 ~57° 이내(정렬됐을 때만 슛)

/// Attacker(ChaseBallAi)/Defender(DefenderAi) 공용 헬퍼. 스턱 탈출·슛 판정은
/// 역할과 무관한 순수 로직이라 자유 함수로 공유한다(KB-57: 협동 AI 역할 분담).
///
/// 필드 중앙(y=0) 쪽으로 도는 turn 부호. 위쪽(+y) 벽이면 CW로 내려오게.
fn escape_turn(pos_y: f32) -> f32 {
    if pos_y >= 0.0 { -1.0 } else { 1.0 }
}

/// 후진하며 중앙 쪽으로 회전(벽에서 멀어짐).
fn escape_output(pos_y: f32) -> ControlOutput {
    ControlOutput {
        thrust: -1.0,
        turn: escape_turn(pos_y),
        run: false,
        kick: false,
    }
}

/// 상대 골 x좌표. 블루는 +x, 레드는 −x로 공격(physics::check_goal과 동일 규약).
fn enemy_goal_x(team: Team) -> f32 {
    match team {
        Team::Blue => FIELD_W / 2.0,
        Team::Red => -FIELD_W / 2.0,
    }
}

/// 자기 골 x좌표(enemy_goal_x의 반대편 — check_goal 규약과 정합). DefenderAi가
/// 지킬 위치를 정하는 데 쓴다.
fn own_goal_x(team: Team) -> f32 {
    match team {
        Team::Blue => -FIELD_W / 2.0,
        Team::Red => FIELD_W / 2.0,
    }
}

/// 공이 정면 사거리 안 + 정면이 상대 골 방향으로 정렬 → 슛(자책골 회피).
/// Attacker/Defender 공용(둘 다 자책골 회피 조건은 동일해야 함).
fn wants_kick(view: &GameView) -> bool {
    let (fx, fy) = (view.me.rot.cos(), view.me.rot.sin());
    let bdx = view.ball.pos.x - view.me.pos.x;
    let bdy = view.ball.pos.y - view.me.pos.y;
    let bdist = (bdx * bdx + bdy * bdy).sqrt();
    if bdist > AI_KICK_RANGE {
        return false;
    }
    // 공이 정면 콘 안(거리≈0이면 정면으로 간주).
    let ball_front = bdist <= 1e-6 || (fx * bdx + fy * bdy) / bdist >= AI_KICK_FRONT_COS;
    // 정면이 상대 골 방향으로 정렬(자책골 방지).
    let gdx = enemy_goal_x(view.me.id) - view.me.pos.x;
    let gdy = -view.me.pos.y;
    let gdist = (gdx * gdx + gdy * gdy).sqrt();
    let goalward = gdist <= 1e-6 || (fx * gdx + fy * gdy) / gdist >= AI_KICK_GOAL_COS;
    ball_front && goalward
}

impl Controller for ChaseBallAi {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn decide(&mut self, view: &GameView) -> ControlOutput {
        // 탈출 기동 진행 중: 끝날 때까지 후진+회전.
        if self.escape > 0 {
            self.escape -= 1;
            self.stuck = 0;
            return escape_output(view.me.pos.y);
        }
        // 정지 지속 추적(공을 밀어도 벽에 막혀 속도가 안 나는 상태).
        let speed = (view.me.vel.x * view.me.vel.x + view.me.vel.y * view.me.vel.y).sqrt();
        if speed < STUCK_SPEED {
            self.stuck += 1;
        } else {
            self.stuck = 0;
        }
        if self.stuck >= STUCK_LIMIT {
            self.stuck = 0;
            self.escape = ESCAPE_FRAMES;
            return escape_output(view.me.pos.y);
        }

        // 평소: 공을 향해 전진.
        let dx = view.ball.pos.x - view.me.pos.x;
        let dy = view.ball.pos.y - view.me.pos.y;
        let target = dy.atan2(dx);
        let mut diff = target - view.me.rot;
        while diff > std::f32::consts::PI {
            diff -= std::f32::consts::TAU;
        }
        while diff < -std::f32::consts::PI {
            diff += std::f32::consts::TAU;
        }
        ControlOutput {
            thrust: 1.0,
            turn: diff.clamp(-1.0, 1.0),
            // AI는 달리기를 쓰지 않는다(KB-45 YAGNI: AI sprint 없음).
            run: false,
            // 슛(KB-52): 정면 사거리 안 + 상대 골 정렬 시에만. 서버가 상승엣지로 1회
            // 발사하므로, 사거리 안에서 kick=true를 유지해도 재발사는 쿨다운/엣지가 관리.
            kick: wants_kick(view),
        }
    }
}

/// 자기 골을 지키는 수비형 AI(KB-57). 협동 역할 분담: 공격형은 `ChaseBallAi`(이름
/// 그대로 유지, 회귀 최소화)를 그대로 쓴다. 공을 직접 쫓지 않고 **자기 골과 공의
/// 예측 위치를 잇는 선 위, 자기 골에서 유효 가드 거리 이내 지점**을 목표로 움직인다:
/// 공이 자기 진영 가까이 오면(혹은 그쪽으로 굴러오면) 그 위치까지 나가서 막고,
/// 멀리 있으면 골 앞에서 대기한다. 공격형(ChaseBallAi)과 목표가 근본적으로 달라
/// 공에 뭉치지 않는다. 스턱 탈출·슛(자책골 회피) 판정은 공격형과 동일 로직을 공유한다.
/// 방어 범위 확대 + 조기 인지 설계:
/// [defender-ai-positioning-design](../../docs/superpowers/specs/2026-07-14-defender-ai-positioning-design.md).
#[derive(Default)]
pub struct DefenderAi {
    /// 정지(속도<STUCK_SPEED) 지속 프레임 수.
    stuck: u32,
    /// 남은 탈출 기동 프레임(>0이면 후진+회전).
    escape: u32,
}

/// 공(예측 위치)이 상대 진영일 때의 가드 거리(m, 골 앞 대기, 보수적).
const GUARD_HOME: f32 = 2.5;
/// 공(예측 위치)이 자기 진영일 때의 가드 거리(m, 더 멀리 나가 요격, 공격적).
const GUARD_ENGAGE: f32 = 4.0;
/// 공 속도 예측 시간(s). 공의 미래 위치 = `pos + vel * LOOKAHEAD`로 조기 인지.
const LOOKAHEAD: f32 = 0.35;
/// 도착 판정 반경(m, KB-59). 목표에 이 안으로 들어오면 전진을 멈추고 공을 조준하며
/// 대기 — 도착 지점에서 목표방향이 ≈0이 돼 각도가 노이즈가 되는 제자리 스핀을 방지.
const DEFENDER_ARRIVE: f32 = 0.45;
/// 회전 데드존(rad). 이보다 작은 조준 오차엔 회전하지 않음(미세 떨림/스핀 억제).
const TURN_DEADZONE: f32 = 0.1;

impl DefenderAi {
    /// 자기 골과 공의 예측 위치를 잇는 선 위, 자기 골에서 유효 가드 거리 이내
    /// 지점을 목표로 계산한다(순수 함수, 테스트 용이). 예측 위치가 자기 진영이면
    /// `GUARD_ENGAGE`(더 멀리 요격), 상대 진영이면 `GUARD_HOME`(골 앞 대기)을 쓴다.
    ///
    /// KB-61: 공 속도의 접선(골라인과 나란한) 성분이 크면 `LOOKAHEAD` 외삽 지점이
    /// 골에서 유클리드 거리로 공의 **실제** 현재 위치보다 더 멀어질 수 있다. 그대로
    /// 두면 수비형이 공보다 골에서 먼 쪽(미드필드 쪽)에 머물며 좁혀오는 공을 자기
    /// 골 방향으로 계속 떠미는 자살골 패턴이 생긴다. 방향은 예측 지점을 그대로 쓰되,
    /// 목표 거리는 공의 실제 위치보다 멀어지지 않게 캡을 걸어 항상 공의 골 쪽에
    /// 자리잡게 한다.
    fn guard_target(team: Team, ball: &BallState) -> (f32, f32) {
        let gx = own_goal_x(team);
        let gy = 0.0;
        let pred_x = ball.pos.x + ball.vel.x * LOOKAHEAD;
        let pred_y = ball.pos.y + ball.vel.y * LOOKAHEAD;
        let dx = pred_x - gx;
        let dy = pred_y - gy;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= 1e-6 {
            return (gx, gy);
        }
        let own_half = match team {
            Team::Blue => pred_x <= 0.0,
            Team::Red => pred_x >= 0.0,
        };
        let guard = if own_half { GUARD_ENGAGE } else { GUARD_HOME };
        let actual_dx = ball.pos.x - gx;
        let actual_dy = ball.pos.y - gy;
        let actual_dist = (actual_dx * actual_dx + actual_dy * actual_dy).sqrt();
        let clamped = dist.min(guard).min(actual_dist.max(1e-6));
        let k = clamped / dist;
        (gx + dx * k, gy + dy * k)
    }
}

impl Controller for DefenderAi {
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }

    fn decide(&mut self, view: &GameView) -> ControlOutput {
        // 탈출 기동 진행 중: 끝날 때까지 후진+회전(공격형과 동일 로직).
        if self.escape > 0 {
            self.escape -= 1;
            self.stuck = 0;
            return escape_output(view.me.pos.y);
        }
        let speed = (view.me.vel.x * view.me.vel.x + view.me.vel.y * view.me.vel.y).sqrt();
        if speed < STUCK_SPEED {
            self.stuck += 1;
        } else {
            self.stuck = 0;
        }
        if self.stuck >= STUCK_LIMIT {
            self.stuck = 0;
            self.escape = ESCAPE_FRAMES;
            return escape_output(view.me.pos.y);
        }

        // 평소: 골 지킴 목표로 이동하되, **목표에 도착하면 제자리서 돌지 말고 공을
        // 바라보며 대기**(골키퍼처럼). 도착 시 목표방향(dx,dy)이 ≈0이 돼 각도가 노이즈가
        // 되고, 높은 회전율 탓에 좌우로 뱅뱅 도는 문제를 방지한다(KB-59).
        let (gtx, gty) = Self::guard_target(view.me.id, view.ball);
        let tdx = gtx - view.me.pos.x;
        let tdy = gty - view.me.pos.y;
        let to_target = (tdx * tdx + tdy * tdy).sqrt();
        // 도착했으면 전진 멈추고 공 조준(대기), 아니면 목표로 전진.
        let (aimx, aimy, thrust) = if to_target > DEFENDER_ARRIVE {
            (tdx, tdy, 1.0)
        } else {
            (view.ball.pos.x - view.me.pos.x, view.ball.pos.y - view.me.pos.y, 0.0)
        };
        let aim_dist = (aimx * aimx + aimy * aimy).sqrt();
        let target = aimy.atan2(aimx);
        let mut diff = target - view.me.rot;
        while diff > std::f32::consts::PI {
            diff -= std::f32::consts::TAU;
        }
        while diff < -std::f32::consts::PI {
            diff += std::f32::consts::TAU;
        }
        // 회전 데드존 + 조준대상이 사실상 제자리(공이 몸 위)면 회전 안 함(스핀 억제).
        let turn = if aim_dist < 1e-3 || diff.abs() < TURN_DEADZONE {
            0.0
        } else {
            diff.clamp(-1.0, 1.0)
        };
        ControlOutput {
            thrust,
            turn,
            run: false,
            // 클리어 슛: 정면 사거리 + 상대 골 정렬 시에만(공격형과 동일 조건 공유).
            kick: wants_kick(view),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaseball_thrusts_forward() {
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
            pos: Vec2 { x: 5.0, y: 0.0 },
            vel: Vec2 { x: 0.0, y: 0.0 },
        };
        let mut ai = ChaseBallAi::default();
        let out = ai.decide(&GameView {
            me: &robot,
            ball: &ball,
        });
        assert!(out.thrust > 0.0); // 공쪽으로 전진
    }

    /// 벽에 박혀 정지(속도≈0)가 오래 지속되면 후진 탈출 기동으로 전환해야 한다(KB-49).
    #[test]
    fn stuck_against_wall_triggers_reverse_escape() {
        // 위쪽(+y) 벽 근처에서 정지, 공은 벽 너머(도달 불가) 방향.
        let robot = RobotState {
            id: Team::Blue,
            pos: Vec2 { x: 5.5, y: 3.5 },
            rot: 0.0,
            vel: Vec2 { x: 0.0, y: 0.0 }, // 정지
            robot: String::new(),
            parts: Vec::new(),
            down: Down::default(),
            st: Vec::new(),
            stamina: 1.0,
        };
        let ball = BallState {
            pos: Vec2 { x: 6.5, y: 4.5 },
            vel: Vec2 { x: 0.0, y: 0.0 },
        };
        let view = GameView { me: &robot, ball: &ball };
        let mut ai = ChaseBallAi::default();
        // 처음에는 전진(공쪽).
        assert!(ai.decide(&view).thrust > 0.0);
        // 정지가 STUCK_LIMIT 넘게 지속되면 후진 탈출로 전환.
        let mut escaped = false;
        for _ in 0..(STUCK_LIMIT + ESCAPE_FRAMES + 5) {
            if ai.decide(&view).thrust < 0.0 {
                escaped = true;
                break;
            }
        }
        assert!(escaped, "정지 지속 시 후진 탈출 기동이 나와야 함");
    }

    fn view_at(team: Team, pos: Vec2, rot: f32, ball: Vec2) -> (RobotState, BallState) {
        (
            RobotState {
                id: team, pos, rot, vel: Vec2 { x: 0.0, y: 0.0 },
                robot: String::new(), parts: Vec::new(),
                down: Down::default(), st: Vec::new(), stamina: 1.0,
            },
            BallState { pos: ball, vel: Vec2 { x: 0.0, y: 0.0 } },
        )
    }

    /// 공이 정면 사거리 안이고 상대 골(블루=+x) 방향으로 정렬되면 슛(KB-52).
    #[test]
    fn ai_kicks_when_ball_in_front_toward_enemy_goal() {
        let (me, ball) = view_at(Team::Blue, Vec2 { x: 0.0, y: 0.0 }, 0.0, Vec2 { x: 0.6, y: 0.0 });
        let mut ai = ChaseBallAi::default();
        assert!(ai.decide(&GameView { me: &me, ball: &ball }).kick, "정렬+사거리 내 → 슛");
    }

    /// 정면이 자기 골 쪽이면(상대 골 반대) 사거리·정면이어도 안 참(자책골 방지).
    #[test]
    fn ai_does_not_kick_toward_own_goal() {
        // 블루가 −x(자기 골)를 향한 채 앞의 공을 참 → 상대 골 정렬 실패로 무슛.
        let (me, ball) = view_at(Team::Blue, Vec2 { x: 0.0, y: 0.0 }, std::f32::consts::PI, Vec2 { x: -0.6, y: 0.0 });
        let mut ai = ChaseBallAi::default();
        assert!(!ai.decide(&GameView { me: &me, ball: &ball }).kick, "자기 골 방향이면 무슛");
    }

    /// 공이 사거리 밖이면 안 참.
    #[test]
    fn ai_does_not_kick_when_ball_far() {
        let (me, ball) = view_at(Team::Blue, Vec2 { x: 0.0, y: 0.0 }, 0.0, Vec2 { x: 3.0, y: 0.0 });
        let mut ai = ChaseBallAi::default();
        assert!(!ai.decide(&GameView { me: &me, ball: &ball }).kick, "사거리 밖 무슛");
    }

    /// 정상 주행(속도 충분)에서는 스턱 판정이 되지 않아야 한다(오탐 방지).
    #[test]
    fn moving_normally_never_escapes() {
        let robot = RobotState {
            id: Team::Blue,
            pos: Vec2 { x: 0.0, y: 0.0 },
            rot: 0.0,
            vel: Vec2 { x: 3.0, y: 0.0 }, // 충분한 속도
            robot: String::new(),
            parts: Vec::new(),
            down: Down::default(),
            st: Vec::new(),
            stamina: 1.0,
        };
        let ball = BallState {
            pos: Vec2 { x: 5.0, y: 0.0 },
            vel: Vec2 { x: 0.0, y: 0.0 },
        };
        let view = GameView { me: &robot, ball: &ball };
        let mut ai = ChaseBallAi::default();
        for _ in 0..(STUCK_LIMIT * 3) {
            assert!(ai.decide(&view).thrust > 0.0, "정상 주행 중엔 항상 전진");
        }
    }

    // -- DefenderAi(KB-57: 협동 AI 역할 분담) -----------------------------------

    /// 공이 상대 진영 멀리 있을 때, 수비형의 목표 지점은 자기 골에서
    /// DEFENDER_GUARD_DIST 이내여야 한다(뭉치지 않고 골 앞을 지킴).
    #[test]
    fn defender_target_stays_near_own_goal_when_ball_is_far() {
        let ball = BallState {
            pos: Vec2 { x: 5.0, y: 1.0 }, // 블루 기준 상대 진영 멀리
            vel: Vec2 { x: 0.0, y: 0.0 },
        };
        let (tx, ty) = DefenderAi::guard_target(Team::Blue, &ball);
        let own_goal_x = -FIELD_W / 2.0;
        let dist = ((tx - own_goal_x).powi(2) + ty.powi(2)).sqrt();
        assert!(
            dist <= GUARD_HOME + 1e-3,
            "공이 멀면 목표는 자기 골 근처에 머물러야 함 (target=({tx},{ty}), dist={dist})"
        );
    }

    /// 공격형은 공이 아무리 멀어도 공 자체를 목표로 조준(뭉치지 않게 역할이 다름을 구분).
    /// 대조: 같은 상황에서 수비형의 목표는 자기 골 근처(위 테스트)에 머무른다.
    #[test]
    fn attacker_targets_ball_directly_unlike_defender() {
        let (me, ball) = view_at(Team::Blue, Vec2 { x: -4.0, y: 0.0 }, 0.0, Vec2 { x: 5.0, y: 1.0 });
        let mut attacker = ChaseBallAi::default();
        let out = attacker.decide(&GameView { me: &me, ball: &ball });
        assert!(out.thrust > 0.0, "공격형은 항상 공 쪽으로 전진");
        // rot=0이므로 turn은 공 방향각(클램프)과 정확히 일치해야 한다.
        let to_ball_angle = (ball.pos.y - me.pos.y).atan2(ball.pos.x - me.pos.x);
        assert_eq!(out.turn, to_ball_angle.clamp(-1.0, 1.0), "공격형 조준각은 공 방향과 일치해야 함");
    }

    /// 공이 자기 골 가까이 오면 수비형은 나가서 막는다(전진).
    #[test]
    fn defender_advances_when_ball_is_close_to_own_goal() {
        let (me, ball) = view_at(
            Team::Blue,
            Vec2 { x: -6.0, y: 0.0 },
            0.0,
            Vec2 { x: -5.0, y: 0.5 }, // 자기 골(-6.0) 근처
        );
        let mut defender = DefenderAi::default();
        let out = defender.decide(&GameView { me: &me, ball: &ball });
        assert!(out.thrust > 0.0, "공이 가까우면 나가서 막아야 함");
    }

    /// 목표에 도착하면 제자리서 돌지 않고(회전 데드존) 전진을 멈추고 공을 바라본다(KB-59).
    /// 블루 수비형을 guard_target(=(-3.5,0))에 공(+x) 쪽으로 배치 → thrust 0, turn≈0.
    #[test]
    fn defender_holds_without_spinning_when_arrived() {
        let (me, ball) = view_at(Team::Blue, Vec2 { x: -3.5, y: 0.0 }, 0.0, Vec2 { x: 5.0, y: 0.0 });
        let mut d = DefenderAi::default();
        let out = d.decide(&GameView { me: &me, ball: &ball });
        assert_eq!(out.thrust, 0.0, "도착하면 전진을 멈춰야 함");
        assert!(
            out.turn.abs() < 1e-3,
            "이미 공을 향하면 회전하지 않아야 함(제자리 스핀 방지) (turn={})",
            out.turn
        );
    }

    /// 수비형도 정면 사거리+상대 골 정렬이면 클리어 슛(공격형과 동일 조건 공유).
    #[test]
    fn defender_kicks_when_ball_in_front_toward_enemy_goal() {
        let (me, ball) = view_at(Team::Blue, Vec2 { x: 0.0, y: 0.0 }, 0.0, Vec2 { x: 0.6, y: 0.0 });
        let mut ai = DefenderAi::default();
        assert!(ai.decide(&GameView { me: &me, ball: &ball }).kick, "정렬+사거리 내 → 슛");
    }

    /// 수비형도 자책골 방향이면 안 참(공격형과 동일 회피 로직 공유).
    #[test]
    fn defender_does_not_kick_toward_own_goal() {
        let (me, ball) = view_at(Team::Blue, Vec2 { x: 0.0, y: 0.0 }, std::f32::consts::PI, Vec2 { x: -0.6, y: 0.0 });
        let mut ai = DefenderAi::default();
        assert!(!ai.decide(&GameView { me: &me, ball: &ball }).kick, "자기 골 방향이면 무슛");
    }

    // -- DefenderAi 방어범위 확대 + 조기인지(2026-07-14 설계) -----------------------

    /// 공(예측 위치)이 상대 진영이면 목표는 GUARD_HOME 이내로 유지(범위 확대 전과 동일).
    #[test]
    fn defender_guard_home_when_predicted_in_enemy_half() {
        let ball = BallState {
            pos: Vec2 { x: 5.0, y: 1.0 }, // 블루 기준 상대 진영, 정지
            vel: Vec2 { x: 0.0, y: 0.0 },
        };
        let (tx, ty) = DefenderAi::guard_target(Team::Blue, &ball);
        let own_goal_x = -FIELD_W / 2.0;
        let dist = ((tx - own_goal_x).powi(2) + ty.powi(2)).sqrt();
        assert!(dist <= GUARD_HOME + 1e-3, "상대 진영이면 GUARD_HOME 이내여야 함 (dist={dist})");
    }

    /// 공(예측 위치)이 자기 진영 멀리면 목표가 GUARD_HOME을 넘어 GUARD_ENGAGE까지 확장돼야 한다.
    #[test]
    fn defender_guard_extends_to_engage_when_predicted_in_own_half() {
        let ball = BallState {
            pos: Vec2 { x: -1.0, y: 0.0 }, // 블루 기준 자기 진영, 골에서 5.0 거리, 정지
            vel: Vec2 { x: 0.0, y: 0.0 },
        };
        let (tx, ty) = DefenderAi::guard_target(Team::Blue, &ball);
        let own_goal_x = -FIELD_W / 2.0;
        let dist = ((tx - own_goal_x).powi(2) + ty.powi(2)).sqrt();
        assert!(
            dist > GUARD_HOME + 1e-3,
            "자기 진영이면 GUARD_HOME을 넘어 확장돼야 함 (dist={dist})"
        );
        assert!(
            dist <= GUARD_ENGAGE + 1e-3,
            "확장 상한은 GUARD_ENGAGE (dist={dist})"
        );
    }

    /// 속도 예측(조기 인지): 정지한 공과 달리, 자기 진영으로 굴러오는 공은 아직 상대 진영에
    /// 있어도 예측 위치가 자기 진영으로 넘어가 목표가 더 멀리(GUARD_ENGAGE 쪽으로) 확장돼야 한다.
    #[test]
    fn defender_lookahead_extends_target_for_incoming_ball_before_it_crosses() {
        let pos = Vec2 { x: 0.3, y: 0.0 }; // 아직 상대 진영(x>0)
        let stationary = BallState { pos, vel: Vec2 { x: 0.0, y: 0.0 } };
        let incoming = BallState { pos, vel: Vec2 { x: -6.0, y: 0.0 } }; // 자기 진영으로 굴러옴
        let own_goal_x = -FIELD_W / 2.0;

        let (sx, sy) = DefenderAi::guard_target(Team::Blue, &stationary);
        let (ix, iy) = DefenderAi::guard_target(Team::Blue, &incoming);
        let sdist = ((sx - own_goal_x).powi(2) + sy.powi(2)).sqrt();
        let idist = ((ix - own_goal_x).powi(2) + iy.powi(2)).sqrt();
        assert!(
            idist > sdist + 1e-3,
            "굴러오는 공은 조기 인지로 목표가 더 멀리 확장돼야 함 (stationary={sdist}, incoming={idist})"
        );
    }

    /// 자살골 회귀(KB-61 조사): 공이 골 근처에서 주로 **접선 방향**(골라인과 나란히,
    /// 예: y로 빠르게 흐르며 x로는 살짝만 접근)으로 구를 때, LOOKAHEAD를 그대로
    /// 외삽하면 예측 지점이 골에서 유클리드 거리로 공의 **현재** 위치보다 더 멀어질 수
    /// 있다(접선 성분이 커서). 이 경우 목표가 공의 실제 위치보다 골에서 더 멀어지면
    /// 수비형이 골 쪽에 자리잡지 못하고 공 뒤(미드필드 쪽)에서 쫓아가며 결과적으로
    /// 공을 자기 골 방향으로 계속 떠밀게 된다(재현: 4대 30초+ 헤드리스 시뮬레이션에서
    /// 관측). 목표는 공의 실제 위치보다 골에서 더 멀어져선 안 된다.
    #[test]
    fn defender_guard_target_never_farther_from_goal_than_balls_actual_position() {
        let ball = BallState {
            pos: Vec2 { x: -5.66, y: -0.03 }, // 블루 자기 골(-6,0) 코앞
            vel: Vec2 { x: -0.45, y: -2.7 },  // 접선(y) 성분이 압도적으로 큼
        };
        let (tx, ty) = DefenderAi::guard_target(Team::Blue, &ball);
        let own_goal_x = -FIELD_W / 2.0;
        let target_dist = ((tx - own_goal_x).powi(2) + ty.powi(2)).sqrt();
        let ball_dist = ((ball.pos.x - own_goal_x).powi(2) + ball.pos.y.powi(2)).sqrt();
        assert!(
            target_dist <= ball_dist + 1e-3,
            "목표가 공의 실제 위치보다 골에서 더 멀면 안 됨 (target_dist={target_dist}, ball_dist={ball_dist})"
        );
    }

    /// 수비형도 벽에 박혀 정지 지속 시 후진 탈출 기동으로 전환해야 한다(공격형과 공유 로직).
    #[test]
    fn defender_stuck_against_wall_triggers_reverse_escape() {
        let robot = RobotState {
            id: Team::Blue,
            pos: Vec2 { x: 5.5, y: 3.5 },
            rot: 0.0,
            vel: Vec2 { x: 0.0, y: 0.0 },
            robot: String::new(),
            parts: Vec::new(),
            down: Down::default(),
            st: Vec::new(),
            stamina: 1.0,
        };
        let ball = BallState {
            pos: Vec2 { x: 6.5, y: 4.5 },
            vel: Vec2 { x: 0.0, y: 0.0 },
        };
        let view = GameView { me: &robot, ball: &ball };
        let mut ai = DefenderAi::default();
        let mut escaped = false;
        for _ in 0..(STUCK_LIMIT + ESCAPE_FRAMES + 5) {
            if ai.decide(&view).thrust < 0.0 {
                escaped = true;
                break;
            }
        }
        assert!(escaped, "정지 지속 시 수비형도 후진 탈출 기동이 나와야 함");
    }
}
