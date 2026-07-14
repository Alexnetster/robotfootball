import { getMySlot, send, setMySlot } from "./net";

/** 현재 눌린 키 상태. (01-UX §3: ↑↓ 이동, ←→ 회전, Space=차기(KB-48) — 모드리스 탭) */
const keys = { up: false, down: false, left: false, right: false, shift: false, space: false };

type InputState = { fwd: boolean; back: boolean; turn: -1 | 0 | 1; run: boolean; kick: boolean };
let lastSent: InputState | null = null;

function computeInput(): InputState {
  // turn: ←=+1(좌회전), →=-1(우회전). 둘 다 눌리면 상쇄(0).
  const turn: -1 | 0 | 1 = keys.left === keys.right ? 0 : keys.left ? 1 : -1;
  return { fwd: keys.up, back: keys.down, turn, run: keys.shift, kick: keys.space };
}

function sendIfChanged(): void {
  const cur = computeInput();
  if (
    lastSent === null ||
    cur.fwd !== lastSent.fwd ||
    cur.back !== lastSent.back ||
    cur.turn !== lastSent.turn ||
    cur.run !== lastSent.run ||
    cur.kick !== lastSent.kick
  ) {
    lastSent = cur;
    send({
      t: "input",
      fwd: cur.fwd,
      back: cur.back,
      turn: cur.turn,
      run: cur.run,
      kick: cur.kick,
    });
  }
}

function handleKey(e: KeyboardEvent, pressed: boolean): void {
  switch (e.key) {
    case "ArrowUp":
      keys.up = pressed;
      break;
    case "ArrowDown":
      keys.down = pressed;
      break;
    case "ArrowLeft":
      keys.left = pressed;
      break;
    case "ArrowRight":
      keys.right = pressed;
      break;
    case "Shift":
      keys.shift = pressed;
      break;
    case " ":
    case "Spacebar": // 구형 브라우저 호환
      keys.space = pressed;
      // Space의 기본 동작(페이지 스크롤)을 막는다.
      e.preventDefault();
      break;
    default:
      return;
  }
  sendIfChanged();
}

/** 키보드 캡처 시작: keydown/keyup에서 변화 시에만 input 업링크 송신. */
export function initInput(): void {
  window.addEventListener("keydown", (e) => handleKey(e, true));
  window.addEventListener("keyup", (e) => handleKey(e, false));
}

/** 참가 버튼 클릭 → 토글(KB-55): 내 팀이면 leave, 다른 팀/미참가면 join. */
function bindJoinButton(btnId: string, slot: "blue" | "red"): void {
  document.getElementById(btnId)?.addEventListener("click", () => {
    if (getMySlot() === slot) {
      send({ t: "leave" });
      setMySlot(null);
    } else {
      send({ t: "join", slot });
      setMySlot(slot);
    }
  });
}

/** [BLUE]/[RED] 참가 버튼 배선: 토글(참가/전환/나가기). */
export function initJoinButtons(blueBtnId: string, redBtnId: string): void {
  bindJoinButton(blueBtnId, "blue");
  bindJoinButton(redBtnId, "red");
}
