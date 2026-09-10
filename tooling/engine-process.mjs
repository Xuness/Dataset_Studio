import { performance } from "node:perf_hooks";

export const engineWaitTimeoutMs = 120000;

export function processExists(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error.code === "ESRCH") return false;
    if (error.code === "EPERM") return true;
    throw error;
  }
}

export async function waitForEngineExit(
  pid,
  {
    timeoutMs = engineWaitTimeoutMs,
    exists = processExists,
    now = () => performance.now(),
    sleep = (ms) => new Promise((done) => setTimeout(done, ms)),
    onWaiting = () => {},
  } = {},
) {
  const started = now();
  let nextNotice = 0;
  while (exists(pid)) {
    const elapsed = now() - started;
    if (elapsed >= timeoutMs)
      throw new Error(
        `旧引擎（PID ${pid}）仍在退出收尾，已等待 ${Math.round(elapsed / 1000)} 秒；请稍后重试启动。`,
      );
    if (elapsed >= nextNotice) {
      onWaiting(elapsed);
      nextNotice = elapsed + 5000;
    }
    await sleep(Math.min(200, timeoutMs - elapsed));
  }
}
