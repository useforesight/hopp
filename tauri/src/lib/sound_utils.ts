// src/lib/sound-utils.ts
import { invoke } from "@tauri-apps/api/core";

let soundCommands: Promise<unknown> = Promise.resolve();

// Each command runs on its own thread in Tauri, so a stop and the play right after
// it could otherwise swap places and leave a ringtone silent, or ringing forever.
const queueSoundCommand = (command: "play_sound" | "stop_sound", soundName: string) => {
  soundCommands = soundCommands.then(() =>
    invoke(command, { soundName }).catch((error) => {
      console.error(`Failed to run ${command}:`, error);
    }),
  );
};

class SoundPlayer {
  private readonly soundName: string;

  constructor(soundName: string) {
    this.soundName = soundName;
  }

  play() {
    queueSoundCommand("play_sound", this.soundName);
  }

  stop() {
    queueSoundCommand("stop_sound", this.soundName);
  }
}

export const soundUtils = {
  createPlayer: (soundName: string) => new SoundPlayer(soundName),
};
