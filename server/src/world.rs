use serde::Serialize;

pub const FIELD_W: f32 = 12.0; // meters
pub const FIELD_H: f32 = 8.0;
pub const GOAL_W: f32 = 2.4;
pub const DT: f32 = 1.0 / 60.0; // fixed timestep

/// 킥오프 로봇 배치 (x, rot) — index 0 = Blue, 1 = Red. 단일 소스.
/// physics(new_kickoff_with)와 GameState::new_kickoff가 공유한다.
/// (2대 레거시 레이아웃 — 다수의 물리 테스트가 의존하므로 값을 바꾸지 않는다.)
/// 실행 바이너리는 `PhysicsWorld::new_match`(4대)를 쓰므로 테스트 전용.
#[cfg(test)]
pub const KICKOFF: [(f32, f32); 2] = [(-3.0, 0.0), (3.0, std::f32::consts::PI)];

/// 4대 매치(팀당 공격형+수비형) 킥오프 배치 (x, y, rot). 단일 소스 —
/// `PhysicsWorld::new_match`가 로봇 생성과 득점 후 리셋(`reset_kickoff`) 모두에 쓴다.
/// 로스터(고정): 0=Blue striker, 1=Blue guard, 2=Red striker, 3=Red guard.
/// Blue는 왼쪽(x<0)/Red는 오른쪽(x>0), 공격형은 중앙 쪽, 수비형은 자기 골 쪽에 배치하고
/// y를 서로 어긋나게 둬 킥오프 직후 곧바로 아군끼리 부딪히지 않게 한다.
/// 튜닝 여지: 좌표/간격은 플레이테스트 대상.
pub const MATCH_KICKOFF: [(f32, f32, f32); 4] = [
    (-2.0, -1.2, 0.0),                 // 0: Blue striker
    (-4.2, 1.2, 0.0),                  // 1: Blue guard
    (2.0, 1.2, std::f32::consts::PI),  // 2: Red striker
    (4.2, -1.2, std::f32::consts::PI), // 3: Red guard
];

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

/// 파손 다운 상태(스냅샷 디버프). `repair_in`=리페어까지 남은 초.
#[derive(Clone, Serialize, Default)]
pub struct Down {
    pub broken: bool,
    pub repair_in: f32,
}

// Copy 불가: `robot: String`/Vec 필드 때문에 Clone만 파생(스냅샷 클론에 충분).
#[derive(Clone, Serialize)]
pub struct RobotState {
    pub id: Team,
    pub pos: Vec2,
    pub rot: f32,
    pub vel: Vec2,
    /// 로드아웃/프리셋 id (스냅샷에 additive; 기존 필드 불변).
    pub robot: String,
    /// 부위별 (부위명, HP비율 0..1).
    pub parts: Vec<(String, f32)>,
    /// 파손 다운 상태.
    pub down: Down,
    /// 상태이상 태그(3b: 파손 다운 시 `["downed"]`, 그 외 빈 벡터).
    pub st: Vec<String>,
    /// 스태미나 비율 0..1(KB-45). 용량 없는 로봇은 항상 1.0.
    pub stamina: f32,
}

#[derive(Clone, Copy, Serialize)]
pub struct BallState {
    pub pos: Vec2,
    pub vel: Vec2,
}

#[derive(Clone, Copy, PartialEq, Debug, Serialize)]
pub enum Team {
    Blue,
    Red,
}

#[derive(Clone, Serialize)]
pub struct GameState {
    pub robots: Vec<RobotState>,
    pub ball: BallState,
    pub score: (u32, u32),
    pub time: f32,
    /// 슬롯별 조종 주체(KB-55): "human"|"ai". 로봇 수와 동일 길이,
    /// 인덱스 0=Blue, 1=Red. physics는 소유자를 모르므로 항상 "ai"를 채우고,
    /// sim 루프가 브로드캐스트 직전 사람 점유 슬롯을 "human"으로 덮어쓴다.
    pub ctrl: Vec<String>,
}

/// 컨트롤러가 보는 읽기 전용 뷰
pub struct GameView<'a> {
    pub me: &'a RobotState,
    pub ball: &'a BallState,
}

/// 컨트롤러가 내는 명령(액추에이터 층)
#[derive(Clone, Copy, Default)]
pub struct ControlOutput {
    pub thrust: f32,
    pub turn: f32, // -1..1
    /// 달리기(Shift 홀드) 요청(KB-45). AI는 항상 false(달리기 미사용, YAGNI).
    pub run: bool,
    /// 차기(킥) 요청(KB-48, 모드리스 탭). 서버가 로봇별 이전 값과 비교해
    /// **false→true 상승엣지에서만** 1회 발사(홀드해도 반복 없음). AI는 항상 false.
    pub kick: bool,
}

impl GameState {
    /// 2대 레거시 킥오프 상태(테스트 전용 — 실행 바이너리는 `PhysicsWorld::new_match`의
    /// 스냅샷으로 4대 상태를 시드한다).
    #[cfg(test)]
    pub fn new_kickoff() -> Self {
        let robots: Vec<RobotState> = KICKOFF
            .iter()
            .enumerate()
            .map(|(i, &(x, rot))| RobotState {
                id: if i == 0 { Team::Blue } else { Team::Red },
                pos: Vec2 { x, y: 0.0 },
                rot,
                vel: Vec2 { x: 0.0, y: 0.0 },
                robot: String::new(),
                parts: Vec::new(),
                down: Down::default(),
                st: Vec::new(),
                stamina: 1.0,
            })
            .collect();
        let ctrl = vec!["ai".to_string(); robots.len()];
        GameState {
            robots,
            ball: BallState {
                pos: Vec2 { x: 0.0, y: 0.0 },
                vel: Vec2 { x: 0.0, y: 0.0 },
            },
            score: (0, 0),
            time: 0.0,
            ctrl,
        }
    }
}

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
