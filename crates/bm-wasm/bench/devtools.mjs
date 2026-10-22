// The page target's DevTools socket URL of a Chrome started with --remote-debugging-port=0 --user-data-dir=<profile>:
// the port it picked is in <profile>/DevToolsActivePort, so parallel runs never share one.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";

export async function pageSocket(profile) {
  for (let attempt = 0; attempt < 150; attempt++) {
    try {
      const port = readFileSync(join(profile, "DevToolsActivePort"), "utf8").split("\n")[0].trim();
      const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const page = targets.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // not up yet
    }
    await sleep(200);
  }
  throw new Error("the browser's DevTools port never opened");
}
