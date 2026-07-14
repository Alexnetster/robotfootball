use crate::session::{parse_uplink, SessionId, Uplink};
use crate::world::GameState;
use serde::Serialize;
use serde_json::Value;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
    routing::get,
    Router,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{mpsc, watch};
use tokio::time::{interval, Duration};

#[derive(Serialize)]
pub struct StateMsg<'a> {
    pub t: &'a str,
    pub state: &'a GameState,
}

/// 파츠 스탯(다운링크용). 문서 02 §4.2대로 중첩 `stats` 객체로 낸다.
#[derive(Serialize)]
pub struct StatsDto {
    pub max_speed: f32,
    pub accel: f32,
    pub turn_rate: f32,
    pub mass: f32,
    pub kick_power: f32,
    pub attack: f32,
    pub defense: f32,
    pub hp: f32,
}

#[derive(Serialize)]
pub struct PartDto {
    pub id: String,
    pub slot: String,
    pub stats: StatsDto,
}

#[derive(Serialize)]
pub struct CatalogMsg {
    pub t: &'static str,
    pub presets: Vec<String>,
    pub parts: Vec<PartDto>,
}

/// 파츠 카탈로그를 다운링크 DTO로 변환. 순수 함수(핸들러에서 직접 호출).
/// HashMap 순회는 JSON 배열 순서에만 영향(결정성 sim 경로 아님).
pub fn catalog_msg() -> CatalogMsg {
    let cat = crate::parts::catalog();
    let parts = cat
        .parts
        .values()
        .map(|p| PartDto {
            id: p.id.to_string(),
            slot: p.slot.as_str().to_string(),
            stats: StatsDto {
                max_speed: p.stats.max_speed,
                accel: p.stats.accel,
                turn_rate: p.stats.turn_rate,
                mass: p.stats.mass,
                kick_power: p.stats.kick_power,
                attack: p.stats.attack,
                defense: p.stats.defense,
                hp: p.stats.hp,
            },
        })
        .collect();
    let presets = cat.presets.keys().map(|k| k.to_string()).collect();
    CatalogMsg {
        t: "catalog",
        presets,
        parts,
    }
}

// ---------------------------------------------------------------------------
// 연결별 아웃바운드 링크 시뮬레이터(전송 계층 wrapper). 결정적 sim에는 영향 없음
// — 여기서 만드는 지연/드랍은 "서버가 클라에 보내는 JSON 문자열"에만 적용되고,
// sim 틱/물리에는 전혀 관여하지 않는다.
// ---------------------------------------------------------------------------

/// 연결의 아웃바운드 링크심 설정. 기본값(전부 0)은 "링크심 없음" = 기존 동작과 동일.
#[derive(Clone, Copy, Debug, PartialEq)]
struct NetSim {
    delay_ms: u64,
    jitter_ms: u64,
    drop_pct: f64,
}

impl Default for NetSim {
    fn default() -> Self {
        NetSim {
            delay_ms: 0,
            jitter_ms: 0,
            drop_pct: 0.0,
        }
    }
}

impl NetSim {
    /// 전부 0(무지연·무드랍)이면 기존 경로(즉시 송신)를 그대로 타야 한다.
    fn is_identity(&self) -> bool {
        self.delay_ms == 0 && self.jitter_ms == 0 && self.drop_pct == 0.0
    }
}

/// 외부 crate(rand) 없이 직접 구현한 xorshift64 PRNG. 전송 계층 지연/드랍
/// 롤에만 쓰이므로 sim 결정성과 무관(암호학적 용도 아님).
struct Xorshift64 {
    state: u64,
}

impl Xorshift64 {
    fn new(seed: u64) -> Self {
        // xorshift는 상태 0에서 고착(항상 0)되므로 0이면 임의 상수로 대체.
        Xorshift64 {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    /// `[0, bound)` 범위의 정수. `bound == 0`이면 항상 0.
    fn next_range(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            0
        } else {
            self.next_u64() % bound
        }
    }

    /// `[0, 100)` 범위의 실수(드랍 판정용 백분율 롤).
    fn roll_percent(&mut self) -> f64 {
        (self.next_u64() as f64 / u64::MAX as f64) * 100.0
    }
}

/// 드랍 판정(순수 함수, 타이밍 무관): `roll`은 `[0,100)` 백분율,
/// `drop_pct`는 연결의 드랍 확률(0..=100). `drop_pct<=0`이면 항상 false,
/// `drop_pct>=100`이면 `roll<100`이 항상 참이므로 항상 true.
fn should_drop(roll: f64, drop_pct: f64) -> bool {
    roll < drop_pct
}

/// `queue_out`의 판정 결과. 실제 소켓 I/O는 호출부(`handle_socket`)가 수행하고,
/// 이 함수 자체는 순수 로직(타이밍은 `Instant::now()`를 인자로 받아 테스트 가능)이다.
enum SendPlan {
    /// netsim이 전부 0 → 큐를 거치지 않고 즉시 송신(기존 경로 보존).
    Immediate(String),
    /// 지연 큐에 넣을 (송신 시각, 메시지).
    Delayed(Instant, String),
    /// drop 롤에 걸려 버림.
    Dropped,
}

/// 아웃바운드 링크심 헬퍼(`queue_out`). netsim이 전부 0이면 즉시 송신 경로를
/// 그대로 타고, 아니면 드랍 롤 → (드랍 안 됐으면) delay+jitter 지연 큐잉을 결정한다.
fn queue_out(msg: String, netsim: &NetSim, now: Instant, prng: &mut Xorshift64) -> SendPlan {
    if netsim.is_identity() {
        return SendPlan::Immediate(msg);
    }
    let roll = prng.roll_percent();
    if should_drop(roll, netsim.drop_pct) {
        return SendPlan::Dropped;
    }
    let jitter = prng.next_range(netsim.jitter_ms + 1);
    let deliver_at = now + Duration::from_millis(netsim.delay_ms + jitter);
    SendPlan::Delayed(deliver_at, msg)
}

/// 지연 큐에서 배출 가능한(deliver_at <= now) 항목을 **모두** 꺼내
/// deliver_at 오름차순(동시각 tie-break은 seq 삽입순서)으로 정렬해 돌려준다.
/// 아직 due가 아닌 항목은 큐에 그대로 남긴다. 순수 함수(소켓 I/O 없음).
///
/// front만 보고 break하면 지터로 뒤 항목의 deliver_at이 더 이른 경우
/// head-of-line 블로킹이 생겨 실측 지연이 설정값을 크게 벗어난다 — 그래서
/// 전체를 스캔해 due 항목을 deliver_at 순으로 내보낸다(큐가 작아 O(n)로 충분).
fn drain_due(queue: &mut VecDeque<(Instant, u64, String)>, now: Instant) -> Vec<String> {
    let mut due: Vec<(Instant, u64, String)> = Vec::new();
    let mut remaining: VecDeque<(Instant, u64, String)> = VecDeque::new();
    while let Some(item) = queue.pop_front() {
        if item.0 <= now {
            due.push(item);
        } else {
            remaining.push_back(item);
        }
    }
    *queue = remaining;
    due.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    due.into_iter().map(|(_, _, m)| m).collect()
}

/// net 계층 전용 제어 메시지(게임 uplink 아님 — `session::Uplink`와 별개).
enum NetCtrl {
    /// `{"t":"ping","id":<number>}` — id는 그대로 에코해야 하므로 원본 `Value` 보존.
    Ping(Value),
    /// `{"t":"netsim","delay_ms":..,"jitter_ms":..,"drop_pct":..}`.
    Sim(NetSim),
}

/// `ping`/`netsim` 업링크 파싱(순수 함수). 그 외 타입은 `None`
/// (join/input/leave는 `session::parse_uplink`가 처리하므로 여기서 관여하지 않음).
fn parse_net_ctrl(s: &str) -> Option<NetCtrl> {
    let v: Value = serde_json::from_str(s).ok()?;
    match v.get("t")?.as_str()? {
        "ping" => {
            let id = v.get("id")?.clone();
            Some(NetCtrl::Ping(id))
        }
        "netsim" => {
            // 소수(예: 15.5)도 조용히 0으로 떨구지 않고 반올림 수용. 음수는 0으로 클램프.
            let ms_field = |key: &str| {
                v.get(key)
                    .and_then(Value::as_f64)
                    .map(|x| x.max(0.0).round() as u64)
                    .unwrap_or(0)
            };
            let delay_ms = ms_field("delay_ms");
            let jitter_ms = ms_field("jitter_ms");
            let drop_pct = v
                .get("drop_pct")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .clamp(0.0, 100.0);
            Some(NetCtrl::Sim(NetSim {
                delay_ms,
                jitter_ms,
                drop_pct,
            }))
        }
        _ => None,
    }
}

type Shared = Arc<watch::Receiver<GameState>>;
type UplinkTx = mpsc::UnboundedSender<(SessionId, Uplink)>;

/// axum state: 다운링크 소스(watch rx) + 업링크 목적지(mpsc tx).
/// UnboundedSender는 Clone+Send+Sync라 Arc 불필요.
#[derive(Clone)]
struct AppState {
    watch_rx: Shared,
    uplink_tx: UplinkTx,
}

/// WS 접속마다 발급되는 세션 id 카운터. `ws_handler` 진입 시 증가.
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

pub async fn serve(watch_rx: Shared, uplink_tx: UplinkTx) {
    let state = AppState {
        watch_rx,
        uplink_tx,
    };
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8090")
        .await
        .unwrap();
    println!("listening on ws://localhost:8090/ws");
    axum::serve(listener, app).await.unwrap();
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    let sid = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    ws.on_upgrade(move |socket| handle_socket(socket, state, sid))
}

/// 다운링크(30Hz state push)와 업링크(recv→parse→mpsc)를 **한 태스크**에서
/// `select!`로 동시 처리한다. (split()/futures-util 비권장 — 드라이런 확정 사항.)
///
/// KB-43(Plan 7a): 연결별 아웃바운드 링크 시뮬레이터(`netsim`)를 추가.
/// `netsim`이 기본값(전부 0)인 동안은 `queue_out`이 즉시 송신 경로를 그대로 타므로
/// 기존 동작(지연 큐 없음)과 완전히 동일하다.
async fn handle_socket(mut socket: WebSocket, state: AppState, sid: SessionId) {
    // 접속 시 카탈로그 1회 전송(welcome 메시지는 없음) — 링크심 큐 우회, 틱 루프 진입 전.
    let cat_json = serde_json::to_string(&catalog_msg()).unwrap();
    if socket.send(Message::Text(cat_json)).await.is_err() {
        return;
    }

    let mut netsim = NetSim::default();
    // (deliver_at, seq, msg) — seq는 동시각 tie-break용 삽입 순서.
    let mut out_queue: VecDeque<(Instant, u64, String)> = VecDeque::new();
    let mut out_seq: u64 = 0;
    // 시드는 세션마다 달라야 충분(암호학적 강도 불필요 — 전송 계층 전용).
    let mut prng = Xorshift64::new(sid ^ 0x9E3779B97F4A7C15);

    let mut tick = interval(Duration::from_millis(33)); // ~30Hz 스냅샷 생성
    let mut flush = interval(Duration::from_millis(5)); // 지연 큐 배출 틱

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let snapshot = state.watch_rx.borrow().clone();
                let msg = StateMsg { t: "state", state: &snapshot };
                let json = serde_json::to_string(&msg).unwrap();
                match queue_out(json, &netsim, Instant::now(), &mut prng) {
                    SendPlan::Immediate(m) => {
                        if socket.send(Message::Text(m)).await.is_err() {
                            break;
                        }
                    }
                    SendPlan::Delayed(at, m) => {
                        out_queue.push_back((at, out_seq, m));
                        out_seq += 1;
                    }
                    SendPlan::Dropped => {}
                }
            }
            _ = flush.tick() => {
                let mut disconnected = false;
                for m in drain_due(&mut out_queue, Instant::now()) {
                    if socket.send(Message::Text(m)).await.is_err() {
                        disconnected = true;
                        break;
                    }
                }
                if disconnected {
                    break;
                }
            }
            msg = socket.recv() => match msg {
                Some(Ok(Message::Text(s))) => {
                    match parse_net_ctrl(&s) {
                        Some(NetCtrl::Ping(id)) => {
                            let pong = serde_json::json!({"t": "pong", "id": id}).to_string();
                            match queue_out(pong, &netsim, Instant::now(), &mut prng) {
                                SendPlan::Immediate(m) => {
                                    if socket.send(Message::Text(m)).await.is_err() {
                                        break;
                                    }
                                }
                                SendPlan::Delayed(at, m) => {
                                    out_queue.push_back((at, out_seq, m));
                                    out_seq += 1;
                                }
                                SendPlan::Dropped => {}
                            }
                        }
                        Some(NetCtrl::Sim(ns)) => {
                            netsim = ns;
                        }
                        None => {
                            if let Some(u) = parse_uplink(&s) {
                                let _ = state.uplink_tx.send((sid, u));
                            }
                        }
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => break,
            }
        }
    }
    // 이탈/끊김 → sim 태스크가 해당 세션의 슬롯을 AI로 복귀.
    let _ = state.uplink_tx.send((sid, Uplink::Leave));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::GameState;

    #[test]
    fn state_serializes_to_json_with_type_tag() {
        let s = GameState::new_kickoff();
        let msg = StateMsg {
            t: "state",
            state: &s,
        };
        let j = serde_json::to_string(&msg).unwrap();
        assert!(j.contains("\"t\":\"state\""));
        assert!(j.contains("\"score\""));
    }

    #[test]
    fn catalog_msg_serializes_parts_and_presets() {
        let j = serde_json::to_string(&catalog_msg()).unwrap();
        assert!(j.contains("\"t\":\"catalog\""));
        assert!(j.contains("striker"));
    }

    // -- xorshift PRNG ------------------------------------------------------

    #[test]
    fn xorshift_cycles_through_nonzero_values() {
        let mut prng = Xorshift64::new(12345);
        let mut seen_distinct = false;
        let mut prev = None;
        for _ in 0..10 {
            let v = prng.next_u64();
            assert_ne!(v, 0, "xorshift는 0이 아닌 값을 내야 함");
            if let Some(p) = prev {
                if p != v {
                    seen_distinct = true;
                }
            }
            prev = Some(v);
        }
        assert!(seen_distinct, "연속 호출은 서로 다른 값을 순환 생성해야 함");
    }

    #[test]
    fn xorshift_seed_zero_is_replaced_and_still_cycles() {
        let mut prng = Xorshift64::new(0);
        let a = prng.next_u64();
        let b = prng.next_u64();
        assert_ne!(a, 0);
        assert_ne!(b, 0);
        assert_ne!(a, b);
    }

    #[test]
    fn roll_percent_stays_in_zero_to_hundred_range() {
        let mut prng = Xorshift64::new(999);
        for _ in 0..50 {
            let r = prng.roll_percent();
            assert!((0.0..100.0).contains(&r), "roll_percent는 [0,100) 범위: {r}");
        }
    }

    // -- drop 판정 ------------------------------------------------------------

    #[test]
    fn drop_pct_100_always_drops() {
        let mut prng = Xorshift64::new(42);
        for _ in 0..50 {
            let roll = prng.roll_percent();
            assert!(should_drop(roll, 100.0), "drop_pct=100이면 항상 드랍");
        }
    }

    #[test]
    fn drop_pct_0_never_drops() {
        let mut prng = Xorshift64::new(43);
        for _ in 0..50 {
            let roll = prng.roll_percent();
            assert!(!should_drop(roll, 0.0), "drop_pct=0이면 절대 드랍 안 함");
        }
    }

    #[test]
    fn drop_pct_boundary_is_pure_comparison() {
        assert!(!should_drop(50.0, 50.0), "roll==drop_pct는 드랍 아님(roll<drop_pct만 드랍)");
        assert!(should_drop(49.9, 50.0));
        assert!(!should_drop(50.1, 50.0));
    }

    // -- queue_out(netsim 결정 로직) -------------------------------------------

    #[test]
    fn queue_out_is_immediate_when_netsim_is_identity() {
        let netsim = NetSim::default();
        let mut prng = Xorshift64::new(7);
        let plan = queue_out("hello".to_string(), &netsim, Instant::now(), &mut prng);
        assert!(matches!(plan, SendPlan::Immediate(m) if m == "hello"));
    }

    #[test]
    fn queue_out_drops_when_drop_pct_100() {
        let netsim = NetSim {
            delay_ms: 0,
            jitter_ms: 0,
            drop_pct: 100.0,
        };
        let mut prng = Xorshift64::new(7);
        let plan = queue_out("x".to_string(), &netsim, Instant::now(), &mut prng);
        assert!(matches!(plan, SendPlan::Dropped));
    }

    #[test]
    fn queue_out_delays_when_delay_configured_and_not_dropped() {
        let netsim = NetSim {
            delay_ms: 100,
            jitter_ms: 0,
            drop_pct: 0.0,
        };
        let mut prng = Xorshift64::new(7);
        let now = Instant::now();
        let plan = queue_out("x".to_string(), &netsim, now, &mut prng);
        match plan {
            SendPlan::Delayed(at, m) => {
                assert_eq!(m, "x");
                assert!(at >= now + Duration::from_millis(100));
            }
            _ => panic!("delay_ms>0이면 Delayed여야 함"),
        }
    }

    // -- parse_net_ctrl -------------------------------------------------------

    // -- drain_due(지연 큐 순서 보장) ------------------------------------------

    #[test]
    fn drain_due_emits_due_items_in_deliver_at_order_despite_insertion_order() {
        // 지터로 인해 삽입 순서와 deliver_at 순서가 어긋난 상황 재현.
        let base = Instant::now();
        let mut q: VecDeque<(Instant, u64, String)> = VecDeque::new();
        // 삽입 순서: 늦은 것 먼저, 이른 것 나중(비-monotonic deliver_at).
        q.push_back((base + Duration::from_millis(30), 0, "c".to_string()));
        q.push_back((base + Duration::from_millis(10), 1, "a".to_string()));
        q.push_back((base + Duration::from_millis(20), 2, "b".to_string()));
        // 아직 due 아님(미래) — 남아야 함.
        q.push_back((base + Duration::from_millis(999), 3, "future".to_string()));

        let now = base + Duration::from_millis(50);
        let out = drain_due(&mut q, now);
        assert_eq!(out, vec!["a", "b", "c"], "due 항목은 deliver_at 오름차순 배출");
        assert_eq!(q.len(), 1, "미래 항목은 큐에 남아야 함");
        assert_eq!(q.front().unwrap().2, "future");
    }

    #[test]
    fn drain_due_tie_breaks_on_insertion_seq() {
        let base = Instant::now();
        let at = base + Duration::from_millis(5);
        let mut q: VecDeque<(Instant, u64, String)> = VecDeque::new();
        // 동일 deliver_at — seq 순서(삽입순)로 안정 정렬돼야 함.
        q.push_back((at, 2, "second".to_string()));
        q.push_back((at, 0, "first".to_string()));
        q.push_back((at, 1, "middle".to_string()));
        let out = drain_due(&mut q, base + Duration::from_millis(10));
        assert_eq!(out, vec!["first", "middle", "second"]);
    }

    #[test]
    fn drain_due_keeps_all_when_nothing_due() {
        let base = Instant::now();
        let mut q: VecDeque<(Instant, u64, String)> = VecDeque::new();
        q.push_back((base + Duration::from_millis(100), 0, "x".to_string()));
        let out = drain_due(&mut q, base);
        assert!(out.is_empty());
        assert_eq!(q.len(), 1);
    }

    #[test]
    fn netsim_accepts_fractional_ms_by_rounding() {
        // 15.5 -> 16, 4.4 -> 4 (조용한 0 폴백 금지).
        let ctrl = parse_net_ctrl(r#"{"t":"netsim","delay_ms":15.5,"jitter_ms":4.4}"#);
        match ctrl {
            Some(NetCtrl::Sim(ns)) => {
                assert_eq!(ns.delay_ms, 16);
                assert_eq!(ns.jitter_ms, 4);
            }
            _ => panic!("netsim 파싱 실패"),
        }
    }

    #[test]
    fn netsim_negative_ms_clamps_to_zero() {
        let ctrl = parse_net_ctrl(r#"{"t":"netsim","delay_ms":-5}"#);
        match ctrl {
            Some(NetCtrl::Sim(ns)) => assert_eq!(ns.delay_ms, 0),
            _ => panic!("netsim 파싱 실패"),
        }
    }

    #[test]
    fn parses_ping_with_numeric_id() {
        let ctrl = parse_net_ctrl(r#"{"t":"ping","id":42}"#);
        match ctrl {
            Some(NetCtrl::Ping(id)) => assert_eq!(id, serde_json::json!(42)),
            _ => panic!("ping 파싱 실패"),
        }
    }

    #[test]
    fn parses_netsim_fields() {
        let ctrl = parse_net_ctrl(r#"{"t":"netsim","delay_ms":120,"jitter_ms":30,"drop_pct":5.5}"#);
        match ctrl {
            Some(NetCtrl::Sim(ns)) => {
                assert_eq!(ns.delay_ms, 120);
                assert_eq!(ns.jitter_ms, 30);
                assert!((ns.drop_pct - 5.5).abs() < f64::EPSILON);
            }
            _ => panic!("netsim 파싱 실패"),
        }
    }

    #[test]
    fn netsim_missing_fields_default_to_zero() {
        let ctrl = parse_net_ctrl(r#"{"t":"netsim"}"#);
        match ctrl {
            Some(NetCtrl::Sim(ns)) => assert_eq!(ns, NetSim::default()),
            _ => panic!("netsim 파싱 실패"),
        }
    }

    #[test]
    fn netsim_drop_pct_is_clamped_to_0_100() {
        let ctrl = parse_net_ctrl(r#"{"t":"netsim","drop_pct":150}"#);
        match ctrl {
            Some(NetCtrl::Sim(ns)) => assert_eq!(ns.drop_pct, 100.0),
            _ => panic!("netsim 파싱 실패"),
        }
    }

    #[test]
    fn parse_net_ctrl_ignores_game_uplinks_and_garbage() {
        assert!(parse_net_ctrl(r#"{"t":"join","slot":"blue"}"#).is_none());
        assert!(parse_net_ctrl(r#"{"t":"input","fwd":true}"#).is_none());
        assert!(parse_net_ctrl(r#"{"t":"leave"}"#).is_none());
        assert!(parse_net_ctrl("garbage").is_none());
        assert!(parse_net_ctrl(r#"{"t":"ping"}"#).is_none(), "id 없는 ping은 None");
    }
}
