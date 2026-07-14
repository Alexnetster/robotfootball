use crate::combat::CombatState;
use crate::parts::{self, StatSet};
use crate::stamina::StaminaState;
use crate::world::*;
use rapier2d::prelude::*;
use std::collections::HashMap;

const WALL_T: f32 = 0.2; // 벽 두께
const BALL_R: f32 = 0.2;
/// 공 속도 상한(m/s, KB-58). 킥/밀기/넉백 합산 과속으로 인한 벽 터널링·이탈 방지.
/// 강킥이 이 근처가 되도록 kick_power를 맞춤(세기 gradient는 이 아래에서 유지).
const BALL_MAX_SPEED: f32 = 12.0;
const RESTITUTION: f32 = 0.85;

/// 충돌 그룹(KB-43): 골 입구는 공만 통과시키고 로봇은 막는 "펜스"가 필요해
/// 멤버십별로 필터를 분리한다. 상호작용 규칙(rapier):
/// `(A.mem & B.filter)!=0 && (B.mem & A.filter)!=0`.
mod groups {
    use rapier2d::prelude::Group;
    pub const BALL: Group = Group::GROUP_1;
    pub const ROBOT: Group = Group::GROUP_2;
    pub const SOLID: Group = Group::GROUP_3; // 상/하 벽 + 좌우 벽 조각(기존 고정 경계)
    pub const GOALFENCE: Group = Group::GROUP_4; // 골 입구를 메우는 로봇 전용 펜스
}

/// 로봇 부위 수. 부위별 자식 콜라이더(복합 바디)로 구성.
/// 차기(킥, KB-48) 튜닝 상수. 결정적 순수 계산에만 쓰임(I/O·RNG 없음).
mod kick {
    /// 정면 사거리(로봇 반경+공 반경+여유). 대략 0.9m 근방(튜닝 대상).
    pub const RANGE: f32 = 0.9;
    /// 정면 콘 반각 75°의 cos값. dot(facing, unit(ball-robot)) ≥ 이 값이어야 유효.
    pub const CONE_COS: f32 = 0.258_819_05; // cos(75°)
    /// 좌우 조준 오프셋(라디안). 0.61rad ≈ 35°.
    pub const AIM: f32 = 0.61;
    /// 쿨다운(초).
    pub const CD: f32 = 0.45;
    /// 세기 3단(상하 입력 기준). 최종 임펄스 크기 = 레벨 × kick_power.
    pub const STRONG: f32 = 1.0; // thrust>0(↑)
    pub const MID: f32 = 0.75; // thrust==0
    pub const WEAK: f32 = 0.5; // thrust<0(↓)
}

const NUM_PARTS: usize = 3;
/// 부위명(스냅샷 `parts`용). PART_SHAPES/PART_HP_WEIGHT와 index 정합.
const PART_NAMES: [&str; NUM_PARTS] = ["body", "foreleg", "hindleg"];
/// 부위 콜라이더 (반폭 hx, 반높이 hy, 로컬 오프셋 ox, oy). 앞(+x)=전진 방향.
/// 합집합 x∈[-0.25,0.25], y∈[-0.2,0.2] — 기존 단일 큐보이드(ROBOT_HX/HY) 풋프린트 근사.
const PART_SHAPES: [(f32, f32, f32, f32); NUM_PARTS] = [
    (0.12, 0.20, 0.0, 0.0),   // body(중심)
    (0.09, 0.15, 0.16, 0.0),  // foreleg(앞)
    (0.09, 0.15, -0.16, 0.0), // hindleg(뒤)
];
/// 부위별 최대 HP = 기저치 + 로봇 총 hp × 가중치. 기저치로 항상 양수(0-HP 오다운 방지).
const PART_HP_WEIGHT: [f32; NUM_PARTS] = [0.5, 0.25, 0.25];
const PART_HP_BASE: f32 = 10.0;

fn part_hps(total_hp: f32) -> Vec<f32> {
    PART_HP_WEIGHT
        .iter()
        .map(|w| PART_HP_BASE + total_hp.max(0.0) * w)
        .collect()
}

/// user_data(u128) 태깅: 상위 64비트=robot_idx, 하위=part_idx. (physics.rs 경계 전용)
fn tag(robot: usize, part: usize) -> u128 {
    ((robot as u128) << 64) | (part as u128)
}

/// 충돌 쌍이 데미지 대상인지 판정(순수). 둘 다 로봇 부위(Some)이고 **서로 다른 로봇**일 때만.
/// 벽/공(None)·같은 로봇 부위 쌍은 무데미지. 결정성 위해 (robot,part) 오름차순 정규화.
/// 팀(친선 데미지) 판정은 팀 정보가 없는 이 순수 함수가 아니라 호출부
/// (`apply_collision_damage`, `self.teams` 접근 가능)에서 추가로 필터링한다.
fn damage_pair(
    a: Option<(usize, usize)>,
    b: Option<(usize, usize)>,
) -> Option<((usize, usize), (usize, usize))> {
    let (a, b) = (a?, b?);
    if a.0 == b.0 {
        return None; // 같은 로봇(자기 부위끼리) → 무데미지
    }
    Some(if a <= b { (a, b) } else { (b, a) })
}

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
    stats: Vec<StatSet>,
    preset_ids: Vec<String>,
    /// 로봇별 팀(로스터 고정). 친선 데미지 금지 판정 + 스냅샷 `id`에 쓰인다.
    teams: Vec<Team>,
    /// 득점 후 리셋 배치(x, y, rot). 생성 시 저장해 `reset_kickoff`가 로봇 수(2/4)에
    /// 무관하게 재사용한다(KB-57: `world::KICKOFF` const 직접 의존 제거).
    kickoff_layout: Vec<(f32, f32, f32)>,
    /// 로봇 부위 콜라이더 멤버십+디코드: handle → (robot_idx, part_idx).
    /// 벽/공은 부재 → 오데미지 방지(둘 다 멤버인 쌍만 데미지).
    part_map: HashMap<ColliderHandle, (usize, usize)>,
    /// 로봇별 부위 HP·파손 다운 상태(결정적 순수 로직 combat.rs).
    combat: Vec<CombatState>,
    /// 로봇별 스태미나 상태(결정적 순수 로직 stamina.rs, KB-45).
    stamina: Vec<StaminaState>,
    /// 차기(킥, KB-48): 직전 스텝의 `kick` 입력값(상승엣지 판정용).
    prev_kick: Vec<bool>,
    /// 차기 쿨다운 잔여초(로봇별). >0인 동안 재발사 불가·스냅샷 "shoot_lock".
    kick_cd: Vec<f32>,
    pub score: (u32, u32),
    pub time: f32,
}

impl PhysicsWorld {
    /// 기본 스탯(기존 하드코딩 등가)으로 위임 — 기존 물리/골/tick 테스트 보존.
    /// 실행 바이너리는 `new_match`(4대 로스터)를 쓰므로 테스트 전용.
    #[cfg(test)]
    pub fn new_kickoff() -> Self {
        use crate::parts::default_stats;
        Self::new_kickoff_with(
            [default_stats(), default_stats()],
            [String::new(), String::new()],
        )
    }

    /// 로봇별 스탯/프리셋 id를 받아 **2대** 킥오프 월드를 만든다(기존 시그니처·동작
    /// 유지 — 수많은 물리 테스트가 이 2대 레이아웃에 의존하므로 절대 깨면 안 됨).
    /// 배치는 `world::KICKOFF`(y=0 고정) 단일 소스. index 0=Blue, 1=Red(서로 다른
    /// 팀이라 친선 데미지 금지 규칙과 무관하게 기존처럼 상호 데미지가 적용된다).
    /// `stats[i].mass`는 콜라이더 밀도 유래 질량에 **가산**(mass=0=no-op).
    /// 실행 바이너리는 `new_match`(4대 로스터)를 쓰므로 테스트 전용.
    #[cfg(test)]
    pub fn new_kickoff_with(stats: [StatSet; 2], preset_ids: [String; 2]) -> Self {
        let layout: Vec<(f32, f32, f32)> = KICKOFF.iter().map(|&(x, rot)| (x, 0.0, rot)).collect();
        Self::build(
            stats.to_vec(),
            preset_ids.to_vec(),
            layout,
            vec![Team::Blue, Team::Red],
        )
    }

    /// 팀당 공격형(striker)+수비형(guard) 1대씩, 총 **4대**(로스터 고정)로 매치 월드를
    /// 만든다. 로스터: 0=Blue striker, 1=Blue guard, 2=Red striker, 3=Red guard
    /// (`world::MATCH_KICKOFF` 배치와 정합). 모델 선택 없음(프리셋 고정) — 실행
    /// 바이너리(main)가 이 생성자를 쓴다.
    pub fn new_match() -> Self {
        let cat = parts::catalog();
        let stats = vec![
            parts::aggregate(&cat, "striker"), // 0: Blue striker
            parts::aggregate(&cat, "guard"),   // 1: Blue guard
            parts::aggregate(&cat, "striker"), // 2: Red striker
            parts::aggregate(&cat, "guard"),   // 3: Red guard
        ];
        let preset_ids = vec![
            "striker".to_string(),
            "guard".to_string(),
            "striker".to_string(),
            "guard".to_string(),
        ];
        let teams = vec![Team::Blue, Team::Blue, Team::Red, Team::Red];
        let layout: Vec<(f32, f32, f32)> = MATCH_KICKOFF.to_vec();
        Self::build(stats, preset_ids, layout, teams)
    }

    /// 공통 빌더: 로봇 수(N=2 또는 4)에 무관하게 필드/펜스/파츠 콜라이더를 구성한다.
    /// `layout[i]`=(x,y,rot) 초기 배치(리셋 시 재사용을 위해 `kickoff_layout`에 저장),
    /// `teams[i]`=팀(친선 데미지 판정 + 스냅샷 `id`에 쓰임).
    fn build(
        stats: Vec<StatSet>,
        preset_ids: Vec<String>,
        layout: Vec<(f32, f32, f32)>,
        teams: Vec<Team>,
    ) -> Self {
        assert_eq!(stats.len(), layout.len(), "스탯 수와 배치 수는 일치해야 함");
        assert_eq!(stats.len(), teams.len(), "스탯 수와 팀 수는 일치해야 함");
        let mut bodies = RigidBodySet::new();
        let mut colliders = ColliderSet::new();

        let hw = FIELD_W / 2.0;
        let hh = FIELD_H / 2.0;

        // SOLID(상/하/좌우 고정 경계)는 모두와 충돌(ALL) — 공/로봇 모두 막아야 함.
        let solid_groups = InteractionGroups::new(groups::SOLID, Group::ALL);

        // 상/하 벽 (고정)
        for (hx, hy, x, y) in [(hw, WALL_T, 0.0, hh), (hw, WALL_T, 0.0, -hh)] {
            colliders.insert(
                ColliderBuilder::cuboid(hx, hy)
                    .translation(vector![x, y])
                    .restitution(RESTITUTION)
                    .collision_groups(solid_groups)
                    .build(),
            );
        }

        // 좌우 벽: 골 입구(y ∈ [−GOAL_W/2, GOAL_W/2])를 비운 위/아래 두 조각
        for side in [hw, -hw] {
            let seg = (hh - GOAL_W / 2.0) / 2.0; // 각 조각 반높이
            let cy = GOAL_W / 2.0 + seg; // 조각 중심 y
            for sy in [cy, -cy] {
                colliders.insert(
                    ColliderBuilder::cuboid(WALL_T, seg)
                        .translation(vector![side, sy])
                        .restitution(RESTITUTION)
                        .collision_groups(solid_groups)
                        .build(),
                );
            }
        }

        // 코너 챔퍼(KB-44): 직각(90°) 코너에 공이 끼어 로봇이 빼낼 수 없는 교착을
        // 막기 위해 4개 코너마다 45° 대각 벽을 추가해 팔각형에 가까운 필드로 만든다.
        // SOLID 그룹(벽과 동일)이라 공과 로봇 모두 튕겨나간다.
        // 골 입구는 좌우 벽 중앙(|y| ≤ GOAL_W/2 ≈ 1.2)이라 hh(4.0) 근처인 코너와는
        // 충분히 떨어져 있어 골문에는 영향이 없다.
        let chamfer = 1.0; // 각 벽을 따라 잘라내는 길이(대각선 폭이 아님). 0.8~1.2m 권장 범위 내.
        let chamfer_half_len = chamfer * std::f32::consts::SQRT_2 / 2.0;
        for sx in [1.0f32, -1.0] {
            for sy in [1.0f32, -1.0] {
                let cx = sx * (hw - chamfer / 2.0);
                let cy = sy * (hh - chamfer / 2.0);
                // 사각형은 180° 회전 대칭이라 부호(sx*sy)만으로 네 코너 모두의
                // 대각 방향(±45°)을 얻을 수 있다.
                let angle = -sx * sy * std::f32::consts::FRAC_PI_4;
                colliders.insert(
                    ColliderBuilder::cuboid(chamfer_half_len, WALL_T)
                        .translation(vector![cx, cy])
                        .rotation(angle)
                        .restitution(RESTITUTION)
                        .collision_groups(solid_groups)
                        .build(),
                );
            }
        }

        // 공 (동적). CCD 활성(KB-58): 강킥 시 한 스텝 이동량이 벽 두께를 넘어 터널링하는
        // 것을 rapier 연속충돌검출로 방지.
        let ball = bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(vector![0.0, 0.0])
                .linear_damping(0.4)
                .ccd_enabled(true)
                .build(),
        );
        // BALL: SOLID(벽) + ROBOT(드리블)과는 충돌하되 GOALFENCE는 무시(골 입구 통과).
        let ball_groups =
            InteractionGroups::new(groups::BALL, groups::SOLID | groups::ROBOT);
        colliders.insert_with_parent(
            ColliderBuilder::ball(BALL_R)
                .restitution(RESTITUTION)
                .collision_groups(ball_groups)
                .build(),
            ball,
            &mut bodies,
        );

        // 로봇 N대 (배치는 인자 layout 단일 소스)
        // 각 로봇 = 부위별 자식 콜라이더 복합 바디. user_data 태깅 + part_map 멤버십.
        let mut robots = Vec::new();
        let mut part_map: HashMap<ColliderHandle, (usize, usize)> = HashMap::new();
        let mut combat = Vec::new();
        let mut stamina = Vec::new();
        for (i, &(x, y, rot)) in layout.iter().enumerate() {
            let rb = bodies.insert(
                RigidBodyBuilder::dynamic()
                    .translation(vector![x, y])
                    .rotation(rot)
                    // 조작감 튜닝(1차): 2.0→2.6. 재방향 시 잔여 미끄럼을 더 빨리 죽여
                    // "회전-후-전진"이 덜 미끄럽게. 너무 높이면 관성이 사라져 뻣뻣해짐.
                    .linear_damping(2.6)
                    // 회전은 apply_controls에서 set_angvel(rate 제어)로 매 스텝 덮어써
                    // angular_damping 효과는 사실상 미미 (조작감 튜닝 여지로만 유지).
                    .angular_damping(4.0)
                    // 콜라이더 밀도 유래 질량에 가산(스탯 mass; 0=no-op).
                    .additional_mass(stats[i].mass)
                    .build(),
            );
            // ROBOT: 전부와 충돌(SOLID/ROBOT/GOALFENCE/BALL) — 전투/드리블/펜스 모두 보존.
            let robot_groups = InteractionGroups::new(
                groups::ROBOT,
                groups::SOLID | groups::ROBOT | groups::GOALFENCE | groups::BALL,
            );
            for (p, &(hx, hy, ox, oy)) in PART_SHAPES.iter().enumerate() {
                let ch = colliders.insert_with_parent(
                    ColliderBuilder::cuboid(hx, hy)
                        .translation(vector![ox, oy])
                        .active_events(ActiveEvents::COLLISION_EVENTS)
                        .user_data(tag(i, p))
                        .collision_groups(robot_groups)
                        .build(),
                    rb,
                    &mut bodies,
                );
                part_map.insert(ch, (i, p));
            }
            combat.push(CombatState::new(&part_hps(stats[i].hp)));
            stamina.push(StaminaState::new(stats[i].stamina_max, stats[i].stamina_regen));
            robots.push(rb);
        }

        // 골 입구 "펜스"(KB-43): 골 입구 틈을 메우되 로봇만 막고 공은 통과시킨다.
        // GOALFENCE.filter=ROBOT → 로봇과만 상호작용, 공(BALL 멤버십)과는 상호작용 없음.
        // (공/로봇 핸들 생성 순서를 기존과 동일하게 유지하기 위해 펜스는 맨 마지막에 삽입)
        let fence_groups = InteractionGroups::new(groups::GOALFENCE, groups::ROBOT);
        for side in [hw, -hw] {
            colliders.insert(
                ColliderBuilder::cuboid(WALL_T, GOAL_W / 2.0)
                    .translation(vector![side, 0.0])
                    .restitution(RESTITUTION)
                    .collision_groups(fence_groups)
                    .build(),
            );
        }

        let n = robots.len();
        PhysicsWorld {
            bodies,
            colliders,
            gravity: vector![0.0, 0.0],
            params: IntegrationParameters {
                dt: DT,
                ..Default::default()
            },
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad: DefaultBroadPhase::new(),
            narrow: NarrowPhase::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            query: QueryPipeline::new(),
            ball,
            robots,
            stats,
            preset_ids,
            teams,
            kickoff_layout: layout,
            part_map,
            combat,
            stamina,
            prev_kick: vec![false; n],
            kick_cd: vec![0.0; n],
            score: (0, 0),
            time: 0.0,
        }
    }

    /// 로봇당 부위 콜라이더 수(테스트/디버그).
    #[cfg(test)]
    pub fn robot_part_count(&self) -> usize {
        NUM_PARTS
    }

    fn apply_controls(&mut self, controls: &[ControlOutput]) {
        for (i, (h, c)) in self.robots.iter().zip(controls.iter()).enumerate() {
            // 파손 다운 또는 스턴 로봇은 입력 무시(행동불능). 몸체는 동적 유지 →
            // 넉백 임펄스는 여전히 적용됨(입력만 차단). 잔여 속도는 물리(감쇠)로 소멸.
            if self.combat[i].broken() || self.combat[i].stunned() {
                continue;
            }
            let st = &self.stats[i];
            // 달리기(KB-45): run 요청 + 스태미나 잔량이 있을 때만 sprint_speed 적용,
            // 소모. 스태미나 0이면 자동으로 walk(max_speed)로 폴백.
            let sprinting = c.run && self.stamina[i].has_stamina();
            // 회복 정책(KB-53): 스프린트=소모, 걷기(이동 입력 thrust≠0)=유지,
            // **자의적 이동 입력이 없을 때(가만히=쉬기)만 회복**. 회전만 하는 것은 이동으로
            // 치지 않아 회복 가능(제자리 조준 허용).
            if sprinting {
                self.stamina[i].drain(DT);
            } else if c.thrust == 0.0 {
                self.stamina[i].regen(DT);
            }
            let speed_cap = if sprinting { st.sprint_speed } else { st.max_speed };
            let rb = &mut self.bodies[*h];
            rb.set_angvel(c.turn * st.turn_rate, true);
            let angle = rb.rotation().angle();
            let dir = vector![angle.cos(), angle.sin()];
            rb.apply_impulse(dir * (c.thrust * st.accel * DT), true);
            // maxSpeed(또는 sprint_speed) 클램프 (impulse 적용 후)
            let v = *rb.linvel();
            let sp = (v.x * v.x + v.y * v.y).sqrt();
            if sp > speed_cap && sp > 0.0 {
                let k = speed_cap / sp;
                rb.set_linvel(vector![v.x * k, v.y * k], true);
            }
        }
    }

    /// 차기(킥, KB-48). 로봇별 `kick` 입력의 **false→true 상승엣지**에서만 1회 발사
    /// (홀드해도 반복 없음). 조건: 다운/스턴 아님(apply_controls와 동일 게이팅),
    /// 쿨다운 없음, 공이 정면 부채꼴 사거리 안(거리 ≤ RANGE, 정면 콘 CONE_COS 이내).
    /// 세기(상하 3단)×방향(좌우 ±AIM)은 그 순간의 이동입력으로 결정, 최종 크기는
    /// 레벨 × 로봇 kick_power(로드아웃 집계). 순수 계산 + rapier 임펄스 적용만(I/O 없음).
    fn apply_kicks(&mut self, controls: &[ControlOutput]) {
        let ball_pos = *self.bodies[self.ball].translation();
        for (i, c) in controls.iter().enumerate() {
            // 다운/스턴 로봇은 킥도 무동작(apply_controls와 동일 게이팅). 엣지 상태(prev_kick)도
            // 갱신하지 않는다 — 행동불능 중 눌린 입력은 존재하지 않았던 것으로 취급.
            if self.combat[i].broken() || self.combat[i].stunned() {
                continue;
            }
            let rising = c.kick && !self.prev_kick[i];
            self.prev_kick[i] = c.kick;
            if !rising || self.kick_cd[i] > 0.0 {
                continue;
            }
            // 로봇 translation/rotation을 먼저 복사(이 스코프 밖에서 self.bodies를
            // 다시 가변 인덱싱해야 하므로 불변 대여를 여기서 끝낸다).
            let (pos, rot) = {
                let rb = &self.bodies[self.robots[i]];
                (*rb.translation(), rb.rotation().angle())
            };
            let to_ball = ball_pos - pos;
            let dist = (to_ball.x * to_ball.x + to_ball.y * to_ball.y).sqrt();
            if dist > kick::RANGE || dist < 1e-6 {
                continue; // 사거리 밖(또는 완전히 겹침, 방향 미정)
            }
            let facing = vector![rot.cos(), rot.sin()];
            let unit_to_ball = vector![to_ball.x / dist, to_ball.y / dist];
            let dot = facing.x * unit_to_ball.x + facing.y * unit_to_ball.y;
            if dot < kick::CONE_COS {
                continue; // 정면 콘 밖
            }
            let level = if c.thrust > 0.0 {
                kick::STRONG
            } else if c.thrust < 0.0 {
                kick::WEAK
            } else {
                kick::MID
            };
            let aim = if c.turn > 0.0 {
                kick::AIM
            } else if c.turn < 0.0 {
                -kick::AIM
            } else {
                0.0
            };
            let kick_angle = rot + aim;
            let dir = vector![kick_angle.cos(), kick_angle.sin()];
            let magnitude = level * self.stats[i].kick_power;
            self.bodies[self.ball].apply_impulse(dir * magnitude, true);
            self.kick_cd[i] = kick::CD;
        }
    }

    pub fn step(&mut self, controls: &[ControlOutput]) {
        self.apply_controls(controls);
        self.apply_kicks(controls);
        // 공 속도 상한(KB-58): 킥/밀기/넉백으로 공이 과속하면 벽 터널링·필드 이탈 유발.
        // 적분(pipeline.step) 전에 캡을 걸어야 한 스텝 이동량이 제한됨.
        self.clamp_ball_speed();
        // 충돌 이벤트 채널: collision + (미사용) contact-force 둘 다 필요(rapier 재수출).
        let (cs, cr) = rapier2d::crossbeam::channel::unbounded();
        let (fs, _fr) = rapier2d::crossbeam::channel::unbounded();
        let ev = ChannelEventCollector::new(cs, fs);
        self.pipeline.step(
            &self.gravity,
            &self.params,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd,
            Some(&mut self.query),
            &(),
            &ev,
        );
        self.apply_collision_damage(&cr);
        for c in &mut self.combat {
            c.tick_status();
        }
        // 차기 쿨다운(KB-48): 매 스텝 DT만큼 감소, 0 클램프.
        for cd in &mut self.kick_cd {
            *cd = (*cd - DT).max(0.0);
        }
        self.check_goal();
        self.time += DT;
    }

    /// 로봇↔로봇 충돌 이벤트를 상호 데미지로 적용. 공/벽 접촉은 무데미지.
    /// 결정성: 한 스텝 다중 히트를 (rA,rB,pA,pB)로 정렬 후 적용(f32 비결합성 방지).
    fn apply_collision_damage(
        &mut self,
        cr: &rapier2d::crossbeam::channel::Receiver<CollisionEvent>,
    ) {
        // 1) 수집 + 필터(둘 다 로봇 부위 & 서로 다른 로봇) + 오름차순 정규화
        let mut hits: Vec<((usize, usize), (usize, usize))> = Vec::new();
        while let Ok(CollisionEvent::Started(h1, h2, _)) = cr.try_recv() {
            let a = self.part_map.get(&h1).copied();
            let b = self.part_map.get(&h2).copied();
            if let Some(pair) = damage_pair(a, b) {
                // 친선 데미지 금지(4대 로스터): 같은 팀 로봇끼리는 데미지/넉백/스턴 없음.
                let ((ra, _), (rb, _)) = pair;
                if self.teams[ra] != self.teams[rb] {
                    hits.push(pair);
                }
            }
        }
        // 2) 결정성 정렬
        hits.sort_by_key(|&((ra, pa), (rb, pb))| (ra, rb, pa, pb));
        // 3) 상호 효과 적용(데미지=3b 모델, 넉백/스턴=effect 프로필)
        for ((ra, pa), (rb, pb)) in hits {
            // impact = 두 로봇 바디 상대 linvel 크기(ADR-009 접촉 임펄스의 의도적 간소화).
            let impact = self.relative_speed(ra, rb);
            // 넉백/스턴/effect데미지: 공격 로봇의 effect 프로필 × impact × 피격 로봇 저항(방어).
            let eff_on_b =
                crate::combat::resolve_effects(&self.effect_profile(ra), impact, self.defense_of(rb));
            let eff_on_a =
                crate::combat::resolve_effects(&self.effect_profile(rb), impact, self.defense_of(ra));
            // 데미지: 기존 attack/defense 모델(3b) + effect 프로필 dmg_w(3c, 가산). KB-34.
            let dmg_a = crate::combat::damage_on_contact(&self.stats[rb], &self.stats[ra], impact);
            let dmg_b = crate::combat::damage_on_contact(&self.stats[ra], &self.stats[rb], impact);
            self.combat[ra].apply_damage(pa, dmg_a + eff_on_a.damage);
            self.combat[rb].apply_damage(pb, dmg_b + eff_on_b.damage);
            // 스턴(입력 무시 상태). 몸체는 동적 유지 → 넉백은 여전히 이동시킴.
            if eff_on_b.stun > 0.0 {
                self.combat[rb].apply_stun(eff_on_b.stun);
            }
            if eff_on_a.stun > 0.0 {
                self.combat[ra].apply_stun(eff_on_a.stun);
            }
            // 넉백 임펄스: 서로 밀어냄(a→b 방향으로 b, 반대로 a). 위치 동일 시 skip(NaN 방지).
            let pos_a = *self.bodies[self.robots[ra]].translation();
            let pos_b = *self.bodies[self.robots[rb]].translation();
            if eff_on_b.knockback > 0.0 {
                if let Some(dir) = unit_dir(pos_b - pos_a) {
                    self.bodies[self.robots[rb]].apply_impulse(dir * eff_on_b.knockback, true);
                }
            }
            if eff_on_a.knockback > 0.0 {
                if let Some(dir) = unit_dir(pos_a - pos_b) {
                    self.bodies[self.robots[ra]].apply_impulse(dir * eff_on_a.knockback, true);
                }
            }
        }
    }

    /// 로봇 r의 effect 프로필(StatSet 가중치 유래). 순수.
    fn effect_profile(&self, r: usize) -> crate::combat::EffectProfile {
        let s = &self.stats[r];
        crate::combat::EffectProfile {
            knockback: s.kb_w,
            stun: s.stun_w,
            damage: s.dmg_w,
        }
    }

    /// 로봇 r의 방어(effect 저항).
    fn defense_of(&self, r: usize) -> f32 {
        self.stats[r].defense
    }

    /// 두 로봇 바디의 상대 속도 크기.
    fn relative_speed(&self, ra: usize, rb: usize) -> f32 {
        let va = *self.bodies[self.robots[ra]].linvel();
        let vb = *self.bodies[self.robots[rb]].linvel();
        let d = va - vb;
        (d.x * d.x + d.y * d.y).sqrt()
    }

    /// 공 속도를 BALL_MAX_SPEED로 클램프(KB-58). 적분 전에 호출해 터널링 방지.
    fn clamp_ball_speed(&mut self) {
        let b = &mut self.bodies[self.ball];
        let v = *b.linvel();
        let sp = (v.x * v.x + v.y * v.y).sqrt();
        if sp > BALL_MAX_SPEED && sp > 0.0 {
            let k = BALL_MAX_SPEED / sp;
            b.set_linvel(vector![v.x * k, v.y * k], true);
        }
    }

    /// 공을 중앙으로 복귀(속도 0). 득점/이탈 안전망 공용.
    fn recenter_ball(&mut self) {
        let b = &mut self.bodies[self.ball];
        b.set_translation(vector![0.0, 0.0], true);
        b.set_linvel(vector![0.0, 0.0], true);
        b.set_angvel(0.0, true);
    }

    fn check_goal(&mut self) {
        let bp = *self.bodies[self.ball].translation();
        let half_w = FIELD_W / 2.0;
        let half_h = FIELD_H / 2.0;
        let in_mouth = bp.y.abs() <= GOAL_W / 2.0;
        if bp.x > half_w && in_mouth {
            self.score.0 += 1;
            self.reset_kickoff();
        } else if bp.x < -half_w && in_mouth {
            self.score.1 += 1;
            self.reset_kickoff();
        } else if bp.x.abs() > half_w + 1.0 || bp.y.abs() > half_h + 0.5 {
            // 안전망(KB-58): 골이 아닌데 공이 필드를 벗어남(터널링/이탈) → 득점 없이
            // 중앙 복귀. 공이 화면 밖으로 사라지고 AI가 그걸 쫓아 흩어지는 것을 막는다.
            self.recenter_ball();
        }
    }

    fn reset_kickoff(&mut self) {
        // 공
        let b = &mut self.bodies[self.ball];
        b.set_translation(vector![0.0, 0.0], true);
        b.set_linvel(vector![0.0, 0.0], true);
        b.set_angvel(0.0, true);
        // 로봇 (배치는 생성 시 저장한 kickoff_layout — 2대/4대 생성자 공용)
        for (h, &(x, y, rot)) in self.robots.iter().zip(self.kickoff_layout.iter()) {
            let rb = &mut self.bodies[*h];
            rb.set_translation(vector![x, y], true);
            rb.set_rotation(Rotation::new(rot), true);
            rb.set_linvel(vector![0.0, 0.0], true);
            rb.set_angvel(0.0, true);
        }
    }

    #[cfg(test)]
    pub fn kick_ball_for_test(&mut self, v: Vector<Real>) {
        self.bodies[self.ball].set_linvel(v, true);
    }

    #[cfg(test)]
    pub fn set_ball_for_test(&mut self, pos: Vector<Real>, vel: Vector<Real>) {
        let b = &mut self.bodies[self.ball];
        b.set_translation(pos, true);
        b.set_linvel(vel, true);
    }

    #[cfg(test)]
    pub fn set_robot_for_test(&mut self, i: usize, pos: Vector<Real>, rot: f32) {
        let rb = &mut self.bodies[self.robots[i]];
        rb.set_translation(pos, true);
        rb.set_rotation(Rotation::new(rot), true);
        rb.set_linvel(vector![0.0, 0.0], true);
        rb.set_angvel(0.0, true);
    }

    /// 로봇 i의 최소 부위 HP비율(테스트/디버그).
    #[cfg(test)]
    pub fn hp_ratio_min(&self, i: usize) -> f32 {
        self.combat[i].hp_ratio_min()
    }

    /// 로봇 i를 강제 파손 다운(테스트 전용).
    #[cfg(test)]
    pub fn force_break_for_test(&mut self, i: usize) {
        self.combat[i].force_down();
    }

    /// 로봇 i가 스턴 중인지(테스트 전용).
    #[cfg(test)]
    pub fn is_stunned_for_test(&self, i: usize) -> bool {
        self.combat[i].stunned()
    }

    /// 로봇 i를 강제 스턴(테스트 전용).
    #[cfg(test)]
    pub fn force_stun_for_test(&mut self, i: usize, secs: f32) {
        self.combat[i].apply_stun(secs);
    }

    pub fn snapshot(&self) -> GameState {
        let b = &self.bodies[self.ball];
        let ball = BallState {
            pos: to_vec2(b.translation()),
            vel: to_vec2(b.linvel()),
        };
        let robots = self
            .robots
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let rb = &self.bodies[*h];
                let cs = &self.combat[i];
                let parts = (0..cs.part_count())
                    .map(|p| (PART_NAMES[p].to_string(), cs.hp_ratio(p)))
                    .collect();
                let broken = cs.broken();
                // 상태이상 태그: 파손 다운("downed")과 스턴("stun")은 동시 가능.
                let mut st = Vec::new();
                if broken {
                    st.push("downed".to_string());
                }
                if cs.stunned() {
                    st.push("stun".to_string());
                }
                if self.kick_cd[i] > 0.0 {
                    st.push("shoot_lock".to_string());
                }
                RobotState {
                    id: self.teams[i],
                    pos: to_vec2(rb.translation()),
                    rot: rb.rotation().angle(), // rapier가 정규화된 각도 반환
                    vel: to_vec2(rb.linvel()),
                    robot: self.preset_ids[i].clone(),
                    parts,
                    down: Down {
                        broken,
                        repair_in: cs.repair_in(),
                    },
                    st,
                    stamina: self.stamina[i].ratio(),
                }
            })
            .collect();
        // 물리 레이어는 슬롯 소유자(사람/AI)를 모르므로 항상 "ai".
        // sim 루프(main.rs)가 브로드캐스트 직전 사람 점유 슬롯을 덮어쓴다(KB-55).
        let ctrl = (0..self.robots.len()).map(|_| "ai".to_string()).collect();
        GameState {
            robots,
            ball,
            score: self.score,
            time: self.time,
            ctrl,
        }
    }
}

fn to_vec2(v: &Vector<Real>) -> Vec2 {
    Vec2 { x: v.x, y: v.y }
}

/// 결정적 단위벡터. 길이가 0에 가까우면 None(넉백 skip으로 NaN 방지).
fn unit_dir(v: Vector<Real>) -> Option<Vector<Real>> {
    let n = (v.x * v.x + v.y * v.y).sqrt();
    if n > 1e-6 {
        Some(vector![v.x / n, v.y / n])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robot_has_multiple_tagged_part_colliders() {
        let w = PhysicsWorld::new_kickoff();
        // 로봇당 부위 콜라이더 ≥2 (복합 바디)
        assert!(w.robot_part_count() >= 2, "로봇당 부위 콜라이더 ≥2");
        // part_map 멤버십 = 로봇 수 × 부위 수, 모두 유효한 (robot,part) 디코드
        assert_eq!(w.part_map.len(), 2 * w.robot_part_count());
        assert!(w.part_map.values().all(|&(r, p)| r < 2 && p < NUM_PARTS));
    }

    #[test]
    fn damage_pair_only_for_different_robots() {
        // 벽/공(None) 포함 쌍 → 무데미지 (wall-no-damage)
        assert_eq!(damage_pair(None, Some((0, 0))), None, "벽↔로봇 무데미지");
        assert_eq!(damage_pair(Some((1, 0)), None), None);
        assert_eq!(damage_pair(None, None), None);
        // 같은 로봇의 다른 부위 → 무데미지 (self-part-no-damage)
        assert_eq!(
            damage_pair(Some((0, 0)), Some((0, 1))),
            None,
            "자기 부위끼리 무데미지"
        );
        // 다른 로봇 → 데미지(오름차순 정규화 쌍)
        assert_eq!(damage_pair(Some((1, 2)), Some((0, 0))), Some(((0, 0), (1, 2))));
        assert_eq!(damage_pair(Some((0, 1)), Some((1, 0))), Some(((0, 1), (1, 0))));
    }

    #[test]
    fn robots_colliding_take_mutual_damage() {
        use crate::parts::{aggregate, catalog};
        let cat = catalog();
        let mut w = PhysicsWorld::new_kickoff_with(
            [aggregate(&cat, "striker"), aggregate(&cat, "guard")],
            ["striker".to_string(), "guard".to_string()],
        );
        // 공을 치워 로봇끼리만 충돌
        w.set_ball_for_test(vector![0.0, 3.0], vector![0.0, 0.0]);
        // 두 로봇을 마주보게 근접 배치
        w.set_robot_for_test(0, vector![-0.4, 0.0], 0.0);
        w.set_robot_for_test(1, vector![0.4, 0.0], std::f32::consts::PI);
        let before = (w.hp_ratio_min(0), w.hp_ratio_min(1));
        // 서로를 향해 돌진(robot0 +x, robot1 -x)
        let toward = [ControlOutput {
            thrust: 1.0,
            turn: 0.0, run: false, kick: false,
        }; 2];
        for _ in 0..120 {
            w.step(&toward);
        }
        let after = (w.hp_ratio_min(0), w.hp_ratio_min(1));
        assert!(
            after.0 < before.0 && after.1 < before.1,
            "충돌 시 양쪽 부위 HP 감소 (before {before:?} after {after:?})"
        );
    }

    #[test]
    fn strong_collision_applies_knockback_and_stun() {
        use crate::parts::StatSet;
        // 넉백/스턴 성향이 강하고 방어가 낮은 로봇 둘을 정면 충돌시킨다.
        let brawler = StatSet {
            max_speed: 10.0,
            accel: 20.0,
            turn_rate: 1.0,
            mass: 1.0,
            attack: 2.0,
            defense: 3.0,
            hp: 2000.0,
            kb_w: 40.0,
            stun_w: 3.0,
            dmg_w: 1.0,
            ..Default::default()
        };
        let mut w = PhysicsWorld::new_kickoff_with(
            [brawler, brawler],
            [String::new(), String::new()],
        );
        // 공을 치워 로봇끼리만 충돌
        w.set_ball_for_test(vector![0.0, 3.0], vector![0.0, 0.0]);
        // 두 로봇을 마주보게 근접 배치
        w.set_robot_for_test(0, vector![-0.4, 0.0], 0.0);
        w.set_robot_for_test(1, vector![0.4, 0.0], std::f32::consts::PI);
        let toward = [ControlOutput {
            thrust: 1.0,
            turn: 0.0, run: false, kick: false,
        }; 2];
        let mut stunned_seen = false;
        let mut max_speed_seen: f32 = 0.0;
        for _ in 0..120 {
            w.step(&toward);
            if w.is_stunned_for_test(1) || w.is_stunned_for_test(0) {
                stunned_seen = true;
            }
            let v = w.snapshot().robots[1].vel;
            max_speed_seen = max_speed_seen.max((v.x * v.x + v.y * v.y).sqrt());
        }
        assert!(stunned_seen, "강한 충돌은 스턴을 유발해야 함");
        // 넉백으로 속도가 튐(입력만으로는 max_speed=10을 넘지 못하므로 간접 확인)
        assert!(
            max_speed_seen > 10.5,
            "넉백 임펄스로 max_speed를 초과하는 속도가 관측되어야 함 (got {max_speed_seen})"
        );
    }

    #[test]
    fn effect_profile_damage_is_applied_additively_on_collision() {
        use crate::parts::StatSet;
        // attack=0 → damage_on_contact(3b 모델)은 항상 0. dmg_w>0만으로도
        // 충돌 시 HP가 깎여야 한다(eff.damage 가산 배선 검증, KB-34 Fix1).
        let dmg_only = StatSet {
            max_speed: 10.0,
            accel: 20.0,
            turn_rate: 1.0,
            mass: 1.0,
            attack: 0.0,
            defense: 3.0,
            hp: 2000.0,
            dmg_w: 5.0,
            ..Default::default()
        };
        let mut w = PhysicsWorld::new_kickoff_with([dmg_only, dmg_only], [String::new(), String::new()]);
        // 공을 치워 로봇끼리만 충돌
        w.set_ball_for_test(vector![0.0, 3.0], vector![0.0, 0.0]);
        // 두 로봇을 마주보게 근접 배치
        w.set_robot_for_test(0, vector![-0.4, 0.0], 0.0);
        w.set_robot_for_test(1, vector![0.4, 0.0], std::f32::consts::PI);
        let before = (w.hp_ratio_min(0), w.hp_ratio_min(1));
        let toward = [ControlOutput {
            thrust: 1.0,
            turn: 0.0, run: false, kick: false,
        }; 2];
        for _ in 0..120 {
            w.step(&toward);
        }
        let after = (w.hp_ratio_min(0), w.hp_ratio_min(1));
        assert!(
            after.0 < before.0 && after.1 < before.1,
            "attack=0이어도 dmg_w 효과데미지가 가산 적용돼야 함 (before {before:?} after {after:?})"
        );
    }

    #[test]
    fn real_preset_collision_triggers_live_knockback_or_stun() {
        // KB-34 Fix3: 손으로 짠 StatSet이 아니라 실제 카탈로그 프리셋(striker/guard)만으로
        // 넉백/스턴이 실전 매치에서 실제로 발동하는지 증명한다(test-force 훅 미사용).
        use crate::parts::{aggregate, catalog};
        let cat = catalog();
        let mut w = PhysicsWorld::new_kickoff_with(
            [aggregate(&cat, "striker"), aggregate(&cat, "guard")],
            ["striker".to_string(), "guard".to_string()],
        );
        // 공을 치워 로봇끼리만 충돌
        w.set_ball_for_test(vector![0.0, 3.0], vector![0.0, 0.0]);
        // 두 로봇을 마주보게 근접 배치하고 정면으로 세게 부딪히게 한다.
        w.set_robot_for_test(0, vector![-0.4, 0.0], 0.0);
        w.set_robot_for_test(1, vector![0.4, 0.0], std::f32::consts::PI);
        let toward = [ControlOutput {
            thrust: 1.0,
            turn: 0.0, run: false, kick: false,
        }; 2];
        let mut stunned_seen = false;
        let mut max_speed_seen: f32 = 0.0;
        let max_speed_cap = aggregate(&cat, "striker")
            .max_speed
            .max(aggregate(&cat, "guard").max_speed);
        for _ in 0..180 {
            w.step(&toward);
            if w.is_stunned_for_test(0) || w.is_stunned_for_test(1) {
                stunned_seen = true;
            }
            let v = w.snapshot().robots[1].vel;
            max_speed_seen = max_speed_seen.max((v.x * v.x + v.y * v.y).sqrt());
        }
        assert!(
            stunned_seen || max_speed_seen > max_speed_cap + 0.5,
            "실제 프리셋(striker/guard) 정면 충돌에서 넉백 또는 스턴이 발동해야 함 \
             (stunned_seen={stunned_seen}, max_speed_seen={max_speed_seen}, cap={max_speed_cap})"
        );
    }

    #[test]
    fn ball_contact_does_no_damage() {
        use crate::parts::{aggregate, catalog};
        let cat = catalog();
        let mut w = PhysicsWorld::new_kickoff_with(
            [aggregate(&cat, "striker"), aggregate(&cat, "striker")],
            [String::new(), String::new()],
        );
        // robot0을 공(원점) 왼쪽에 두고 돌진, robot1은 멀리(로봇충돌 배제)
        w.set_robot_for_test(0, vector![-0.6, 0.0], 0.0);
        w.set_robot_for_test(1, vector![5.0, 3.0], 0.0);
        for _ in 0..300 {
            w.step(&[
                ControlOutput {
                    thrust: 1.0,
                    turn: 0.0, run: false, kick: false,
                },
                ControlOutput::default(),
            ]);
        }
        assert!(w.hp_ratio_min(0) > 0.99, "공 접촉은 무데미지");
        assert!(w.hp_ratio_min(1) > 0.99);
    }

    #[test]
    fn wall_contact_does_no_damage() {
        use crate::parts::{aggregate, catalog};
        let cat = catalog();
        let mut w = PhysicsWorld::new_kickoff_with(
            [aggregate(&cat, "striker"), aggregate(&cat, "striker")],
            [String::new(), String::new()],
        );
        w.set_ball_for_test(vector![5.0, -3.0], vector![0.0, 0.0]);
        // robot0을 상단 벽으로 돌진(rot=+PI/2), robot1은 멀리
        w.set_robot_for_test(0, vector![-3.0, 3.0], std::f32::consts::FRAC_PI_2);
        w.set_robot_for_test(1, vector![3.0, -3.0], 0.0);
        for _ in 0..200 {
            w.step(&[
                ControlOutput {
                    thrust: 1.0,
                    turn: 0.0, run: false, kick: false,
                },
                ControlOutput::default(),
            ]);
        }
        assert!(w.hp_ratio_min(0) > 0.99, "벽 접촉은 무데미지");
    }

    #[test]
    fn downed_robot_ignores_input_and_snapshot_shows_state() {
        let mut w = PhysicsWorld::new_kickoff();
        w.force_break_for_test(0);
        let s = w.snapshot();
        assert!(s.robots[0].down.broken, "스냅샷에 파손 다운 표시");
        assert!(s.robots[0].down.repair_in > 0.0, "리페어 잔여시간 노출");
        assert!(s.robots[0].st.iter().any(|x| x == "downed"));
        assert!(!s.robots[0].parts.is_empty(), "부위 HP 노출");
        assert!(s.robots[1].st.is_empty(), "정상 로봇은 상태이상 없음");
        // 와이어(JSON) 직렬화에도 디버프 필드가 실리는지(net.rs와 동일 경로)
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"broken\":true"));
        assert!(json.contains("\"downed\""));
        assert!(json.contains("\"parts\":"));
        // 다운 중 전진 입력 줘도 크게 안 움직임(입력 무시)
        let p0 = w.snapshot().robots[0].pos.x;
        for _ in 0..30 {
            w.step(&[
                ControlOutput {
                    thrust: 1.0,
                    turn: 0.0, run: false, kick: false,
                },
                ControlOutput::default(),
            ]);
        }
        assert!((w.snapshot().robots[0].pos.x - p0).abs() < 0.5);
        // 타이머 소진까지 진행 → 리페어(broken=false, 전체 부위 HP 복구)
        for _ in 0..w.combat[0].repair_ticks() {
            w.step(&[ControlOutput::default(); 2]);
        }
        let s2 = w.snapshot();
        assert!(!s2.robots[0].down.broken, "리페어 후 다운 해제");
        assert!(s2.robots[0].st.is_empty());
        assert!(w.hp_ratio_min(0) > 0.99, "리페어 시 전체 부위 HP 복구");
    }

    #[test]
    fn snapshot_st_shows_stun() {
        let mut w = PhysicsWorld::new_kickoff();
        w.force_stun_for_test(1, 0.5);
        let s = w.snapshot();
        assert!(s.robots[1].st.iter().any(|x| x == "stun"));
        assert!(s.robots[0].st.is_empty(), "스턴 안 된 로봇은 태그 없음");
        // 파손 다운과 스턴 동시 표기 가능
        w.force_break_for_test(1);
        let s2 = w.snapshot();
        assert!(s2.robots[1].st.iter().any(|x| x == "downed"));
        assert!(s2.robots[1].st.iter().any(|x| x == "stun"));
        // 와이어(JSON) 직렬화에도 실림
        let json = serde_json::to_string(&s2).unwrap();
        assert!(json.contains("\"stun\""));
    }

    #[test]
    fn kickoff_world_has_ball_and_two_robots_in_bounds() {
        let w = PhysicsWorld::new_kickoff();
        let s = w.snapshot();
        assert_eq!(s.robots.len(), 2);
        assert_eq!(s.ball.pos, Vec2 { x: 0.0, y: 0.0 });
        // 경계 안
        assert!(s.ball.pos.x.abs() <= FIELD_W / 2.0);
    }

    #[test]
    fn stepping_keeps_ball_in_bounds_and_advances_time() {
        let mut w = PhysicsWorld::new_kickoff();
        // 공에 강한 초기 속도
        w.kick_ball_for_test(vector![50.0, 30.0]);
        for _ in 0..600 {
            w.step(&[ControlOutput::default(); 2]);
        } // 10초
        let s = w.snapshot();
        assert!(s.time > 9.0);
        assert!(s.ball.pos.x.abs() <= FIELD_W / 2.0 + 0.5); // 벽 안(여유)
        assert!(s.ball.pos.y.abs() <= FIELD_H / 2.0 + 0.5);
    }

    #[test]
    fn ball_driven_into_right_goal_scores_blue() {
        let mut w = PhysicsWorld::new_kickoff();
        // KB-43: 골 입구 펜스 도입 전에는 robot1(킥오프 x=3.0, 공의 직선 경로상)이
        // 공에 맞아 넉백으로 필드 밖까지 날아가며 "우연히" 경로를 비켜줬다(바로 그 탈출
        // 버그). 펜스가 로봇을 담아내는 지금은 robot1이 골 입구에 멈춰 서서 슛을
        // 가로막으므로, 이 테스트 본연의 목적(공이 펜스를 통과해 득점)만 검증하도록
        // robot1을 공의 경로 밖으로 옮겨 격리한다(다른 테스트의 set_ball/robot_for_test
        // 격리 패턴과 동일).
        w.set_robot_for_test(1, vector![3.0, 3.0], std::f32::consts::PI);
        w.kick_ball_for_test(vector![40.0, 0.0]); // 오른쪽으로 강하게
        let mut scored = false;
        for _ in 0..300 {
            w.step(&[ControlOutput::default(); 2]);
            if w.score.0 == 1 {
                scored = true;
                break;
            }
        }
        assert!(scored, "공이 오른쪽 골로 들어가 Blue 득점해야 함");
        // 득점 후 공은 킥오프로 리셋
        assert!(w.snapshot().ball.pos.x.abs() < 0.1);
    }

    #[test]
    fn snapshot_carries_preset_id() {
        use crate::parts::{aggregate, catalog};
        let cat = catalog();
        let w = PhysicsWorld::new_kickoff_with(
            [aggregate(&cat, "striker"), aggregate(&cat, "guard")],
            ["striker".to_string(), "guard".to_string()],
        );
        let s = w.snapshot();
        assert_eq!(s.robots[0].robot, "striker");
        assert_eq!(s.robots[1].robot, "guard");
    }

    #[test]
    fn robot_speed_capped_by_max_speed() {
        use crate::parts::StatSet;
        let slow = StatSet {
            max_speed: 1.0,
            accel: 10.0,
            turn_rate: 1.0,
            mass: 1.0,
            ..Default::default()
        };
        let mut w =
            PhysicsWorld::new_kickoff_with([slow, slow], [String::new(), String::new()]);
        let fwd = [ControlOutput {
            thrust: 1.0,
            turn: 0.0, run: false, kick: false,
        }; 2];
        for _ in 0..120 {
            w.step(&fwd);
        }
        let v = w.snapshot().robots[0].vel;
        let sp = (v.x * v.x + v.y * v.y).sqrt();
        assert!(sp <= 1.05, "속도는 max_speed 근처로 제한되어야 함 (got {sp})");
    }

    #[test]
    fn sprint_exceeds_walk_speed_then_falls_back_when_stamina_depleted() {
        use crate::parts::StatSet;
        // stamina_regen=0 → 소진 후 회복 없이 결정적으로 walk 유지(오실레이션 배제).
        let runner = StatSet {
            max_speed: 5.0,
            accel: 50.0,
            turn_rate: 1.0,
            mass: 1.0,
            sprint_speed: 10.0,
            stamina_max: 0.5, // 0.5초분 = DT(1/60) 기준 30틱
            stamina_regen: 0.0,
            ..Default::default()
        };
        let mut w =
            PhysicsWorld::new_kickoff_with([runner, runner], [String::new(), String::new()]);
        let run_input = [ControlOutput {
            thrust: 1.0,
            turn: 0.0,
            run: true,
            kick: false,
        }; 2];
        let mut max_speed_seen: f32 = 0.0;
        for _ in 0..20 {
            // 스태미나 소진(30틱) 전 구간
            w.step(&run_input);
            let v = w.snapshot().robots[0].vel;
            max_speed_seen = max_speed_seen.max((v.x * v.x + v.y * v.y).sqrt());
        }
        assert!(
            max_speed_seen > 5.5,
            "스태미나가 있으면 sprint_speed까지 max_speed를 초과해야 함 (got {max_speed_seen})"
        );
        assert!(
            w.snapshot().robots[0].stamina < 1.0,
            "달리는 동안 스태미나가 소모되어야 함"
        );
        // 스태미나 소진(총 200틱 진행, 30틱 넘음) 후 계속 run:true를 눌러도 walk로 폴백.
        for _ in 0..200 {
            w.step(&run_input);
        }
        assert_eq!(
            w.snapshot().robots[0].stamina,
            0.0,
            "스태미나가 0으로 소진되어야 함(회복 없음 설정)"
        );
        let v = w.snapshot().robots[0].vel;
        let sp = (v.x * v.x + v.y * v.y).sqrt();
        assert!(
            sp <= 5.5,
            "스태미나 소진 후 run:true를 유지해도 walk(max_speed)로 자동 폴백해야 함 (got {sp})"
        );
    }

    #[test]
    fn stamina_recovers_only_when_idle_not_while_walking() {
        use crate::parts::StatSet;
        let runner = StatSet {
            max_speed: 5.0,
            accel: 50.0,
            turn_rate: 1.0,
            mass: 1.0,
            sprint_speed: 10.0,
            stamina_max: 1.0,
            stamina_regen: 1.0, // 초당 1.0 회복
            ..Default::default()
        };
        let mut w =
            PhysicsWorld::new_kickoff_with([runner, runner], [String::new(), String::new()]);
        // 충돌 배제: 서로 다른 레인(y)에 평행 배치, 같은 입력이라도 안 부딪힘.
        w.set_robot_for_test(0, vector![-4.0, -2.0], 0.0);
        w.set_robot_for_test(1, vector![-4.0, 2.0], 0.0);

        // 스프린트로 일부 소모.
        let run = [ControlOutput { thrust: 1.0, turn: 0.0, run: true, kick: false }; 2];
        for _ in 0..30 {
            w.step(&run);
        }
        let after_sprint = w.snapshot().robots[0].stamina;
        assert!(after_sprint < 1.0, "스프린트로 소모돼야 함 (got {after_sprint})");

        // 걷기(이동 입력 있음, run=false) → 회복하지 않고 유지.
        let walk = [ControlOutput { thrust: 1.0, turn: 0.0, run: false, kick: false }; 2];
        for _ in 0..60 {
            w.step(&walk);
        }
        let after_walk = w.snapshot().robots[0].stamina;
        assert!(
            (after_walk - after_sprint).abs() < 1e-3,
            "걷는 중엔 회복하지 않아야 함 (sprint {after_sprint} → walk {after_walk})"
        );

        // 멈춤(이동 입력 없음) → 회복.
        let idle = [ControlOutput { thrust: 0.0, turn: 0.0, run: false, kick: false }; 2];
        for _ in 0..60 {
            w.step(&idle);
        }
        let after_idle = w.snapshot().robots[0].stamina;
        assert!(
            after_idle > after_walk + 0.1,
            "가만히 있으면 회복해야 함 (walk {after_walk} → idle {after_idle})"
        );
    }

    #[test]
    fn higher_accel_robot_travels_farther() {
        use crate::parts::{aggregate, catalog};
        let cat = catalog();
        // robot0=guard(accel↑), robot1=striker → 이동 거리가 달라야 함
        let mut w = PhysicsWorld::new_kickoff_with(
            [aggregate(&cat, "guard"), aggregate(&cat, "striker")],
            ["guard".to_string(), "striker".to_string()],
        );
        let fwd = [
            ControlOutput {
                thrust: 1.0,
                turn: 0.0, run: false, kick: false,
            },
            ControlOutput {
                thrust: 1.0,
                turn: 0.0, run: false, kick: false,
            },
        ];
        let x0 = w
            .snapshot()
            .robots
            .iter()
            .map(|r| r.pos.x)
            .collect::<Vec<_>>();
        for _ in 0..60 {
            w.step(&fwd);
        }
        let s = w.snapshot();
        let d0 = (s.robots[0].pos.x - x0[0]).abs();
        let d1 = (s.robots[1].pos.x - x0[1]).abs();
        assert!(d0 != d1, "스탯이 다르면 이동 거리가 달라야 함");
    }

    #[test]
    fn robot_cannot_escape_through_goal_mouth() {
        // 골 입구(y∈[-GOAL_W/2, GOAL_W/2]) 정중앙에 로봇을 놓고 오른쪽으로 강하게 전진.
        // 공은 이 틈을 빠져나가야 득점이 성립하지만, 로봇은 펜스에 막혀야 한다.
        let mut w = PhysicsWorld::new_kickoff();
        w.set_ball_for_test(vector![0.0, 3.0], vector![0.0, 0.0]); // 공은 치워둠(간섭 배제)
        w.set_robot_for_test(0, vector![FIELD_W / 2.0 - 1.0, 0.0], 0.0);
        let toward = [
            ControlOutput {
                thrust: 1.0,
                turn: 0.0, run: false, kick: false,
            },
            ControlOutput::default(),
        ];
        let mut max_x: f32 = 0.0;
        for _ in 0..120 {
            w.step(&toward);
            max_x = max_x.max(w.snapshot().robots[0].pos.x);
        }
        assert!(
            max_x <= FIELD_W / 2.0 + 0.5,
            "로봇은 골 입구 펜스에 막혀 필드 밖으로 나가면 안 됨 (got x={max_x})"
        );
    }

    #[test]
    fn ball_escapes_corner() {
        // KB-44: 필드 모서리(90도 직각)에 공이 끼어 로봇이 빼낼 수 없는 교착을 막기 위해
        // 각 모서리에 45도 챔퍼(대각 벽)를 추가한다.
        //
        // 판별 기준: 챔퍼가 없으면(기존 직각 벽 두 장만 있으면) 공이 안정적으로 정지할
        // 수 있는 "가장 깊은 코너 안착점"은 두 벽 두께(WALL_T)+공 반지름(BALL_R)만큼
        // 안쪽인 (hw-WALL_T-BALL_R, hh-WALL_T-BALL_R) 부근이며, apex(hw,hh)로부터의
        // 거리는 약 0.57에 불과하다(코너에 사실상 박힘). 챔퍼를 추가하면 그 안착점이
        // 통째로 사라지고 대각면에 훨씬 못 미쳐 멈추므로 apex로부터의 거리가 뚜렷하게
        // (>1.0) 커진다. 이 임계값(1.0)은 "박힘(≈0.57)"과 "챔퍼로 밀려남(≈1.8, 계산상)"
        // 사이를 확실히 가르도록 선택했다(단순 반사/디플렉션이 아니라 최종 안착 위치로 검증).
        let hw = FIELD_W / 2.0;
        let hh = FIELD_H / 2.0;
        let mut w = PhysicsWorld::new_kickoff();
        // 로봇들을 코너와 무관한 곳으로 치워 간섭 배제
        w.set_robot_for_test(0, vector![0.0, -3.5], 0.0);
        w.set_robot_for_test(1, vector![0.0, 3.5], std::f32::consts::PI);
        // 공을 우상단 코너를 향해 굴려보내 감쇠(linear_damping)로 자연히 정착하게 한다.
        let start = vector![hw - 1.0, hh - 1.0];
        w.set_ball_for_test(start, vector![0.3, 0.3]);
        // apex(꼭짓점)까지의 유클리드 거리
        let dist_to_apex = |x: f32, y: f32| ((x - hw).powi(2) + (y - hh).powi(2)).sqrt();
        for _ in 0..300 {
            w.step(&[ControlOutput::default(); 2]);
        }
        let end = w.snapshot().ball.pos;
        let end_dist = dist_to_apex(end.x, end.y);
        // 공이 챔퍼/벽을 뚫고 필드 밖으로 나가면 안 됨(여유 포함 경계 안).
        assert!(
            end.x.abs() <= hw + 0.5 && end.y.abs() <= hh + 0.5,
            "공이 챔퍼/벽을 뚫고 필드 밖으로 나가면 안 됨 (got {end:?})"
        );
        // 챔퍼가 코너 안착점을 없애 apex로부터 확실히 멀어진 곳에서 멈춰야 함.
        assert!(
            end_dist > 1.0,
            "챔퍼가 있으면 코너 apex 근접 안착이 불가능해야 함 \
             (end={end:?}, end_dist={end_dist}, 임계값 1.0 — 챔퍼 없는 안착점은 ≈0.57)"
        );
    }

    #[test]
    fn ball_exiting_outside_goal_mouth_does_not_score() {
        let mut w = PhysicsWorld::new_kickoff();
        // 골 입구 밖(|y| > GOAL_W/2)에서 오른쪽 벽 쪽으로 밀어냄
        let y = GOAL_W / 2.0 + 1.0;
        w.set_ball_for_test(vector![FIELD_W / 2.0 - 1.0, y], vector![40.0, 0.0]);
        for _ in 0..300 {
            w.step(&[ControlOutput::default(); 2]);
        }
        // 골 입구 밖이라 벽에 막혀 무득점
        assert_eq!(w.score, (0, 0), "골 입구 밖으로 나가면 무득점이어야 함");
    }

    /// 킥 테스트 공용 스탯: 이동은 거의 영향 없게, kick_power만 지정.
    fn kicker_stats(kick_power: f32) -> StatSet {
        StatSet {
            max_speed: 5.0,
            accel: 5.0,
            turn_rate: 3.0,
            mass: 1.0,
            kick_power,
            ..Default::default()
        }
    }

    /// 로봇0을 원점(정면=+x)에 놓고, 공을 정면 사거리 안(0.5m)에 정지 배치한 킥 테스트용 월드.
    fn kick_test_world(kick_power: f32) -> PhysicsWorld {
        let mut w = PhysicsWorld::new_kickoff_with(
            [kicker_stats(kick_power), StatSet::default()],
            [String::new(), String::new()],
        );
        w.set_robot_for_test(0, vector![0.0, 0.0], 0.0);
        w.set_ball_for_test(vector![0.5, 0.0], vector![0.0, 0.0]);
        w
    }

    fn kick_input(thrust: f32, turn: f32) -> [ControlOutput; 2] {
        [
            ControlOutput {
                thrust,
                turn,
                run: false,
                kick: true,
            },
            ControlOutput::default(),
        ]
    }

    fn speed(v: Vec2) -> f32 {
        (v.x * v.x + v.y * v.y).sqrt()
    }

    #[test]
    fn kick_launches_ball_when_in_front_range() {
        let mut w = kick_test_world(2.0);
        w.step(&kick_input(0.0, 0.0));
        let v = w.snapshot().ball.vel;
        assert!(speed(v) > 0.5, "정면 사거리 내 킥은 공을 발사해야 함 (got {v:?})");
        assert!(v.x > 0.0, "정면(+x) 성분이 양수여야 함 (got {v:?})");
    }

    #[test]
    fn kick_ignored_when_ball_out_of_range() {
        let mut w = kick_test_world(2.0);
        // 사거리(0.9m) 밖으로 공을 멀리 재배치.
        w.set_ball_for_test(vector![5.0, 0.0], vector![0.0, 0.0]);
        w.step(&kick_input(0.0, 0.0));
        let v = w.snapshot().ball.vel;
        assert!(speed(v) < 1e-6, "사거리 밖이면 공 속도가 불변해야 함 (got {v:?})");
    }

    #[test]
    fn kick_power_scales_with_thrust_level() {
        let mut w_strong = kick_test_world(2.0);
        w_strong.step(&kick_input(1.0, 0.0)); // thrust>0(↑) = 강(1.0)
        let v_strong = w_strong.snapshot().ball.vel;

        let mut w_weak = kick_test_world(2.0);
        w_weak.step(&kick_input(-1.0, 0.0)); // thrust<0(↓) = 약(0.5)
        let v_weak = w_weak.snapshot().ball.vel;

        assert!(
            speed(v_strong) > speed(v_weak),
            "thrust>0(강)은 thrust<0(약)보다 세야 함 (strong {} weak {})",
            speed(v_strong),
            speed(v_weak)
        );
    }

    #[test]
    fn kick_direction_offset_by_turn() {
        let mut w = kick_test_world(2.0);
        w.step(&kick_input(0.0, 1.0)); // turn>0(←) = 정면 기준 좌(+AIM)
        let v = w.snapshot().ball.vel;
        assert!(v.y > 0.0, "turn>0(←)은 정면 기준 좌(+y)로 치우쳐야 함 (got {v:?})");
    }

    #[test]
    fn kick_edge_triggered_once() {
        let mut w = kick_test_world(2.0);
        let hold = kick_input(0.0, 0.0);
        w.step(&hold);
        let v1 = w.snapshot().ball.vel;
        assert!(speed(v1) > 0.0, "1스텝째는 발사되어야 함 (got {v1:?})");
        // 공을 원위치·무속도로 되돌리되(사거리/정면 조건은 유지), kick:true는 계속 홀드.
        w.set_ball_for_test(vector![0.5, 0.0], vector![0.0, 0.0]);
        w.step(&hold);
        let v2 = w.snapshot().ball.vel;
        assert!(
            speed(v2) < 1e-6,
            "kick을 홀드해도 2번째 스텝엔 추가 발사(가속)가 없어야 함 (got {v2:?})"
        );
    }

    #[test]
    fn kick_blocked_when_stunned() {
        let mut w = kick_test_world(2.0);
        w.force_stun_for_test(0, 1.0);
        w.step(&kick_input(0.0, 0.0));
        let v = w.snapshot().ball.vel;
        assert!(speed(v) < 1e-6, "스턴 중엔 킥이 무동작이어야 함 (got {v:?})");
    }

    #[test]
    fn kick_blocked_when_broken() {
        let mut w = kick_test_world(2.0);
        w.force_break_for_test(0); // 파손 다운
        w.step(&kick_input(0.0, 0.0));
        let v = w.snapshot().ball.vel;
        assert!(speed(v) < 1e-6, "파손 다운 중엔 킥이 무동작이어야 함 (got {v:?})");
    }

    #[test]
    fn kick_ignored_when_ball_behind_cone() {
        let mut w = kick_test_world(2.0);
        // 사거리 안(0.5m)이지만 로봇 뒤쪽(정면 +x의 반대) → 정면 콘 밖이라 무동작.
        w.set_ball_for_test(vector![-0.5, 0.0], vector![0.0, 0.0]);
        w.step(&kick_input(0.0, 0.0));
        let v = w.snapshot().ball.vel;
        assert!(speed(v) < 1e-6, "정면 콘 밖(뒤쪽)이면 킥 무동작이어야 함 (got {v:?})");
    }

    #[test]
    fn kick_cooldown_blocks_immediate_repress() {
        let mut w = kick_test_world(2.0);
        // 1) 발사(상승엣지).
        w.step(&kick_input(0.0, 0.0));
        assert!(speed(w.snapshot().ball.vel) > 0.0, "1회차는 발사되어야 함");
        // 2) 릴리스(kick:false)로 엣지 리셋 — 쿨다운은 아직 잔여(0.45s ≫ 1스텝).
        w.set_ball_for_test(vector![0.5, 0.0], vector![0.0, 0.0]);
        w.step(&[ControlOutput::default(), ControlOutput::default()]);
        // 3) 즉시 재-press(새 상승엣지)지만 쿨다운 중이라 막혀야 함.
        w.set_ball_for_test(vector![0.5, 0.0], vector![0.0, 0.0]);
        w.step(&kick_input(0.0, 0.0));
        let v = w.snapshot().ball.vel;
        assert!(
            speed(v) < 1e-6,
            "쿨다운 중 재-press는 막혀야 함(shoot_lock) (got {v:?})"
        );
    }

    #[test]
    fn shoot_lock_appears_in_snapshot_after_kick() {
        let mut w = kick_test_world(2.0);
        w.step(&kick_input(0.0, 0.0));
        let st = &w.snapshot().robots[0].st;
        assert!(
            st.iter().any(|s| s == "shoot_lock"),
            "발사 직후 쿨다운 동안 st에 shoot_lock이 노출되어야 함 (got {st:?})"
        );
    }

    // -- 4대 매치(팀당 공격형+수비형) ------------------------------------------

    #[test]
    fn new_match_has_four_robots_with_fixed_roster_and_presets() {
        let w = PhysicsWorld::new_match();
        let s = w.snapshot();
        assert_eq!(s.robots.len(), 4, "팀당 2대(공격형+수비형), 총 4대여야 함");
        assert_eq!(s.robots[0].id, Team::Blue);
        assert_eq!(s.robots[1].id, Team::Blue);
        assert_eq!(s.robots[2].id, Team::Red);
        assert_eq!(s.robots[3].id, Team::Red);
        assert_eq!(s.robots[0].robot, "striker", "0=Blue striker");
        assert_eq!(s.robots[1].robot, "guard", "1=Blue guard");
        assert_eq!(s.robots[2].robot, "striker", "2=Red striker");
        assert_eq!(s.robots[3].robot, "guard", "3=Red guard");
    }

    #[test]
    fn new_match_kickoff_places_blue_left_red_right_in_bounds() {
        let w = PhysicsWorld::new_match();
        let s = w.snapshot();
        assert!(s.robots[0].pos.x < 0.0, "Blue striker는 왼쪽(x<0)");
        assert!(s.robots[1].pos.x < 0.0, "Blue guard는 왼쪽(x<0)");
        assert!(s.robots[2].pos.x > 0.0, "Red striker는 오른쪽(x>0)");
        assert!(s.robots[3].pos.x > 0.0, "Red guard는 오른쪽(x>0)");
        for r in &s.robots {
            assert!(r.pos.x.abs() <= FIELD_W / 2.0, "필드 안에서 킥오프해야 함");
            assert!(r.pos.y.abs() <= FIELD_H / 2.0, "필드 안에서 킥오프해야 함");
        }
    }

    #[test]
    fn friendly_fire_is_disabled_but_cross_team_contact_still_damages() {
        // 같은 팀(0,1=Blue)을 근접 정면 배치하고 서로에게 돌진 → 무데미지여야 함.
        // 나머지 로봇(2,3)은 필드 안 먼 곳으로 치워 간섭을 배제한다.
        let mut w = PhysicsWorld::new_match();
        w.set_ball_for_test(vector![0.0, 3.5], vector![0.0, 0.0]);
        w.set_robot_for_test(0, vector![-0.4, 0.0], 0.0);
        w.set_robot_for_test(1, vector![0.4, 0.0], std::f32::consts::PI);
        w.set_robot_for_test(2, vector![5.0, 3.5], 0.0);
        w.set_robot_for_test(3, vector![5.0, -3.5], 0.0);
        let before_friendly = (w.hp_ratio_min(0), w.hp_ratio_min(1));
        let toward = [ControlOutput {
            thrust: 1.0,
            turn: 0.0,
            run: false,
            kick: false,
        }; 4];
        for _ in 0..120 {
            w.step(&toward);
        }
        let after_friendly = (w.hp_ratio_min(0), w.hp_ratio_min(1));
        assert_eq!(
            after_friendly, before_friendly,
            "같은 팀 충돌은 무데미지여야 함 (before {before_friendly:?} after {after_friendly:?})"
        );

        // 다른 팀(0=Blue, 2=Red)을 근접 정면 배치하고 돌진 → 데미지가 나야 함.
        let mut w2 = PhysicsWorld::new_match();
        w2.set_ball_for_test(vector![0.0, 3.5], vector![0.0, 0.0]);
        w2.set_robot_for_test(0, vector![-0.4, 0.0], 0.0);
        w2.set_robot_for_test(2, vector![0.4, 0.0], std::f32::consts::PI);
        w2.set_robot_for_test(1, vector![5.0, 3.5], 0.0);
        w2.set_robot_for_test(3, vector![5.0, -3.5], 0.0);
        let before_enemy = (w2.hp_ratio_min(0), w2.hp_ratio_min(2));
        for _ in 0..120 {
            w2.step(&toward);
        }
        let after_enemy = (w2.hp_ratio_min(0), w2.hp_ratio_min(2));
        assert!(
            after_enemy.0 < before_enemy.0 && after_enemy.1 < before_enemy.1,
            "다른 팀 충돌은 데미지가 나야 함 (before {before_enemy:?} after {after_enemy:?})"
        );
    }

    #[test]
    fn new_match_reset_kickoff_uses_four_robot_layout() {
        // 득점 후 리셋이 4대 레이아웃(world::MATCH_KICKOFF)을 그대로 복원하는지 확인.
        let mut w = PhysicsWorld::new_match();
        // robot1(Blue guard)을 골 입구 경로 밖으로 옮겨 슛을 가로막지 않게 격리
        // (KB-43 ball_driven_into_right_goal_scores_blue와 동일한 격리 패턴).
        for i in 1..4 {
            w.set_robot_for_test(i, vector![0.0, FIELD_H / 2.0 - 0.5], 0.0);
        }
        w.kick_ball_for_test(vector![40.0, 0.0]);
        let mut scored = false;
        for _ in 0..300 {
            w.step(&[ControlOutput::default(); 4]);
            if w.score.0 == 1 {
                scored = true;
                break;
            }
        }
        assert!(scored, "득점이 발생해야 함");
        let s = w.snapshot();
        assert_eq!(s.robots.len(), 4, "리셋 후에도 4대 유지");
        assert!(s.robots[0].pos.x < 0.0, "리셋 후 Blue striker는 다시 왼쪽");
        assert!(s.robots[2].pos.x > 0.0, "리셋 후 Red striker는 다시 오른쪽");
    }
}
