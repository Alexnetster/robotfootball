use crate::control::Controller;
use crate::physics::PhysicsWorld;
use crate::world::*;

/// 한 tick: 각 컨트롤러 decide → physics step. (결정적)
pub fn tick(world: &mut PhysicsWorld, controllers: &mut [Box<dyn Controller>]) {
    let snap = world.snapshot();
    debug_assert_eq!(
        controllers.len(),
        snap.robots.len(),
        "컨트롤러 수와 로봇 수가 일치해야 함 (controls[i] ↔ robots[i])"
    );
    let outs: Vec<ControlOutput> = controllers
        .iter_mut()
        .enumerate()
        .map(|(i, c)| {
            let view = GameView {
                me: &snap.robots[i],
                ball: &snap.ball,
            };
            c.decide(&view)
        })
        .collect();
    world.step(&outs);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::ChaseBallAi;

    #[test]
    fn tick_drives_physics_and_ball_moves_when_robot_pushes() {
        let mut w = PhysicsWorld::new_kickoff();
        let mut ctrls: Vec<Box<dyn Controller>> =
            vec![Box::new(ChaseBallAi::default()), Box::new(ChaseBallAi::default())];
        // 두 AI가 좌우 대칭이라 공을 중앙에서 맞미는 평형으로 수렴한다.
        // "밀면 움직인다"는 의도상, 스냅샷 최종값이 아니라 구동 중 임의 시점에
        // 공이 실제로 움직였는지(변위/속도)를 판정한다. (결정적)
        let mut ball_moved = false;
        for _ in 0..300 {
            tick(&mut w, &mut ctrls);
            let s = w.snapshot();
            if s.ball.pos.x.abs() > 0.05
                || s.ball.pos.y.abs() > 0.05
                || s.ball.vel.x.abs() > 0.05
                || s.ball.vel.y.abs() > 0.05
            {
                ball_moved = true;
                break;
            }
        } // 5초
        assert!(ball_moved, "AI가 공을 밀면 공이 움직여야 함");
    }

    /// 재현 테스트(KB-58 조사): 실제 4대 로스터 + 협동 AI를 30초 돌려 공/로봇 좌표가
    /// NaN·무한대가 되거나 필드를 벗어나지 않는지 확인. "공이 안 보이고 로봇이 따로
    /// 논다"는 증상이 공 좌표 오염(NaN/탈출)에서 오는지 데이터로 판별.
    #[test]
    fn match_ai_keeps_ball_and_robots_on_field() {
        use crate::control::DefenderAi;
        let mut w = PhysicsWorld::new_match();
        let mut ctrls: Vec<Box<dyn Controller>> = vec![
            Box::new(ChaseBallAi::default()),
            Box::new(DefenderAi::default()),
            Box::new(ChaseBallAi::default()),
            Box::new(DefenderAi::default()),
        ];
        for i in 0..1800 {
            tick(&mut w, &mut ctrls);
            let s = w.snapshot();
            assert!(
                s.ball.pos.x.is_finite() && s.ball.pos.y.is_finite(),
                "tick {i}: 공 좌표 NaN/inf = {:?}",
                s.ball.pos
            );
            assert!(
                s.ball.pos.x.abs() <= FIELD_W && s.ball.pos.y.abs() <= FIELD_H,
                "tick {i}: 공이 필드를 벗어남 = {:?}",
                s.ball.pos
            );
            for (ri, r) in s.robots.iter().enumerate() {
                assert!(
                    r.pos.x.is_finite() && r.pos.y.is_finite(),
                    "tick {i}: 로봇 {ri} 좌표 NaN = {:?}",
                    r.pos
                );
            }
        }
    }
}
