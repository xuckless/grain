// Recording takes over the song, shared by the Score view (one take) and Karaoke (one take per
// run): the backing, the microphone, and handing the scored take to the session. The state
// machines that drive these live in takes.ts and karaoke.ts.

import type { EngineHost, PlayerCommand, Project, ScoreSettings, TakeAdded } from "@workstation/engine-api";
import { errorMessage, type Session } from "./state";
import { backingCommands, backingRestoreCommands, defaultBacking, type Backing } from "./takes";

/** Sends commands in order. The engine rejects illegal ones; the transport poll shows the truth. */
export async function runCommands(host: EngineHost, commands: PlayerCommand[]): Promise<void> {
  for (const command of commands) {
    try {
      await host.send(command);
    } catch (e) {
      console.warn(errorMessage(e));
    }
  }
}

function microphone(host: EngineHost) {
  const mic = host.microphone;
  if (mic.kind !== "available") throw new Error(mic.reason);
  return mic;
}

/** The backing a take records over: the singer's pick, else the song's default. */
export function effectiveBacking(session: Session): Backing {
  return session.backing ?? defaultBacking(session.project.analysis.accompaniment);
}

/** Sets the backing, then opens the microphone and starts the count-in from the current
 * position. On failure the vocal is put back. */
export async function armTake(host: EngineHost, project: Project, backing: Backing): Promise<void> {
  const mic = microphone(host);
  await runCommands(host, backingCommands(project, backing));
  try {
    await mic.start();
  } catch (e) {
    await runCommands(host, backingRestoreCommands(project));
    throw e;
  }
}

/** Stops the take, puts the vocal back (before the host snapshots the project into the take's
 * result), and returns the take added to the song and scored. */
export async function finishTake(host: EngineHost, project: Project, settings: ScoreSettings): Promise<TakeAdded> {
  const mic = microphone(host);
  await runCommands(host, backingRestoreCommands(project));
  return mic.finish(settings);
}

/** Abandons the take and puts the vocal back. */
export async function cancelTake(host: EngineHost, project: Project): Promise<void> {
  await microphone(host).cancel();
  await runCommands(host, backingRestoreCommands(project));
}

/** A take was added: it becomes the shown, scored take and the song has unsaved changes. */
export function applyTake(session: Session, added: TakeAdded): void {
  session.project = added.project;
  session.score = { kind: "scored", take: added.take, report: added.report };
  if (session.save.kind === "clean") session.save = { kind: "dirty" };
}

/** Runs `step` once per animation frame (awaiting it) until the returned function is called. */
export function eachFrame(step: () => Promise<void> | void): () => void {
  let alive = true;
  const tick = async () => {
    if (!alive) return;
    await step();
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  return () => {
    alive = false;
  };
}
