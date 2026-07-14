import type { GameState, Robot } from "./net";
const FIELD_W = 12, FIELD_H = 8, GOAL_W = 2.4;

// ── Neon Telemetry Arena 팔레트(ADR-014) ──────────────────────────────
const COL = {
  pitch: "#0d1220",
  grid: "#1a2130",
  line: "#324055",
  blue: "#3AA0FF",
  red: "#FF5A6A",
  amber: "#FFB020",
  green: "#3DDC97",
  blueBody: "#12233b",
  redBody: "#3b1219",
  blueEye: "#bcd8ff",
  redEye: "#ffc4cb",
  hpBg: "#0d1220",
  hpBorder: "#242c3a",
} as const;

/** "#rrggbb" + 알파 → rgba() 문자열. */
function rgba(hex: string, a: number): string {
  const n = parseInt(hex.slice(1), 16);
  const r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
  return `rgba(${r},${g},${b},${a})`;
}

/** roundRect 폴리필(구형 환경 대비). */
function roundRectPath(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number): void {
  if (ctx.roundRect) {
    ctx.beginPath();
    ctx.roundRect(x, y, w, h, r);
    return;
  }
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

// ── 다리/보행(KB-47, 클라 전용 비주얼) ────────────────────────────────
// 이동 물리는 서버에서 몸체속도로 추상화돼 있고, 다리는 순수 렌더 연출이다.
// 4족=트로트(대각선 쌍 교대), 6족=삼각보행(tripod, 좌우 교대 3각).
type Chassis = "quad" | "hex";
type Leg = { x: number; y: number; ph: number }; // 로컬(+x=전방, y=측면), ph=위상오프셋
const QUAD: Leg[] = [
  { x: 6, y: 7, ph: 0 }, { x: 6, y: -7, ph: Math.PI },
  { x: -6, y: 7, ph: Math.PI }, { x: -6, y: -7, ph: 0 },
];
const HEX: Leg[] = [
  { x: 7, y: 7, ph: 0 }, { x: 7, y: -7, ph: Math.PI },
  { x: 0, y: 7, ph: Math.PI }, { x: 0, y: -7, ph: 0 },
  { x: -7, y: 7, ph: 0 }, { x: -7, y: -7, ph: Math.PI },
];

// 섀시 = 6족(거미형)으로 확정(KB-47 후속). 4족 렌더 코드(QUAD)는 향후
// 로드아웃/섀시 파츠로 개별화할 때를 위해 남겨둔다.
function chassisFor(_r: Robot): Chassis {
  return "hex";
}

// 보행 위상 상태(로봇별). rAF 프레임마다 실시간 dt × 평활 속도로 위상 전진 →
// 30Hz 스냅샷 사이에도 다리가 끊기지 않고 부드럽게 움직인다.
type Gait = { phase: number; px: number; py: number; spd: number };
const gait = new Map<string, Gait>();
let lastT = 0;

const GAIT_FREQ = 2.6; // 보행 주파수(rad / 이동 m)

/** 로봇 몸체+다리+발자국 텔레메트리 링을 그린다(로컬 원점=로봇 중심, +x=전방). */
function drawRobotBody(ctx: CanvasRenderingContext2D, r: Robot, phase: number, scale: number): void {
  const chassis = chassisFor(r);
  const legs = chassis === "hex" ? HEX : QUAD;
  const reach = (chassis === "hex" ? 13 : 11) * scale;
  const swing = 5 * scale;
  const isBlue = r.id === "Blue";
  const teamCol = isBlue ? COL.blue : COL.red;
  const bodyFill = isBlue ? COL.blueBody : COL.redBody;
  const eyeCol = isBlue ? COL.blueEye : COL.redEye;

  // 타입별 외형(KB-56): 방어형(guard)=통통(폭↑·두꺼운 다리, 안 밀리는 느낌),
  // 공격형(striker)=얇고 길쭉·가는 다리(스피디한 느낌). 미지 프리셋=기본.
  const preset = r.robot ?? "";
  const blf = preset === "striker" ? 1.14 : 1.0; // 반길이 배수
  const bwf = preset === "guard" ? 1.42 : preset === "striker" ? 0.66 : 1.0; // 반폭 배수
  const legW = preset === "guard" ? 2.9 : preset === "striker" ? 1.7 : 2.2; // 다리 굵기
  const bl = 11 * scale * blf, bw = 8 * scale * bwf; // 몸통 반길이/반폭

  // 팀색 헤일로(뒤쪽, 은은하게)
  ctx.save();
  ctx.globalAlpha = 0.22;
  ctx.fillStyle = teamCol;
  ctx.beginPath();
  ctx.ellipse(0, 0, bl * 2.1, bw * 2.4, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();

  // 다리(몸통 뒤에 먼저 그림) + 발자국 임팩트 링(접지 위상에 연동)
  ctx.lineCap = "round";
  for (const l of legs.map((l) => ({ x: l.x * scale, y: l.y * scale, ph: l.ph }))) {
    const side = l.y > 0 ? 1 : -1;
    const footX = l.x + Math.sin(phase + l.ph) * swing;
    const footY = l.y + side * reach;
    const kneeX = (l.x + footX) / 2;
    const kneeY = (l.y + footY) / 2 + side * 2 * scale; // 바깥으로 살짝 꺾인 무릎

    // 접지 위상(0..1, 1=완전 접지) → 저알파 링, 페이드 인/아웃.
    const contact = Math.max(0, Math.sin(phase + l.ph));
    if (contact > 0.05) {
      ctx.save();
      ctx.strokeStyle = teamCol;
      ctx.globalAlpha = contact * 0.28;
      ctx.lineWidth = 1.4;
      ctx.beginPath();
      ctx.arc(footX, footY, (6 + contact * 4) * scale, 0, Math.PI * 2);
      ctx.stroke();
      ctx.restore();
    }

    ctx.strokeStyle = teamCol;
    ctx.globalAlpha = 0.85;
    ctx.lineWidth = legW * scale;
    ctx.beginPath();
    ctx.moveTo(l.x, l.y);
    ctx.lineTo(kneeX, kneeY);
    ctx.lineTo(footX, footY);
    ctx.stroke();
    ctx.fillStyle = teamCol;
    ctx.beginPath(); ctx.arc(footX, footY, 1.6 * scale, 0, Math.PI * 2); ctx.fill();
  }
  ctx.globalAlpha = 1.0;

  // 몸통(둥근 사각, 팀틴트 채움 + 팀색 아웃라인)
  ctx.fillStyle = bodyFill;
  ctx.strokeStyle = teamCol;
  ctx.lineWidth = 2.5;
  roundRectPath(ctx, -bl, -bw, bl * 2, bw * 2, 4.5 * scale);
  ctx.fill();
  ctx.stroke();

  // 전방 표시("눈")
  ctx.fillStyle = eyeCol;
  ctx.beginPath(); ctx.arc(bl - 4 * scale, 0, 2.6 * scale, 0, Math.PI * 2); ctx.fill();
}

/** HP/스태미나 캡슐(6px 캡슐, 상태 3단 색). 로봇 머리 위, 화면 기준(회전 없음). */
function drawCapsules(ctx: CanvasRenderingContext2D, cx: number, cy: number, r: Robot): void {
  const w = 34, h = 6;
  let y = cy - 26;

  if (r.parts && r.parts.length > 0) {
    const minHp = Math.min(...r.parts.map(([, hp]) => hp));
    const fillCol = minHp > 0.5 ? COL.green : minHp > 0.2 ? COL.amber : COL.red;
    ctx.fillStyle = COL.hpBg;
    ctx.strokeStyle = COL.hpBorder;
    ctx.lineWidth = 1;
    roundRectPath(ctx, cx - w / 2, y, w, h, h / 2);
    ctx.fill(); ctx.stroke();
    const fw = Math.max(0, Math.min(1, minHp)) * (w - 2);
    if (fw > 0.5) {
      ctx.fillStyle = fillCol;
      roundRectPath(ctx, cx - w / 2 + 1, y + 1, fw, h - 2, (h - 2) / 2);
      ctx.fill();
    }
    y += h + 3;
  }

  if (r.stamina !== undefined) {
    const sh = 4;
    ctx.fillStyle = COL.hpBg;
    ctx.strokeStyle = COL.hpBorder;
    ctx.lineWidth = 1;
    roundRectPath(ctx, cx - w / 2, y, w, sh, sh / 2);
    ctx.fill(); ctx.stroke();
    const fw = Math.max(0, Math.min(1, r.stamina)) * (w - 2);
    if (fw > 0.5) {
      ctx.fillStyle = COL.blue;
      roundRectPath(ctx, cx - w / 2 + 1, y + 1, fw, sh - 2, (sh - 2) / 2);
      ctx.fill();
    }
  }
}

/** 팀색 글로우 프레임 골대. */
function drawGoal(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number, color: string): void {
  ctx.save();
  ctx.shadowColor = color;
  ctx.shadowBlur = 7;
  ctx.strokeStyle = color;
  ctx.globalAlpha = 0.35;
  ctx.lineWidth = 3;
  roundRectPath(ctx, x, y, w, h, r);
  ctx.stroke();
  ctx.shadowBlur = 0;
  ctx.globalAlpha = 0.9;
  roundRectPath(ctx, x, y, w, h, r);
  ctx.stroke();
  ctx.restore();
}

// 공 트레일(짧게, 프레임 간 위치 이력).
type TrailPt = { x: number; y: number };
const ballTrail: TrailPt[] = [];
const BALL_TRAIL_LEN = 7;

export function render(ctx: CanvasRenderingContext2D, s: GameState): void {
  const { width, height } = ctx.canvas;
  const sx = width / FIELD_W, sy = height / FIELD_H;
  const tx = (x: number) => width / 2 + x * sx;
  const ty = (y: number) => height / 2 - y * sy;
  // 로봇 아트는 원래 720×480(60px/m) 캔버스 기준으로 튜닝됐다 — 캔버스 크기가
  // 달라져도 필드 대비 비율이 유지되도록 스케일 보정.
  const robotScale = sx / 60;
  // 정적 필드 아트(그리드/골대/센터서클)는 시안의 1200×800(100px/m) 뷰박스 기준.
  const k = width / 1200;

  // 프레임 dt(초). 탭 복귀 등 큰 점프는 클램프.
  const now = (typeof performance !== "undefined" ? performance.now() : Date.now());
  const dt = lastT === 0 ? 0 : Math.min(0.1, (now - lastT) / 1000);
  lastT = now;

  ctx.clearRect(0, 0, width, height);

  // 필드 배경
  ctx.fillStyle = COL.pitch;
  ctx.fillRect(0, 0, width, height);

  // 좌/우 팀 진영 그라데이션(팀색 ~14%→0)
  const gB = ctx.createLinearGradient(0, 0, width / 2, 0);
  gB.addColorStop(0, rgba(COL.blue, 0.14));
  gB.addColorStop(1, rgba(COL.blue, 0));
  ctx.fillStyle = gB;
  ctx.fillRect(0, 0, width / 2, height);
  const gR = ctx.createLinearGradient(width, 0, width / 2, 0);
  gR.addColorStop(0, rgba(COL.red, 0.14));
  gR.addColorStop(1, rgba(COL.red, 0));
  ctx.fillStyle = gR;
  ctx.fillRect(width / 2, 0, width / 2, height);

  // 1m 그리드
  ctx.strokeStyle = COL.grid;
  ctx.lineWidth = 1;
  for (let i = 1; i < FIELD_W; i++) {
    const gx = i * sx;
    ctx.beginPath(); ctx.moveTo(gx, 0); ctx.lineTo(gx, height); ctx.stroke();
  }
  for (let j = 1; j < FIELD_H; j++) {
    const gy = j * sy;
    ctx.beginPath(); ctx.moveTo(0, gy); ctx.lineTo(width, gy); ctx.stroke();
  }

  // 필드 경계 = 팔각형(코너 챔퍼 반영, KB-54). 서버 챔퍼(1m)로 잘린 4코너를 '벽'으로
  // 채워 "여기는 더 못 들어감"을 눈에 보이게 표시(충돌 지오메트리와 화면 일치).
  {
    const CH = 1.0, hw = FIELD_W / 2, hh = FIELD_H / 2;
    const cxx = hw - CH, cyy = hh - CH;
    const oct: [number, number][] = [
      [-cxx, hh], [cxx, hh], [hw, cyy], [hw, -cyy],
      [cxx, -hh], [-cxx, -hh], [-hw, -cyy], [-hw, cyy],
    ];
    const corners: [number, number][][] = [
      [[cxx, hh], [hw, hh], [hw, cyy]],
      [[-cxx, hh], [-hw, hh], [-hw, cyy]],
      [[cxx, -hh], [hw, -hh], [hw, -cyy]],
      [[-cxx, -hh], [-hw, -hh], [-hw, -cyy]],
    ];
    // 잘린 코너 삼각형 = 벽(슬레이트 채움).
    ctx.fillStyle = rgba(COL.line, 0.38);
    for (const tri of corners) {
      ctx.beginPath();
      ctx.moveTo(tx(tri[0][0]), ty(tri[0][1]));
      ctx.lineTo(tx(tri[1][0]), ty(tri[1][1]));
      ctx.lineTo(tx(tri[2][0]), ty(tri[2][1]));
      ctx.closePath();
      ctx.fill();
    }
    // 팔각 경계선(챔퍼 대각선이 곧 no-go 벽면).
    ctx.strokeStyle = COL.line;
    ctx.lineWidth = Math.max(1, 2 * k);
    ctx.beginPath();
    oct.forEach(([x, y], idx) => {
      const px = tx(x), py = ty(y);
      if (idx === 0) ctx.moveTo(px, py); else ctx.lineTo(px, py);
    });
    ctx.closePath();
    ctx.stroke();
  }
  // 중앙선 + 센터서클
  ctx.strokeStyle = COL.line;
  ctx.lineWidth = Math.max(1, 2 * k);
  ctx.beginPath(); ctx.moveTo(width / 2, 16 * k); ctx.lineTo(width / 2, height - 16 * k); ctx.stroke();
  ctx.beginPath(); ctx.arc(width / 2, height / 2, 92 * k, 0, Math.PI * 2); ctx.stroke();
  ctx.fillStyle = COL.line;
  ctx.beginPath(); ctx.arc(width / 2, height / 2, 4 * k, 0, Math.PI * 2); ctx.fill();

  // 골대(팀색 글로우 프레임)
  drawGoal(ctx, 6 * k, ty(GOAL_W / 2), 26 * k, GOAL_W * sy, 4 * k, COL.blue);
  drawGoal(ctx, width - 32 * k, ty(GOAL_W / 2), 26 * k, GOAL_W * sy, 4 * k, COL.red);

  // 로봇
  for (const r of s.robots) {
    const downed = r.st?.includes("downed") ?? false;
    const stunned = r.st?.includes("stun") ?? false;

    // 보행 위상 전진: 스냅샷 위치 변화로 순간속도 추정 → EMA 평활 → dt로 전진.
    // 4대 로스터(KB-57): 팀당 2대라 r.id가 유일하지 않으므로 팀+프리셋으로 키잉
    // (Blue-striker/Blue-guard/Red-striker/Red-guard는 서로 유일).
    const gkey = `${r.id}:${r.robot ?? ""}`;
    const g = gait.get(gkey) ?? { phase: 0, px: r.pos.x, py: r.pos.y, spd: 0 };
    const d = Math.hypot(r.pos.x - g.px, r.pos.y - g.py);
    const inst = dt > 0 ? d / dt : 0;
    g.spd += (inst - g.spd) * Math.min(1, dt * 8);
    g.px = r.pos.x; g.py = r.pos.y;
    g.phase += g.spd * dt * GAIT_FREQ;
    gait.set(gkey, g);

    const cx = tx(r.pos.x), cy = ty(r.pos.y);

    ctx.save();
    ctx.translate(cx, cy);
    ctx.rotate(-r.rot);
    // 미세 몸통 흔들림(걸을 때만): 측면으로 살짝 sway.
    ctx.translate(0, Math.sin(g.phase * 2) * Math.min(1, g.spd) * 1.2 * robotScale);
    ctx.globalAlpha = downed ? 0.4 : 1.0; // 파손 다운 시 흐리게
    drawRobotBody(ctx, r, g.phase, robotScale);
    ctx.restore();
    ctx.globalAlpha = 1.0;

    // STUN: 머리 위 앰버 점선 링
    if (stunned) {
      ctx.save();
      ctx.strokeStyle = COL.amber;
      ctx.globalAlpha = 0.9;
      ctx.lineWidth = 2.2;
      ctx.setLineDash([5, 6]);
      ctx.beginPath();
      ctx.arc(cx, cy, 20 * robotScale, 0, Math.PI * 2);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.restore();
    }

    // DOWN: 반투명(이미 적용) + X 배지
    if (downed) {
      ctx.save();
      ctx.fillStyle = "rgba(0,0,0,0.55)";
      ctx.strokeStyle = COL.amber;
      ctx.lineWidth = 1.5;
      ctx.beginPath(); ctx.arc(cx, cy, 10, 0, Math.PI * 2); ctx.fill(); ctx.stroke();
      ctx.strokeStyle = COL.amber;
      ctx.lineWidth = 2;
      ctx.beginPath();
      ctx.moveTo(cx - 4, cy - 4); ctx.lineTo(cx + 4, cy + 4);
      ctx.moveTo(cx + 4, cy - 4); ctx.lineTo(cx - 4, cy + 4);
      ctx.stroke();
      ctx.restore();
    }

    // 상태 라벨(다운/스턴) — 앰버 등폭 텍스트
    if (downed || stunned) {
      ctx.fillStyle = COL.amber;
      ctx.font = "700 11px " + '"SF Mono","JetBrains Mono",Menlo,Consolas,monospace';
      ctx.textAlign = "center";
      ctx.fillText(downed ? "DOWN" : "STUN", cx, cy - 34);
      ctx.textAlign = "left";
    }

    // HP/스태미나 캡슐
    drawCapsules(ctx, cx, cy, r);
  }

  // 공 + 트레일
  const bx = tx(s.ball.pos.x), by = ty(s.ball.pos.y);
  ballTrail.push({ x: bx, y: by });
  while (ballTrail.length > BALL_TRAIL_LEN) ballTrail.shift();
  for (let i = 0; i < ballTrail.length - 1; i++) {
    const p = ballTrail[i];
    const a = ((i + 1) / ballTrail.length) * 0.35;
    const rr = 3 + (i / ballTrail.length) * 4;
    ctx.fillStyle = rgba("#ffffff", a);
    ctx.beginPath(); ctx.arc(p.x, p.y, rr, 0, Math.PI * 2); ctx.fill();
  }
  // 공: 난전에서도 확실히 보이도록 글로우 + 앰버 외곽 링 + 흰 코어.
  ctx.save();
  ctx.shadowColor = "#fff";
  ctx.shadowBlur = 12;
  ctx.fillStyle = "#fff";
  ctx.beginPath(); ctx.arc(bx, by, 8.5, 0, Math.PI * 2); ctx.fill();
  ctx.shadowBlur = 0;
  ctx.strokeStyle = COL.amber;
  ctx.lineWidth = 2;
  ctx.beginPath(); ctx.arc(bx, by, 11, 0, Math.PI * 2); ctx.stroke();
  ctx.restore();

  // 인캔버스 스코어/시간 없음 — 크롬 HUD(hud.ts)에서 담당.
}
