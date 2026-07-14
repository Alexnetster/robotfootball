/// 경과 시간을 고정 dt 스텝 수로 변환(잔여 누적). 순수·결정적.
///
/// spiral-of-death 방지를 위해 한 번의 `feed` 호출이 돌려주는 스텝 수를
/// `MAX_STEPS`로 상한 처리한다. 상한을 넘긴 초과분은 잔여(acc)에 남기지 않고
/// **버린다**(프레임이 밀려도 따라잡기를 포기해 폭주를 막는다).
///
/// 시간 누산은 결정적 코어 밖(main의 I/O 층)이라, dt·잔여를 배정도(f64)로
/// 유지해 f32 반올림 누적으로 스텝 경계가 어긋나는 것을 막는다.
pub struct Accumulator {
    acc: f64,
    dt: f64,
}

/// 한 프레임에서 소비할 수 있는 고정 스텝의 상한(spiral-of-death 방지).
pub const MAX_STEPS: u32 = 5;

impl Accumulator {
    pub fn new(dt: f32) -> Self {
        Accumulator {
            acc: 0.0,
            dt: dt as f64,
        }
    }

    /// elapsed를 더하고, 소비할 스텝 수를 반환(잔여는 보존, 단 상한 초과분은 폐기).
    pub fn feed(&mut self, elapsed: f32) -> u32 {
        self.acc += elapsed as f64;
        // 스텝 경계 비교는 상대 epsilon으로 완화한다: 여러 f32 조각(예: 2.5·dt +
        // 0.5·dt)이 반올림으로 정확히 3·dt에 못 미쳐 스텝을 놓치는 것을 막는다.
        let eps = self.dt * 1e-6;
        let mut n = 0;
        while self.acc + eps >= self.dt && n < MAX_STEPS {
            self.acc -= self.dt;
            n += 1;
        }
        // 상한에 걸렸으면 남은 잔여를 폐기해 따라잡기 폭주를 막는다.
        if n == MAX_STEPS {
            self.acc = 0.0;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_yields_fixed_steps_and_keeps_remainder() {
        let dt = 1.0 / 60.0;
        let mut a = Accumulator::new(dt);
        assert_eq!(a.feed(dt * 2.5), 2); // 2스텝, 0.5 잔여
        assert_eq!(a.feed(dt * 0.5), 1); // 잔여 합쳐 1스텝
    }

    #[test]
    fn feed_clamps_steps_to_max_and_drops_excess() {
        let dt = 1.0 / 60.0;
        let mut a = Accumulator::new(dt);
        // 큰 프레임 정체: 10스텝 분량을 먹여도 MAX_STEPS로 상한.
        assert_eq!(a.feed(dt * 10.0), MAX_STEPS);
        // 초과분은 폐기됐으므로 다음 프레임은 0스텝(잔여 없음).
        assert_eq!(a.feed(0.0), 0);
    }
}
