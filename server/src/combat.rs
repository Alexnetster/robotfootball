//! 전투/데미지 순수 로직 (결정적, I/O 없음). 충돌 감지는 physics.rs의 경계.
//! attack/defense는 **로봇 총합**(3b) — 부위별 세분화·취약도 항은 Plan 3c+ 여지.
//! effect 프로필(kb_w/stun_w/dmg_w)은 3c에서 배선 완료: 넉백/스턴은 임계 이상일 때,
//! dmg_w 유래 데미지는 damage_on_contact(3b) 결과에 **가산**으로 physics.rs에서 적용된다(KB-34).

use crate::parts::StatSet;

/// 상호 데미지 한쪽 산출: impact × (공격 / (방어+1)) × 계수. 결정적·비음수.
///
/// `impact`는 ADR-009의 접촉 임펄스를 **상대 linvel 크기**로 간소화한 근사(3b 의도적 간소화).
/// 진짜 임펄스는 `ContactForceEvent`(CONTACT_FORCE_EVENTS + threshold)로 3c/튜닝에서.
/// `defense + 1`로 0방어 폭주를 방지한다.
pub fn damage_on_contact(attacker: &StatSet, defender: &StatSet, impact: f32) -> f32 {
    const K: f32 = 1.0;
    let atk = attacker.attack.max(0.0);
    let def = defender.defense.max(0.0) + 1.0;
    (impact.max(0.0) * (atk / def) * K).max(0.0)
}

/// 부위 effect 프로필: 넉백/스턴/데미지 성향(StatSet 가중치 유래).
#[derive(Clone, Copy)]
pub struct EffectProfile {
    pub knockback: f32,
    pub stun: f32,
    pub damage: f32,
}

/// resolve_effects 결과: 이번 히트로 발동한 넉백/스턴/데미지 세기.
#[derive(Clone, Copy, Default)]
pub struct Effects {
    pub knockback: f32,
    pub stun: f32,
    pub damage: f32,
}

/// 임팩트 임계(튜닝): 이 이상이어야 해당 효과 발동.
const T_KNOCK: f32 = 0.8;
const T_STUN: f32 = 1.5;

/// 결정적. 임팩트 비례 중첩 + 프로필/저항 스케일.
/// 데미지는 항상(임팩트 비례), 넉백은 impact≥T_KNOCK, 스턴은 impact≥T_STUN일 때만 발동.
/// 세기 = weight × impact / resistance.max(1.0). 저항 하한으로 폭주 방지.
pub fn resolve_effects(p: &EffectProfile, impact: f32, resistance: f32) -> Effects {
    let i = impact.max(0.0);
    let r = resistance.max(1.0);
    let mut e = Effects::default();
    e.damage = p.damage * i / r;
    if i >= T_KNOCK {
        e.knockback = p.knockback * i / r;
    }
    if i >= T_STUN {
        e.stun = p.stun * i / r;
    }
    e
}

/// 파손 다운 지속 틱(3초 @60Hz). 튜닝 대상.
const REPAIR_TICKS: u32 = 180;

/// 로봇 1대의 부위별 HP + 파손 다운/리페어 타이머 (결정적 순수 상태).
/// 어떤 부위든 HP가 0에 닿으면 파손 다운 → 타이머 소진 시 **전체 부위** 리페어.
pub struct CombatState {
    max: Vec<f32>,
    hp: Vec<f32>,
    down_timer: u32,
    /// 스턴 남은 시간(초). 파손 다운과 달리 짧고 HP와 무관한 입력 무시 상태.
    stun_timer: f32,
}

impl CombatState {
    pub fn new(max_hp: &[f32]) -> Self {
        Self {
            max: max_hp.to_vec(),
            hp: max_hp.to_vec(),
            down_timer: 0,
            stun_timer: 0.0,
        }
    }

    pub fn broken(&self) -> bool {
        self.down_timer > 0
    }

    /// 스턴 중인지(입력 무시 판정용).
    pub fn stunned(&self) -> bool {
        self.stun_timer > 0.0
    }

    /// 스턴 부여. 갱신은 최대값(더 길면) — 누적 아님.
    pub fn apply_stun(&mut self, secs: f32) {
        self.stun_timer = self.stun_timer.max(secs);
    }

    #[cfg(test)]
    pub fn repair_ticks(&self) -> u32 {
        REPAIR_TICKS
    }

    /// 리페어까지 남은 초(스냅샷 `down.repair_in`용). 다운 아니면 0.
    pub fn repair_in(&self) -> f32 {
        self.down_timer as f32 * crate::world::DT
    }

    pub fn part_count(&self) -> usize {
        self.hp.len()
    }

    pub fn hp_ratio(&self, i: usize) -> f32 {
        if self.max[i] > 0.0 {
            self.hp[i] / self.max[i]
        } else {
            1.0
        }
    }

    /// 모든 부위 중 최소 HP비율(테스트/디버프 판정용).
    #[cfg(test)]
    pub fn hp_ratio_min(&self) -> f32 {
        (0..self.hp.len())
            .map(|i| self.hp_ratio(i))
            .fold(1.0_f32, f32::min)
    }

    pub fn apply_damage(&mut self, part: usize, dmg: f32) {
        if self.broken() {
            return;
        }
        self.hp[part] = (self.hp[part] - dmg).max(0.0);
        if self.hp.iter().any(|&h| h <= 0.0) {
            self.down_timer = REPAIR_TICKS;
        }
    }

    /// 다운 중 매 tick 호출. 타이머 소진 시 전체 리페어.
    pub fn tick_down(&mut self) {
        if self.down_timer > 0 {
            self.down_timer -= 1;
            if self.down_timer == 0 {
                self.hp = self.max.clone();
            }
        }
    }

    /// 매 tick: 스턴 타이머 감소 + 기존 다운 타이머 진행(스턴·다운 함께 진행).
    pub fn tick_status(&mut self) {
        if self.stun_timer > 0.0 {
            self.stun_timer = (self.stun_timer - crate::world::DT).max(0.0);
        }
        self.tick_down();
    }

    /// 강제 파손 다운(테스트 전용).
    #[cfg(test)]
    pub fn force_down(&mut self) {
        self.down_timer = REPAIR_TICKS;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_scales_with_impact_and_attack_over_defense() {
        // 공격력↑ 또는 impact↑ → 데미지↑, 방어력↑ → 데미지↓
        let atk = StatSet {
            attack: 10.0,
            ..Default::default()
        };
        let def_low = StatSet {
            defense: 2.0,
            ..Default::default()
        };
        let def_high = StatSet {
            defense: 8.0,
            ..Default::default()
        };
        let d_low = damage_on_contact(&atk, &def_low, 1.0);
        let d_high = damage_on_contact(&atk, &def_high, 1.0);
        let d_big = damage_on_contact(&atk, &def_low, 2.0);
        assert!(d_low > d_high, "방어 높으면 데미지 감소");
        assert!(d_big > d_low, "impact 크면 데미지 증가");
        assert!(d_low >= 0.0);
    }

    #[test]
    fn part_hp_depletes_and_triggers_down_then_repairs() {
        let mut cs = CombatState::new(&[40.0, 30.0]); // 2 부위
        assert!(!cs.broken());
        cs.apply_damage(0, 100.0); // 부위0 과다 피해
        assert!(cs.broken(), "부위 HP 0 → 파손 다운");
        // 다운 중 추가 피해는 무시(재트리거/중첩 없음)
        cs.apply_damage(1, 100.0);
        // 다운 지속 후 리페어
        for _ in 0..(cs.repair_ticks()) {
            cs.tick_down();
        }
        assert!(!cs.broken(), "일정 시간 뒤 전체 리페어");
        assert!(cs.hp_ratio(0) > 0.99, "리페어 시 부위0 HP 복구");
        assert!(cs.hp_ratio(1) > 0.99, "리페어 시 전체 부위 복구");
    }

    #[test]
    fn effects_stack_with_impact_and_scale_by_profile_and_resistance() {
        // 프로필: 넉백0.6 스턴0.3 데미지0.5
        let prof = EffectProfile {
            knockback: 0.6,
            stun: 0.3,
            damage: 0.5,
        };
        let weak = resolve_effects(&prof, 0.2, 1.0); // 약한 접촉
        let hard = resolve_effects(&prof, 2.0, 1.0); // 강한 태클
                                                     // 약한 접촉: 데미지만(넉백/스턴 임계 미달)
        assert!(weak.damage > 0.0 && weak.knockback == 0.0 && weak.stun == 0.0);
        // 강한 태클: 셋 다 발동, 데미지도 더 큼
        assert!(hard.damage > weak.damage && hard.knockback > 0.0 && hard.stun > 0.0);
        // 저항↑ → 효과↓
        let resisted = resolve_effects(&prof, 2.0, 4.0);
        assert!(resisted.damage < hard.damage);
    }

    #[test]
    fn stun_blocks_input_for_duration_then_clears() {
        let mut cs = CombatState::new(&[40.0]);
        assert!(!cs.stunned());
        cs.apply_stun(0.5); // 0.5초
        assert!(cs.stunned());
        // dt 진행하면 언젠가 해제
        let steps = (0.5 / crate::world::DT).ceil() as u32 + 1;
        for _ in 0..steps {
            cs.tick_status();
        }
        assert!(!cs.stunned());
    }

    #[test]
    fn zero_damage_does_not_trigger_down() {
        // 데미지 0(예: attack=0 로봇)은 파손 다운을 유발하지 않는다.
        let mut cs = CombatState::new(&[20.0, 20.0]);
        cs.apply_damage(0, 0.0);
        assert!(!cs.broken());
        assert!(cs.hp_ratio_min() > 0.99);
    }
}
