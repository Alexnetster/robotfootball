// 스냅샷 보간(Plan 7a). 30Hz 다운링크 스냅샷 사이를 부드럽게 잇기 위해
// 렌더 전용 시간축(renderClock, 서버 sim초 도메인)을 두고 그 시각을
// 감싸는 두 스냅샷을 선형/각도보간한다. 입력·서버 시뮬레이션에는 영향 없음.
import type { GameState, Robot, Snapshot } from "./net";
import { getSnapshotBuffer } from "./net";

/** 렌더가 실제 최신 스냅샷보다 이만큼(초) 뒤에서 재생 — 지터/역전 완충. */
export const INTERP_DELAY = 0.1;

let renderClock: number | null = null;
let enabled = true;

export function setInterpEnabled(v: boolean): void {
  enabled = v;
}

export function isInterpEnabled(): boolean {
  return enabled;
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

/** 최단호 각도보간: b-a를 -PI..PI로 정규화한 뒤 lerp(한바퀴 도는 것 방지). */
function lerpAngle(a: number, b: number, t: number): number {
  const twoPi = Math.PI * 2;
  let d = (b - a) % twoPi;
  if (d > Math.PI) d -= twoPi;
  if (d < -Math.PI) d += twoPi;
  return a + d * t;
}

function interpolateState(s0: GameState, s1: GameState, alphaRaw: number): GameState {
  const t = Math.max(0, Math.min(1, alphaRaw));
  // 인덱스 기반 매칭(KB-57): 팀당 2대라 r.id가 유일하지 않으므로 id로 매칭하면
  // 같은 팀 로봇이 뭉개진다. 서버는 로봇을 고정 순서로 방출하므로 위치(i)로 매칭.
  const robots: Robot[] = s1.robots.map((r1, i) => {
    const r0 = s0.robots[i];
    if (!r0) return r1; // 개수 변동 등 매칭 불가 시 보간 없이 최신값
    return {
      ...r1, // 이산 필드(parts/down/st/stamina 등)는 최신 스냅샷 값 유지
      pos: { x: lerp(r0.pos.x, r1.pos.x, t), y: lerp(r0.pos.y, r1.pos.y, t) },
      rot: lerpAngle(r0.rot, r1.rot, t),
    };
  });
  return {
    robots,
    ball: { pos: { x: lerp(s0.ball.pos.x, s1.ball.pos.x, t), y: lerp(s0.ball.pos.y, s1.ball.pos.y, t) } },
    score: s1.score,
    time: s1.time,
    ctrl: s1.ctrl, // 이산 필드: 최신 스냅샷 값 그대로(보간 대상 아님).
  };
}

/** 매 프레임 실제 dt(초)만큼 renderClock 전진. 최초 스냅샷 도착 시 초기화하고,
 * 버퍼 범위 [가장오래된.time, 최신.time]로 클램프한다. */
export function advanceRenderClock(dt: number): void {
  const buf = getSnapshotBuffer();
  if (buf.length === 0) return;
  if (renderClock === null) {
    renderClock = buf[0].time - INTERP_DELAY;
  } else {
    renderClock += dt;
  }
  const lo = buf[0].time;
  const hi = buf[buf.length - 1].time;
  if (renderClock < lo) renderClock = lo;
  if (renderClock > hi) renderClock = hi;
}

/** 렌더용 state 계산. 보간 off면 최신 원본 스냅샷을 그대로 반환(기존 동작과 동일).
 * 버퍼가 비어있으면 null. */
export function getRenderState(): GameState | null {
  const buf = getSnapshotBuffer();
  if (buf.length === 0) return null;
  const latest: Snapshot = buf[buf.length - 1];
  if (!enabled || renderClock === null || buf.length === 1) return latest.state;

  // renderClock을 감싸는 [s0,s1] 탐색. 못 찾으면(=starve, 최신 초과) 최신에 클램프.
  let idx = buf.length - 1;
  for (let i = 1; i < buf.length; i++) {
    if (buf[i].time >= renderClock) {
      idx = i;
      break;
    }
  }
  const s1 = buf[idx];
  const s0 = buf[Math.max(0, idx - 1)];
  if (s0 === s1) return s1.state;
  const span = s1.time - s0.time;
  const alpha = span > 0 ? (renderClock - s0.time) / span : 1;
  return interpolateState(s0.state, s1.state, alpha);
}
