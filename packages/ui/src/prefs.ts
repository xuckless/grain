// The singer's app-wide preferences, kept in this browser or app's local storage. Storage can be
// missing or blocked (private windows, cleared site data): every read falls back to the default.

const CALIBRATE_FIRST = "grain.calibrateBeforeRecording";

/** Settings → Calibrate before each recording (default on). */
export function loadCalibrateFirst(): boolean {
  try {
    return localStorage.getItem(CALIBRATE_FIRST) !== "off";
  } catch {
    return true;
  }
}

export function saveCalibrateFirst(on: boolean): void {
  try {
    localStorage.setItem(CALIBRATE_FIRST, on ? "on" : "off");
  } catch {
    // Not stored: the choice holds until the song closes.
  }
}
