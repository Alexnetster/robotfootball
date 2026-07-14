export type Vec2 = { x: number; y: number };
/** 파손 다운 상태(스냅샷 디버프). repair_in = 리페어까지 남은 초. */
export type Down = { broken: boolean; repair_in: number };
export type Robot = {
  id: "Blue" | "Red";
  pos: Vec2;
  rot: number;
  /** 부위별 (부위명, HP비율 0..1). 3b부터 서버가 방출. */
  parts?: [string, number][];
  down?: Down;
  /** 상태이상 태그: "downed" | "stun" 등. */
  st?: string[];
  /** 스태미나 비율 0..1(KB-45). */
  stamina?: number;
  /** 로드아웃/프리셋 id("striker"|"guard" 등). 렌더 외형 구분에 사용(KB-56). */
  robot?: string;
};
export type Ball = { pos: Vec2 };
/** 슬롯별 조종 주체(KB-55): index 0=Blue, 1=Red, 값 "human"|"ai". */
export type GameState = {
  robots: Robot[];
  ball: Ball;
  score: [number, number];
  time: number;
  ctrl?: string[];
};

/** 보간용 스냅샷 버퍼 항목. time=서버 sim초(다운링크 state.time). */
export type Snapshot = { time: number; state: GameState };

let socket: WebSocket | null = null;

// ── 보간용 스냅샷 버퍼(Plan 7a) ───────────────────────────────────────
// 오름차순(time) 유지, 길이 상한 도달 시 가장 오래된 것부터 제거.
const MAX_BUFFER = 20;
const buffer: Snapshot[] = [];

function pushSnapshot(state: GameState): void {
  buffer.push({ time: state.time, state });
  // netsim(delay/jitter)으로 도착 순서가 뒤섞일 수 있어 time 기준 재정렬.
  buffer.sort((a, b) => a.time - b.time);
  while (buffer.length > MAX_BUFFER) buffer.shift();
}

/** 보간 모듈이 읽기 전용으로 참조하는 스냅샷 버퍼(오름차순). */
export function getSnapshotBuffer(): readonly Snapshot[] {
  return buffer;
}

// ── ping/RTT(Plan 7a) ────────────────────────────────────────────────
let nextPingId = 1;
const pingSentAt = new Map<number, number>();
let rtt: number | null = null;

function now(): number {
  return typeof performance !== "undefined" ? performance.now() : Date.now();
}

/** {t:"ping",id} 송신 + 송신시각 기록(RTT 계산용). */
export function sendPing(): void {
  const id = nextPingId++;
  pingSentAt.set(id, now());
  send({ t: "ping", id });
}

/** 최근 계산된 RTT(ms). pong을 아직 못 받았으면 null. */
export function getRtt(): number | null {
  return rtt;
}

/** 개발 패널에서 netsim 파라미터 변경 시 서버로 송신. */
export function sendNetsim(delay_ms: number, jitter_ms: number, drop_pct: number): void {
  send({ t: "netsim", delay_ms, jitter_ms, drop_pct });
}

export function connect(onState: (s: GameState) => void): void {
  // 127.0.0.1 고정: 이 머신에서 `localhost`는 IPv6(::1)로 다른 서비스에 갈 수 있어
  // IPv4 0.0.0.0 바인드 서버에 안 닿음. (LAN/폰은 추후 PUBLIC_URL 설정으로)
  const ws = new WebSocket("ws://127.0.0.1:8090/ws");
  socket = ws;
  ws.onopen = () => console.log("[ws] connected");
  ws.onerror = (e) => console.error("[ws] error", e);
  ws.onclose = (e) => console.warn("[ws] closed", e.code, e.reason);
  ws.onmessage = (e) => {
    const msg = JSON.parse(e.data);
    if (msg.t === "state") {
      const state = msg.state as GameState;
      pushSnapshot(state);
      onState(state);
    } else if (msg.t === "pong") {
      const id = msg.id as number;
      const sentAt = pingSentAt.get(id);
      if (sentAt !== undefined) {
        rtt = now() - sentAt;
        pingSentAt.delete(id);
      }
    }
  };
}

/** 업링크 송신(join/input/leave/ping/netsim 등). 연결이 열려있지 않으면 조용히 무시. */
export function send(msg: Record<string, unknown>): void {
  if (socket && socket.readyState === WebSocket.OPEN) {
    socket.send(JSON.stringify(msg));
  }
}

// ── 내 슬롯 낙관적 추적(KB-55) ────────────────────────────────────────
// 세션 식별자를 두지 않는 대신(YAGNI), 클라가 자기 join/leave를 낙관적으로
// 기억하고 매 스냅샷의 ctrl로 재조정한다(hud.ts). 데모 스코프: 로컬/소수
// 플레이어 가정, 다중 탭 등은 한계로 허용.
let mySlot: "blue" | "red" | null = null;

export function getMySlot(): "blue" | "red" | null {
  return mySlot;
}

export function setMySlot(slot: "blue" | "red" | null): void {
  mySlot = slot;
}
