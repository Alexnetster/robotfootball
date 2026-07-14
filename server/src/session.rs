use crate::world::{ControlOutput, Team};
use serde_json::Value;

/// WS 접속 세션 식별자. `net.rs`가 `AtomicU64` 카운터로 발급한다.
pub type SessionId = u64;

/// WS 업링크(클라 → 서버) 파싱 결과. 로드아웃은 4b로 이연(join엔 slot만).
#[derive(Clone)]
pub enum Uplink {
    Join(Team),
    Input(ControlOutput),
    Leave,
}

/// 업링크 JSON 문자열 파싱. 미지 타입/기형 JSON은 조용히 `None`(무시).
/// 서버 권위: 여기선 스키마만 검증하고, 상태 적합성(다운/스턴 중 무시 등)은
/// 이미 physics 쪽에서 처리한다.
pub fn parse_uplink(s: &str) -> Option<Uplink> {
    let v: Value = serde_json::from_str(s).ok()?;
    match v.get("t")?.as_str()? {
        "join" => {
            let team = match v.get("slot")?.as_str()? {
                "blue" => Team::Blue,
                "red" => Team::Red,
                _ => return None,
            };
            Some(Uplink::Join(team))
        }
        "input" => {
            let fwd = v.get("fwd").and_then(Value::as_bool).unwrap_or(false);
            let back = v.get("back").and_then(Value::as_bool).unwrap_or(false);
            let turn = v.get("turn").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            let run = v.get("run").and_then(Value::as_bool).unwrap_or(false);
            // 차기(KB-48): 모드리스 탭 발사 플래그. 기본 false(미지정 시 발사 없음).
            let kick = v.get("kick").and_then(Value::as_bool).unwrap_or(false);
            let thrust = if fwd { 1.0 } else if back { -1.0 } else { 0.0 };
            Some(Uplink::Input(ControlOutput {
                thrust,
                turn: turn.clamp(-1.0, 1.0),
                run,
                kick,
            }))
        }
        "leave" => Some(Uplink::Leave),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_join_and_input_uplink() {
        assert!(matches!(
            parse_uplink(r#"{"t":"join","slot":"blue"}"#),
            Some(Uplink::Join(Team::Blue))
        ));
        assert!(matches!(
            parse_uplink(r#"{"t":"join","slot":"red"}"#),
            Some(Uplink::Join(Team::Red))
        ));
        let u = parse_uplink(r#"{"t":"input","fwd":true,"turn":-1}"#);
        assert!(matches!(u, Some(Uplink::Input(_))));
        if let Some(Uplink::Input(out)) = u {
            assert_eq!(out.thrust, 1.0);
            assert_eq!(out.turn, -1.0);
            assert!(!out.run, "run 미지정 시 기본 false");
        }
        assert!(matches!(parse_uplink(r#"{"t":"leave"}"#), Some(Uplink::Leave)));
    }

    #[test]
    fn parses_run_flag_on_input_uplink() {
        let u = parse_uplink(r#"{"t":"input","fwd":true,"turn":0,"run":true}"#);
        if let Some(Uplink::Input(out)) = u {
            assert!(out.run, "run:true가 파싱되어야 함");
        } else {
            panic!("input 파싱 실패");
        }
    }

    #[test]
    fn parses_kick_flag_on_input_uplink() {
        let u = parse_uplink(r#"{"t":"input","fwd":true,"turn":0,"kick":true}"#);
        if let Some(Uplink::Input(out)) = u {
            assert!(out.kick, "kick:true가 파싱되어야 함");
        } else {
            panic!("input 파싱 실패");
        }
        // 미지정 시 기본 false
        let u2 = parse_uplink(r#"{"t":"input","fwd":true,"turn":0}"#);
        if let Some(Uplink::Input(out)) = u2 {
            assert!(!out.kick, "kick 미지정 시 기본 false");
        } else {
            panic!("input 파싱 실패");
        }
    }

    #[test]
    fn garbage_and_unknown_types_are_ignored() {
        assert!(parse_uplink("garbage").is_none());
        assert!(parse_uplink(r#"{"t":"unknown"}"#).is_none());
        assert!(parse_uplink(r#"{"t":"join","slot":"green"}"#).is_none());
    }
}
