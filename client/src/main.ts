import { connect } from "./net";
import { render } from "./render";
import { initInput, initJoinButtons } from "./input";
import { initDevPanel } from "./devpanel";
import { advanceRenderClock, getRenderState } from "./interp";
import { updateHud } from "./hud";

const ctx = (document.getElementById("c") as HTMLCanvasElement).getContext("2d")!;
// 원본 스냅샷은 net.ts 내부 버퍼에 쌓이고 렌더는 interp.ts를 통해 읽는다
// (보간 off 시에도 항상 최신 스냅샷을 반환하므로 기존 렌더 동작과 동일).
connect(() => {});
initInput();
initJoinButtons("join-blue", "join-red");
initDevPanel();

let lastFrameT = 0;
function frame(now: number) {
  const dt = lastFrameT === 0 ? 0 : Math.min(0.1, (now - lastFrameT) / 1000);
  lastFrameT = now;
  advanceRenderClock(dt);
  const s = getRenderState();
  if (s) {
    render(ctx, s);
    updateHud(s);
  }
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
