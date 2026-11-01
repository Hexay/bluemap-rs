  // Paced delivery (docs/18), off by default: the first TUNING.pace tiles of a frame go straight on, later ones wait for
  // the next frame. rAF stops in a hidden tab, hence the timer.
  const PACE_TIMEOUT_MS = 100;
  const waiting = [];
  let deliveredInFrame = 0;
  let paceArmed = false;

  function armPace() {
    if (paceArmed) return;
    paceArmed = true;
    let fired = false;
    const once = () => {
      if (fired) return;
      fired = true;
      paceArmed = false;
      deliveredInFrame = 0;
      for (const release of waiting.splice(0, TUNING.pace)) {
        deliveredInFrame++;
        release();
      }
      if (waiting.length || deliveredInFrame) armPace();
    };
    if (typeof scope.requestAnimationFrame === "function") scope.requestAnimationFrame(once);
    setTimeout(once, PACE_TIMEOUT_MS);
  }

  const paced = () => {
    if (!TUNING.pace) return null;
    armPace();
    if (deliveredInFrame < TUNING.pace && !waiting.length) {
      deliveredInFrame++;
      return null;
    }
    return new Promise((release) => waiting.push(release));
  };
