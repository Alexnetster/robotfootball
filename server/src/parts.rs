//! 파츠·스탯 데이터 모델 + 카탈로그 + 로드아웃 집계 (순수 로직, 결정성 안전).
//! `HashMap`은 파츠/프리셋 조회·집계(합산)에만 쓰여 순서 무관 → sim/스냅샷 경로 무순회.

use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default)]
pub struct StatSet {
    pub max_speed: f32,
    pub accel: f32,
    pub turn_rate: f32,
    pub mass: f32,
    // 정의만(Plan 3b/4에서 사용):
    pub kick_power: f32,
    pub attack: f32,
    pub defense: f32,
    pub hp: f32,
    // 전투 effect 프로필 성향(Plan 3c). 기본 0.0 = 효과 없음.
    pub kb_w: f32,   // 넉백 성향
    pub stun_w: f32, // 스턴 성향
    pub dmg_w: f32,  // 데미지 성향
    // 스태미나/달리기(KB-45, 문서 §8 최소 슬라이스: 걷기/달리기만, 오버히트 제외).
    pub sprint_speed: f32,   // 달리기 시 속도 상한(walk의 max_speed 대체)
    pub stamina_max: f32,    // 스태미나 용량(초 단위 소모량과 동일 축)
    pub stamina_regen: f32,  // 초당 회복량(달리지 않을 때)
}

impl StatSet {
    fn add(&mut self, o: &StatSet) {
        self.max_speed += o.max_speed;
        self.accel += o.accel;
        self.turn_rate += o.turn_rate;
        self.mass += o.mass;
        self.kick_power += o.kick_power;
        self.attack += o.attack;
        self.defense += o.defense;
        self.hp += o.hp;
        self.kb_w += o.kb_w;
        self.stun_w += o.stun_w;
        self.dmg_w += o.dmg_w;
        self.sprint_speed += o.sprint_speed;
        self.stamina_max += o.stamina_max;
        self.stamina_regen += o.stamina_regen;
    }
}

/// 기존 하드코딩(THRUST=6/TURN_RATE=3)과 등가인 기본 스탯.
/// mass는 콜라이더 밀도 유래 질량에 가산되므로 0=no-op(기존 거동 보존).
/// 실행 바이너리는 프리셋 집계를 쓰므로(main) 현재 소비처는 테스트 경로뿐.
#[cfg(test)]
pub fn default_stats() -> StatSet {
    StatSet {
        max_speed: 10.0,
        accel: 6.0,
        turn_rate: 3.0,
        mass: 0.0,
        // sprint_speed ≈ 1.6× max_speed, stamina_max = 3초분, 4초에 완전 회복.
        sprint_speed: 16.0,
        stamina_max: 3.0,
        stamina_regen: 0.75,
        ..Default::default()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Slot {
    Head,
    Neck,
    Body,
    ForelegL,
    ForelegR,
    HindlegL,
    HindlegR,
    Tail,
}

impl Slot {
    pub fn as_str(&self) -> &'static str {
        match self {
            Slot::Head => "head",
            Slot::Neck => "neck",
            Slot::Body => "body",
            Slot::ForelegL => "foreleg_l",
            Slot::ForelegR => "foreleg_r",
            Slot::HindlegL => "hindleg_l",
            Slot::HindlegR => "hindleg_r",
            Slot::Tail => "tail",
        }
    }
}

#[derive(Clone)]
pub struct Part {
    pub id: &'static str,
    pub slot: Slot,
    pub stats: StatSet,
}

/// 파츠 id 목록으로 표현한 로드아웃.
pub struct Loadout {
    pub parts: Vec<&'static str>,
}

pub struct Catalog {
    pub parts: HashMap<&'static str, Part>,
    pub presets: HashMap<&'static str, Loadout>,
}

/// 데이터 주도 카탈로그(개발자 배포). 값은 밸런싱 대상 초기값.
pub fn catalog() -> Catalog {
    let mut parts = HashMap::new();
    let mut add = |id, slot, s: StatSet| {
        parts.insert(id, Part { id, slot, stats: s });
    };
    // 이동 스탯은 다리가 주도(좌/우 한 쌍씩 기여). 몸통은 mass/hp/defense,
    // 목은 turn_rate. 로봇은 4족: 앞다리 L/R + 뒷다리 L/R.
    // 프리셋 총합이 default_stats(max_speed≈10/accel≈6/turn_rate=3)에 준하도록 구성.
    add(
        "body-std",
        Slot::Body,
        StatSet {
            mass: 1.0,
            hp: 40.0,
            defense: 6.0,
            // 무거운 몸통 → 스턴 성향(Plan 3c 초기값, 밸런싱 대상).
            stun_w: 0.5,
            // 스태미나 용량/회복(몸통이 주 기여, KB-45). 3초분, ~4초에 완전 회복.
            stamina_max: 3.0,
            stamina_regen: 0.75,
            // 차기 세기(KB-58 재조정): 임펄스=level×kick_power. 공 질량(~0.13kg) 대비
            // 이전 값(9)은 수십 m/s로 공을 쏴 벽 터널링·세기 무의미를 유발했다.
            // 강킥이 공 속도 상한(BALL_MAX_SPEED≈12) 근처가 되도록 축소. guard는 약간 낮게.
            kick_power: 1.05,
            ..Default::default()
        },
    );
    add(
        "body-light",
        Slot::Body,
        StatSet {
            mass: 0.7,
            hp: 30.0,
            defense: 4.0,
            // 가볍고 예리한 몸통 → 데미지 성향(Plan 3c 초기값, 밸런싱 대상).
            dmg_w: 0.4,
            // 가벼운 몸통은 스태미나 용량은 약간 적지만 회복은 더 빠름.
            stamina_max: 2.5,
            stamina_regen: 0.85,
            // 차기 세기(KB-58 재조정). striker는 강킥(강=속도 상한 근처). level×kick_power가
            // 임펄스라 공 질량 대비 과대하면 터널링. 강 1.0×1.4≈상한, 약 0.5×1.4는 그 절반.
            kick_power: 1.4,
            ..Default::default()
        },
    );
    // 스피드형 뒷다리(빠르지만 가속 낮음) — 좌/우. sprint_speed ≈ 1.6× max_speed.
    let hind_speed = StatSet {
        max_speed: 5.5,
        accel: 2.0,
        sprint_speed: 8.8,
        ..Default::default()
    };
    add("hind-speed-l", Slot::HindlegL, hind_speed);
    add("hind-speed-r", Slot::HindlegR, hind_speed);
    // 파워형 뒷다리(가속 높지만 최고속 낮음) — 좌/우. sprint_speed ≈ 1.6× max_speed.
    let hind_power = StatSet {
        max_speed: 4.0,
        accel: 3.5,
        sprint_speed: 6.4,
        ..Default::default()
    };
    add("hind-power-l", Slot::HindlegL, hind_power);
    add("hind-power-r", Slot::HindlegR, hind_power);
    // 표준 앞다리 — 좌/우
    let fore_std = StatSet {
        accel: 1.0,
        attack: 2.5,
        // 공격 부위(앞다리) → 넉백 성향(Plan 3c 초기값, 밸런싱 대상).
        kb_w: 0.6,
        ..Default::default()
    };
    add("fore-std-l", Slot::ForelegL, fore_std);
    add("fore-std-r", Slot::ForelegR, fore_std);
    add(
        "neck-std",
        Slot::Neck,
        StatSet {
            // 조준 속도(조작감 튜닝, 플레이테스트 1차): 3.0(≈172°/s, 굼뜸) → 6.0(≈344°/s).
            // 전차식 제어(회전 후 전진)에서 방향 전환 답답함의 주 원인이 낮은 회전율.
            turn_rate: 6.0,
            ..Default::default()
        },
    );
    add("head-std", Slot::Head, StatSet { ..Default::default() });
    add("tail-std", Slot::Tail, StatSet { ..Default::default() });

    let mut presets = HashMap::new();
    presets.insert(
        "striker",
        Loadout {
            parts: vec![
                "head-std",
                "neck-std",
                "body-light",
                "fore-std-l",
                "fore-std-r",
                "hind-speed-l",
                "hind-speed-r",
                "tail-std",
            ],
        },
    );
    presets.insert(
        "guard",
        Loadout {
            parts: vec![
                "head-std",
                "neck-std",
                "body-std",
                "fore-std-l",
                "fore-std-r",
                "hind-power-l",
                "hind-power-r",
                "tail-std",
            ],
        },
    );

    Catalog { parts, presets }
}

/// 프리셋 id의 총 스탯 = 부위 기여 합.
pub fn aggregate(cat: &Catalog, preset: &str) -> StatSet {
    let mut s = StatSet::default();
    if let Some(lo) = cat.presets.get(preset) {
        for pid in &lo.parts {
            if let Some(p) = cat.parts.get(pid) {
                s.add(&p.stats);
            }
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preset_aggregates_part_stats_and_presets_differ() {
        let cat = catalog();
        let striker = aggregate(&cat, "striker");
        let guard = aggregate(&cat, "guard");
        // 집계는 부위 기여 합
        assert!(striker.max_speed > 0.0 && striker.accel > 0.0);
        // 프리셋이 서로 다르다(비대칭)
        assert!(striker.max_speed != guard.max_speed || striker.accel != guard.accel);
    }

    /// 스프린트는 걷기보다 빨라야 유의미(KB-45). 출하 프리셋 전부에서 불변식 보장:
    /// sprint_speed > max_speed. (미래 파츠가 max만 올리고 sprint를 안 올리면 스프린트가
    /// 순수 열화—느리면서 스태미나만 소모—가 되는 회귀를 여기서 잡는다.)
    #[test]
    fn shipped_presets_sprint_faster_than_walk() {
        let cat = catalog();
        for preset in ["striker", "guard"] {
            let s = aggregate(&cat, preset);
            assert!(
                s.sprint_speed > s.max_speed,
                "{preset}: sprint_speed {} must exceed max_speed {}",
                s.sprint_speed,
                s.max_speed
            );
        }
    }

    /// kick_power 실배선(KB-48): 프리셋마다 양수여야 하고, striker(민첩/강킥)가
    /// guard(약간 낮게)보다 커야 한다는 카탈로그 의도를 회귀 방지로 고정.
    #[test]
    fn shipped_presets_have_positive_asymmetric_kick_power() {
        let cat = catalog();
        let striker = aggregate(&cat, "striker");
        let guard = aggregate(&cat, "guard");
        assert!(striker.kick_power > 0.0);
        assert!(guard.kick_power > 0.0);
        assert!(
            striker.kick_power > guard.kick_power,
            "striker({})가 guard({})보다 강킥이어야 함",
            striker.kick_power,
            guard.kick_power
        );
    }
}
