// Memory guards for run.mjs: Chrome's process-tree RSS (sampled, with an abort limit) and this session's cgroup usage.
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";

// RSS in MB of `root` and every descendant (Linux `ps`; null elsewhere)
export function treeRssMB(root) {
  let out;
  try {
    out = execFileSync("ps", ["-e", "-o", "pid=,ppid=,rss="], { encoding: "utf8" });
  } catch {
    return null;
  }
  const kids = new Map();
  const rss = new Map();
  for (const line of out.trim().split("\n")) {
    const [pid, ppid, kb] = line.trim().split(/\s+/).map(Number);
    rss.set(pid, kb);
    if (!kids.has(ppid)) kids.set(ppid, []);
    kids.get(ppid).push(pid);
  }
  let kb = 0;
  for (const stack = [root]; stack.length; ) {
    const p = stack.pop();
    kb += rss.get(p) ?? 0;
    stack.push(...(kids.get(p) ?? []));
  }
  return Math.round(kb / 1024);
}

// Samples the tree every second; calls onLimit once when it passes limitMB. stop() returns the peak.
export function watchRss(root, limitMB, onLimit) {
  let peak = 0;
  let fired = false;
  const timer = setInterval(() => {
    const mb = treeRssMB(root) ?? 0;
    peak = Math.max(peak, mb);
    if (mb > limitMB && !fired) {
      fired = true;
      onLimit(mb);
    }
  }, 1000);
  return () => (clearInterval(timer), peak);
}

// memory.current of the cgroup this process runs in, MB (null if not cgroup v2)
export function cgroupMB() {
  try {
    const path = readFileSync("/proc/self/cgroup", "utf8").split("\n").find((l) => l.startsWith("0::"))?.slice(3);
    return Math.round(Number(readFileSync(`/sys/fs/cgroup${path}/memory.current`, "utf8")) / 2 ** 20);
  } catch {
    return null;
  }
}
