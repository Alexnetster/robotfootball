//! 스태미나(달리기 자원) 순수 로직 (결정적, I/O 없음). KB-45 최소 슬라이스:
//! 걷기(기본)/달리기(Shift)만 — 오버히트·감속 페널티·쉬기 정책은 범위 밖(문서 §8,
//! 구현은 그중 "걷기/달리기" 부분만).
//!
//! 단위: `current`/`max`는 "달리기 가능 초"와 같은 축(즉 drain은 dt를 그대로 차감).
//! `regen`은 초당 회복량(스탯 `stamina_regen`). 적용 정책(KB-53)은 `apply_controls`에
//! 있다: 스프린트=소모, 걷기(이동 입력)=유지, **가만히(이동 입력 없음)=회복**.

/// 로봇 1대의 스태미나 상태. `current` ∈ [0, max].
#[derive(Clone, Copy, Debug)]
pub struct StaminaState {
    current: f32,
    max: f32,
    regen: f32,
}

impl StaminaState {
    pub fn new(max: f32, regen: f32) -> Self {
        let max = max.max(0.0);
        Self {
            current: max,
            max,
            regen: regen.max(0.0),
        }
    }

    /// 0..1 비율(스냅샷용). max=0이면 항상 1.0(용량 없는 로봇은 소모 개념 자체가 없음).
    pub fn ratio(&self) -> f32 {
        if self.max > 0.0 {
            self.current / self.max
        } else {
            1.0
        }
    }

    /// 스태미나가 남아있는지(달리기 시작 가능 판정용).
    pub fn has_stamina(&self) -> bool {
        self.current > 0.0
    }

    /// 달리는 동안 매 tick 호출: dt만큼 소모(0 하한).
    pub fn drain(&mut self, dt: f32) {
        self.current = (self.current - dt.max(0.0)).max(0.0);
    }

    /// 회복 조건(가만히 있을 때) 충족 시 매 tick 호출: regen×dt만큼 회복(max 상한).
    pub fn regen(&mut self, dt: f32) {
        self.current = (self.current + self.regen * dt.max(0.0)).min(self.max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sprinting_depletes_to_zero_then_stops() {
        let mut s = StaminaState::new(1.0, 0.5);
        assert!((s.ratio() - 1.0).abs() < 1e-6, "초기값은 full");
        // dt=0.1을 20번(=2.0초분) 소모 시도 → max(1.0초분)만 소모되고 0에서 멈춤.
        for _ in 0..20 {
            s.drain(0.1);
        }
        assert_eq!(s.ratio(), 0.0, "소모는 0에서 멈추고 음수로 내려가지 않음");
        assert!(!s.has_stamina());
    }

    #[test]
    fn not_sprinting_refills_to_max() {
        let mut s = StaminaState::new(2.0, 1.0); // 2초분, 초당 1.0 회복(=2초에 완전 회복)
        s.drain(2.0); // 완전 소모
        assert_eq!(s.ratio(), 0.0);
        for _ in 0..300 {
            s.regen(1.0 / 60.0); // 5초분 진행 → 2초 회복 시간보다 충분히 김
        }
        assert!((s.ratio() - 1.0).abs() < 1e-6, "회복은 max에서 멈춤(초과 없음)");
    }
}
