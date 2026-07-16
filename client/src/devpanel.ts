// 개발용 넷코드 패널(Plan 7a): netsim(delay/jitter/drop) 조절, 보간 on/off,
// RTT 표시. index.html의 Link Monitor 카드(Neon Telemetry Arena 콘솔) 마크업과 짝을 이룬다.
import { sendNetsim, sendPing, getRtt } from "./net";
import { setInterpEnabled } from "./interp";
import { setShowVelocityVectors, setShowAiPredictions } from "./render";

const PING_INTERVAL_MS = 1000;
const RTT_REFRESH_MS = 250;

// RTT 임계(ms): 이 아래는 good(green), 이 아래는 warn(amber), 그 이상은 bad(red).
const RTT_GOOD_MAX = 60;
const RTT_WARN_MAX = 150;

type RangeField = { input: HTMLInputElement; num: HTMLElement | null; unit: string };

function updateRangeVisual(f: RangeField): void {
  const min = Number(f.input.min) || 0;
  const max = Number(f.input.max) || 100;
  const val = Number(f.input.value) || 0;
  const pct = max > min ? ((val - min) / (max - min)) * 100 : 0;
  f.input.style.background = `linear-gradient(to right, var(--blue) ${pct}%, var(--panel-2) ${pct}%)`;
  if (f.num) f.num.textContent = `${val} ${f.unit}`;
}

export function initDevPanel(): void {
  const delayInput = document.getElementById("netsim-delay") as HTMLInputElement | null;
  const jitterInput = document.getElementById("netsim-jitter") as HTMLInputElement | null;
  const dropInput = document.getElementById("netsim-drop") as HTMLInputElement | null;
  const interpToggle = document.getElementById("interp-toggle") as HTMLInputElement | null;
  const velVectorToggle = document.getElementById("vel-vector-toggle") as HTMLInputElement | null;
  const aiPredictToggle = document.getElementById("ai-predict-toggle") as HTMLInputElement | null;
  const rttEl = document.getElementById("rtt-value");

  const fields: RangeField[] = [];
  if (delayInput) fields.push({ input: delayInput, num: document.getElementById("netsim-delay-num"), unit: "ms" });
  if (jitterInput) fields.push({ input: jitterInput, num: document.getElementById("netsim-jitter-num"), unit: "ms" });
  if (dropInput) fields.push({ input: dropInput, num: document.getElementById("netsim-drop-num"), unit: "%" });

  function sendCurrentNetsim(): void {
    const delay_ms = Number(delayInput?.value) || 0;
    const jitter_ms = Number(jitterInput?.value) || 0;
    const drop_pct = Number(dropInput?.value) || 0;
    sendNetsim(delay_ms, jitter_ms, drop_pct);
  }

  for (const f of fields) {
    updateRangeVisual(f); // 초기 상태(0)를 트랙 채움/숫자에 반영
    f.input.addEventListener("input", () => {
      updateRangeVisual(f);
      sendCurrentNetsim();
    });
  }

  if (interpToggle) {
    setInterpEnabled(interpToggle.checked);
    interpToggle.addEventListener("change", () => setInterpEnabled(interpToggle.checked));
  }

  if (velVectorToggle) {
    setShowVelocityVectors(velVectorToggle.checked);
    velVectorToggle.addEventListener("change", () => setShowVelocityVectors(velVectorToggle.checked));
  }

  if (aiPredictToggle) {
    setShowAiPredictions(aiPredictToggle.checked);
    aiPredictToggle.addEventListener("change", () => setShowAiPredictions(aiPredictToggle.checked));
  }

  setInterval(() => sendPing(), PING_INTERVAL_MS);
  setInterval(() => {
    if (!rttEl) return;
    const rtt = getRtt();
    rttEl.textContent = rtt === null ? "--" : String(Math.round(rtt));
    rttEl.className =
      "val " + (rtt === null ? "unknown" : rtt <= RTT_GOOD_MAX ? "good" : rtt <= RTT_WARN_MAX ? "warn" : "bad");
  }, RTT_REFRESH_MS);
}
