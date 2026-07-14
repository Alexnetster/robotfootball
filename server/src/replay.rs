//! 결정성 회귀(골든 리플레이) 하니스. 테스트 전용(`#[cfg(test)] mod replay`)이라
//! 릴리스 바이너리엔 포함되지 않으며, `run_headless`/`hash_state`는 테스트에서 소비된다.

use crate::control::{ChaseBallAi, Controller};
use crate::parts::{default_stats, StatSet};
use crate::physics::PhysicsWorld;
use crate::world::GameState;

/// 결정적 상태 해시(부동소수를 비트로). 로봇 pos/rot + 부위 HP·파손다운 + 공 pos + 스코어.
/// 전투(부위 HP/다운)도 결정적 회귀 대상 → 해시에 포함(same-build 동일입력 동일해시).
pub fn hash_state(s: &GameState) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for r in &s.robots {
        r.pos.x.to_bits().hash(&mut h);
        r.pos.y.to_bits().hash(&mut h);
        r.rot.to_bits().hash(&mut h);
        for (name, ratio) in &r.parts {
            name.hash(&mut h);
            ratio.to_bits().hash(&mut h);
        }
        r.down.broken.hash(&mut h);
        r.down.repair_in.to_bits().hash(&mut h);
    }
    s.ball.pos.x.to_bits().hash(&mut h);
    s.ball.pos.y.to_bits().hash(&mut h);
    s.score.hash(&mut h);
    h.finish()
}

/// 로봇별 스탯을 받아 2 ChaseBallAi로 N 스텝 돌린 뒤 최종 스냅샷 해시. 결정적.
pub fn run_headless_with(stats: [StatSet; 2], steps: u32) -> u64 {
    let mut w = PhysicsWorld::new_kickoff_with(stats, [String::new(), String::new()]);
    let mut c: Vec<Box<dyn Controller>> = vec![Box::new(ChaseBallAi::default()), Box::new(ChaseBallAi::default())];
    for _ in 0..steps {
        crate::loop_runner::tick(&mut w, &mut c);
    }
    hash_state(&w.snapshot())
}

/// 기본 스탯으로 위임. 결정적.
pub fn run_headless(steps: u32) -> u64 {
    run_headless_with([default_stats(), default_stats()], steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_inputs_same_hash_same_build() {
        assert_eq!(run_headless(600), run_headless(600));
    }

    #[test]
    fn combat_state_is_deterministic_and_hashed() {
        use crate::parts::{aggregate, catalog};
        let cat = catalog();
        // 두 로봇이 공을 쫓다 중앙에서 충돌 → 부위 HP/다운이 해시에 반영되어도 결정적.
        let run = || {
            run_headless_with([aggregate(&cat, "striker"), aggregate(&cat, "guard")], 600)
        };
        assert_eq!(run(), run(), "전투 포함 상태도 same-build 동일입력 동일해시");
    }

    #[test]
    fn asymmetric_presets_diverge_from_symmetric() {
        use crate::parts::{aggregate, catalog};
        let cat = catalog();
        let asym =
            run_headless_with([aggregate(&cat, "striker"), aggregate(&cat, "guard")], 300);
        let sym =
            run_headless_with([aggregate(&cat, "striker"), aggregate(&cat, "striker")], 300);
        assert_ne!(asym, sym, "비대칭 로드아웃은 대칭과 다른 경기 전개");
    }
}
