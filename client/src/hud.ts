// 크롬 HUD 배선(미드나잇 프리시전 콘솔): 서버 스냅샷의 score/time을
// 캔버스 밖 스코어보드·경기시계 DOM에 반영한다. (인캔버스 스코어/시간은 제거)
// KB-55: 참가 상태 배지(사람/AI/YOU) + 내 슬롯 재조정 + 참가 버튼 라벨도 배선.
import type { GameState } from "./net";
import { getMySlot, setMySlot } from "./net";

const scoreBlueEl = document.getElementById("score-blue");
const scoreRedEl = document.getElementById("score-red");
const clockEl = document.getElementById("clock-time");

type Team = "blue" | "red";
// 4대 로스터(KB-57): 0=Blue striker, 1=Blue guard, 2=Red striker, 3=Red guard.
// 사람이 조종 가능한 건 각 팀 striker뿐이라, 팀 참가 상태는 striker 슬롯으로 판단.
const SLOT_INDEX: Record<Team, number> = { blue: 0, red: 2 };
const TEAM_LABEL: Record<Team, string> = { blue: "Blue", red: "Red" };

const ctrlBadgeEls: Record<Team, HTMLElement | null> = {
  blue: document.getElementById("ctrl-blue"),
  red: document.getElementById("ctrl-red"),
};
const joinBtnEls: Record<Team, HTMLElement | null> = {
  blue: document.getElementById("join-blue"),
  red: document.getElementById("join-red"),
};
const joinLabelEls: Record<Team, HTMLElement | null> = {
  blue: document.getElementById("join-blue-label"),
  red: document.getElementById("join-red-label"),
};

/** 서버 ctrl로 내 낙관적 슬롯을 재조정: 서버가 이미 해제(AFK/경합)했는데
 * 클라만 still-mine으로 알고 있으면 null로 되돌린다. */
function reconcileMySlot(ctrl: string[] | undefined): void {
  if (!ctrl) return;
  const mine = getMySlot();
  if (mine === null) return;
  if (ctrl[SLOT_INDEX[mine]] !== "human") {
    setMySlot(null);
  }
}

const lastBadge: Record<Team, string | null> = { blue: null, red: null };

function updateCtrlBadges(ctrl: string[] | undefined): void {
  if (!ctrl) return;
  const mine = getMySlot();
  (["blue", "red"] as Team[]).forEach((team) => {
    const el = ctrlBadgeEls[team];
    if (!el) return;
    const isHuman = ctrl[SLOT_INDEX[team]] === "human";
    const state = !isHuman ? "ai" : mine === team ? "you" : "human";
    if (lastBadge[team] === state) return;
    lastBadge[team] = state;
    el.classList.remove("is-ai", "is-human", "is-you");
    if (state === "ai") {
      el.textContent = "AI";
      el.classList.add("is-ai");
    } else if (state === "you") {
      el.textContent = "YOU";
      el.classList.add("is-you");
    } else {
      el.textContent = "사람";
      el.classList.add("is-human");
    }
  });
}

let lastMineButtons: Team | null | undefined = undefined;

/** 참가 버튼 라벨/강조: 내 팀이면 "나가기"(강조), 아니면 "OO로 참가". */
function updateJoinButtons(): void {
  const mine = getMySlot();
  if (lastMineButtons === mine) return;
  lastMineButtons = mine;
  (["blue", "red"] as Team[]).forEach((team) => {
    const isMine = mine === team;
    const label = joinLabelEls[team];
    if (label) label.textContent = isMine ? `${TEAM_LABEL[team]} 나가기` : `${TEAM_LABEL[team]}로 참가`;
    joinBtnEls[team]?.classList.toggle("mine", isMine);
  });
}

function formatClock(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  const mm = Math.floor(s / 60);
  const ss = s % 60;
  return `${String(mm).padStart(2, "0")}:${String(ss).padStart(2, "0")}`;
}

let lastBlue: number | null = null;
let lastRed: number | null = null;
let lastClock: string | null = null;

/** 매 프레임 최신 렌더 state로 호출. score[0]=Blue, score[1]=Red(서버 world.rs 규약). */
export function updateHud(s: GameState): void {
  const [blue, red] = s.score;
  if (scoreBlueEl && blue !== lastBlue) {
    scoreBlueEl.textContent = String(blue);
    lastBlue = blue;
  }
  if (scoreRedEl && red !== lastRed) {
    scoreRedEl.textContent = String(red);
    lastRed = red;
  }
  const clock = formatClock(s.time);
  if (clockEl && clock !== lastClock) {
    clockEl.textContent = clock;
    lastClock = clock;
  }
  reconcileMySlot(s.ctrl);
  updateCtrlBadges(s.ctrl);
  updateJoinButtons();
}
