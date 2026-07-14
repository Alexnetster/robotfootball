# 사람 조작 최소 슬라이스 (Playable Slice) — Plan 4a

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development. 체크박스(`- [ ]`) 추적.

**Goal:** 브라우저에서 사람이 **슬롯을 잡고 키보드로 로봇을 직접 조종**한다. 클라는 최소한 **현 전투 상태(파츠 HP·다운·스턴)** 를 표시한다. → 지금까지 구축한 물리·전투·효과가 **사람 조작으로 처음 라이브로** 보인다. (지금은 대칭 AI 평형이라 안 보임 — [09-AI](../../09-AI-설계.md), Plan 2/3b/3c 반복 관찰)

**Architecture:** 현재 `main.rs`는 sim 루프를 **spawn된 태스크**에서 돌리며 `watch<GameState>` 로 브로드캐스트(다운링크 전용). 사람 입력을 넣으려면 **WS 핸들러 → sim 태스크** 방향 채널(**mpsc**)이 필요하다: WS 세션이 `join`/`input`/`leave` 이벤트를 mpsc로 보내고, sim 태스크가 매 tick 드레인해 적용. 슬롯의 Controller를 AI ↔ **HumanController** 로 스왑(**Controller 추상화 회수**). 이탈/끊김 시 AI 복귀. 결정성 코어(step/tick)는 불변.

**Tech Stack:** tokio mpsc(신규 외부 API 없음), axum ws recv, 기존 스택. 클라 canvas 렌더 확장. 설계: [02 §3.2 업링크](../../02-네트워크-프로토콜.md), [00 §9 Controller](../../00-개요-및-게임설계.md), [01 §3 입력 매핑](../../01-UX-화면구성.md).

> **드라이런 완료(axum 0.7+tokio 프로브 검증)** — 아래 필수 반영 참조.

## ⚠️ 착수 전 필수 반영 (드라이런 점검 — 이 목록이 태스크 코드보다 우선)

프로브로 비동기 배선 컴파일 확인, 신규 rapier API 없음. 반영 사항:

1. **[핵심] WS send+recv = `tokio::select!` (신규 dep 0).** Task 3 Step 2의 단독 `recv` 루프는 **동시 송신 불가** → 한 태스크 안에서:
   ```rust
   loop { tokio::select! {
     _ = tick.tick() => { if socket.send(Message::Text(state.watch_rx.borrow().clone())).await.is_err() { break; } }
     msg = socket.recv() => match msg {
       Some(Ok(Message::Text(s))) => { let _ = state.uplink_tx.send((sid, s)); }
       Some(Ok(_)) => {}, Some(Err(_)) | None => break,
     }
   }}
   ```
   (split()는 `futures-util` 필요 + 오늘 crates.io 핀 이슈(`0.3.31` 강제) → **비권장**.)
2. **net.rs 시그니처**: `serve(rx)` → **`serve(watch_rx, uplink_tx)`** (또는 `AppState{watch_rx, uplink_tx}` State). `mpsc::UnboundedSender`는 Clone+Send+Sync → Arc 불필요.
3. **mpsc 배선**: main에서 `mpsc::unbounded_channel::<(SessionId,Uplink)>()`; tx→State, rx→sim 태스크 `move`. sim 루프는 매 프레임 **논블로킹 드레인** `while let Ok((sid,u)) = rx.try_recv() { slots.apply(u, sid); }`. 컨트롤러는 sim 태스크 배타 소유 → **Mutex 불필요**.
4. **세션ID**: `AtomicU64` 카운터를 `ws_handler` 진입 시 증가시켜 발급.
5. **슬롯 경합**(Task 4): 이미 다른 세션이 점유한 슬롯 join은 **거부/무시** — 테스트 `join_rejected_when_slot_already_taken` 추가.
6. **loadout 이연**: 이번 슬라이스 `join`은 `slot`만. `loadout`은 4b. 프리셋(striker/guard)은 **기존 하드코딩 유지**([02 §4.3]의 축소).
7. **결정성 근거**: 다운/스턴 입력무시는 이미 physics에 구현 → `HumanController`는 최근 입력만 반환. mpsc 주입은 I/O 경계, 코어(step/tick) 순수 유지.

---

## File Structure
- Create: `server/src/human.rs` — `HumanController`(최근 입력 보유, `decide`가 그 입력 반환) + `SlotInput`(공유 입력 상태)
- Create: `server/src/session.rs` — 업링크 메시지(`Join`/`Input`/`Leave`) 파싱 타입 + WS→sim 이벤트 enum
- Modify: `server/src/net.rs` — WS **recv** 루프(메시지 파싱 → mpsc 송신), 세션 생명주기
- Modify: `server/src/main.rs` — mpsc 수신부, sim 태스크가 join/input/leave 적용, 슬롯 Controller 스왑
- Modify: `server/src/control.rs` — (필요 시) Controller 트레잇 재사용
- Modify: `client/src/input.ts`(신규) — 키보드 캡처 → `input` 송신, `join` 버튼
- Modify: `client/src/net.ts` — 업링크 송신(`send`), 타입 확장(parts/down/st 수신)
- Modify: `client/src/render.ts` — **HP바·스턴/다운 표시**(캐치업)

---

## Task 1: HumanController + 입력 상태 (순수)

**Files:** Create `server/src/human.rs`; `main.rs`(`mod human;`)

- [ ] **Step 1: 실패 테스트** — `HumanController`가 보유한 입력을 `decide`로 반환.
```rust
#[test]
fn human_controller_returns_held_input() {
    let mut hc = HumanController::default();
    hc.set(ControlOutput { thrust: 1.0, turn: -1.0 });
    let view = /* 더미 GameView */;
    let out = hc.decide(&view);
    assert_eq!(out.thrust, 1.0);
    assert_eq!(out.turn, -1.0);
}
```
- [ ] **Step 2~4**: `HumanController { last: ControlOutput }` + `set()` + `Controller for HumanController`(decide=last 반환). 통과 확인.
- [ ] **Step 5: Commit** — `[KB-36] feat: HumanController (holds latest input)`

---

## Task 2: 업링크 메시지 파싱 (순수)

**Files:** Create `server/src/session.rs`; `main.rs`(`mod session;`)

- [ ] **Step 1: 실패 테스트** — JSON → 이벤트 파싱.
```rust
#[test]
fn parses_join_and_input_uplink() {
    assert!(matches!(parse_uplink(r#"{"t":"join","slot":"blue"}"#), Some(Uplink::Join(Team::Blue))));
    let u = parse_uplink(r#"{"t":"input","fwd":true,"turn":-1}"#);
    assert!(matches!(u, Some(Uplink::Input(_))));
    assert!(parse_uplink("garbage").is_none()); // 기형 무시
}
```
- [ ] **Step 2~4**: `enum Uplink { Join(Team), Input(ControlOutput), Leave }` + `parse_uplink(&str)->Option<Uplink>`(serde, **미지/기형은 None**). 통과.
- [ ] **Step 5: Commit** — `[KB-37] feat: uplink message parsing (join/input/leave)`

---

## Task 3: WS recv → mpsc, 세션 생명주기

**Files:** Modify `server/src/net.rs`, `server/src/main.rs`

- [ ] **Step 1: 설계 배선** — `main.rs`에서 `tokio::sync::mpsc::unbounded_channel::<(SessionId, Uplink)>()` 생성. `tx`를 WS 핸들러에 공유, `rx`는 sim 태스크로.
- [ ] **Step 2: net.rs recv 루프** — 기존 push(다운링크)와 **동시에** recv: `while let Some(Ok(Message::Text(s))) = socket.recv().await { if let Some(u)=parse_uplink(&s) { tx.send((sid,u)); } }`. 세션 종료 시 `Leave` 전송. (send/recv 동시 → `socket.split()` 또는 select. **드라이런에서 axum 0.7 ws split/recv 확정.**)
- [ ] **Step 3: 스모크** — 서버 기동 후 WS로 `join`/`input` 보내면 로그로 수신 확인(단위 테스트는 파싱까지, 소켓은 수동).
- [ ] **Step 4: Commit** — `[KB-38] feat: websocket uplink recv → mpsc`

---

## Task 4: 슬롯 Controller 스왑 (사람↔AI) + 입력 적용

**Files:** Modify `server/src/main.rs`

- [ ] **Step 1: 실패 테스트(순수 헬퍼)** — 슬롯 컨트롤러 스왑 로직을 순수 함수로 뽑아 테스트: `join`→그 슬롯 HumanController, `leave`→AI 복귀.
```rust
#[test]
fn join_swaps_slot_to_human_leave_reverts_to_ai() {
    let mut slots = SlotControllers::new_ai(); // [AI, AI]
    slots.apply(Uplink::Join(Team::Blue), sid1);
    assert!(slots.is_human(0));
    slots.apply(Uplink::Leave, sid1);
    assert!(!slots.is_human(0)); // AI 복귀
}
```
- [ ] **Step 2~4**: sim 태스크 루프에서 매 tick **mpsc 드레인** → `SlotControllers.apply(uplink)`(join/leave 스왑, input은 해당 슬롯 HumanController.set). `tick`은 그대로 컨트롤러들로 구동. 통과.
- [ ] **Step 5: Commit** — `[KB-39] feat: slot controller swap (human↔AI) + input apply`

---

## Task 5: 클라 입력 캡처 + 참가 버튼

**Files:** Create `client/src/input.ts`; Modify `client/src/net.ts`, `client/src/main.ts`, `client/index.html`

- [ ] **Step 1**: `net.ts`에 `send(msg)` 추가(WS 열림 후). `input.ts`: keydown/keyup → 현재 입력상태(fwd/back/turn/…) 계산, **변화 시** `{t:"input",...}` 송신([01 §3] 키매핑: ↑↓ 이동, ←→ 회전, Space 슛 등 — 3c까지의 액션 중 이동/회전 우선).
- [ ] **Step 2**: `index.html`에 `[BLUE로 참가][RED로 참가]` 버튼 → `{t:"join","slot":...}` 송신.
- [ ] **Step 3**: `npm run build` 타입체크 통과. (브라우저 조작은 KB-42 E2E)
- [ ] **Step 4: Commit** — `[KB-40] feat: client keyboard input + join buttons`

---

## Task 6: 클라 렌더 캐치업 (HP·스턴·다운)

**Files:** Modify `client/src/net.ts`(타입), `client/src/render.ts`

- [ ] **Step 1**: `net.ts` `Robot` 타입에 `parts?:[string,number][]`, `down?:{broken:boolean,repair_in:number}`, `st?:string[]` 추가(서버 스냅샷과 일치).
- [ ] **Step 2**: `render.ts`: 각 로봇 위에 **HP바**(parts 최소 비율), **스턴/다운 아이콘**(st에 "stun"/"downed"), 다운 시 로봇 흐리게.
- [ ] **Step 3**: `npm run build` 통과.
- [ ] **Step 4: Commit** — `[KB-41] feat: client renders HP/stun/down`

---

## Task 7: E2E + 결정성 + 문서/KANBAN

- [ ] **Step 1**: `cargo test` 전부 통과, warning 0(debug+release). `npm run build` 통과.
- [ ] **Step 2 (E2E)**: `cargo run` + `npm run dev` → 브라우저에서 **BLUE 참가 + 키보드로 로봇을 몰아 공을 밀고**, (비대칭/충돌 시) 상대와 부딪혀 **HP 감소·스턴/다운·넉백이 화면에 보이는지** 확인. 이탈 시 AI 복귀. (포트 8090, 서버 정리)
- [ ] **Step 3**: 리플레이 — 사람 입력이 들어오면 결정성은 "입력 기록 재생" 전제. 이번 슬라이스에선 **AI-only 골든 리플레이 유지**(사람 입력 리플레이는 후속). 노트만.
- [ ] **Step 4**: [KANBAN] Plan 4a Done. [09-AI]·[02] 갱신 반영 확인.
- [ ] **Step 5: Commit** — `[KB-42] docs: playable slice done, kanban update`

---

## Self-Review 결과
- **스펙 커버리지**: 업링크(join/input/leave, [02 §3.2]) ✅, HumanController(Controller 회수, [00 §9]) ✅, 슬롯 스왑(사람↔AI) ✅, 클라 입력·참가·상태렌더 ✅. **전략 모드·AI 토글·풀 게임흐름 UI(ATTRACT/SELECT/RESULT) = Plan 4b/5.**
- **결정성**: step/tick 순수 유지. 사람 입력은 I/O 경계(mpsc)에서만 주입 → 코어 결정성 불변. 사람-입력 골든 리플레이는 후속(입력 스트림 기록).
- **타입 일관성**: `HumanController`/`Uplink`/`parse_uplink`/`SlotControllers`/클라 `send`·타입확장 명칭 태스크 간 일치. 스냅샷 필드(parts/down/st)는 3b/3c에서 이미 서버가 방출 → 클라 타입만 맞추면 됨.
- **리스크**: WS **send+recv 동시**(split/select) — 드라이런 확정. mpsc 역압/세션 정리. 다중 세션이 같은 슬롯 join 경합 → "이미 점유면 거부"(에러 다운링크 or 무시).

**다음 Plan(4b): 전략 모드 + AI 토글 + 게임 흐름 UI** — 마우스 전략 지시, 런타임 제어 전환, ATTRACT/SELECT/RESULT.
